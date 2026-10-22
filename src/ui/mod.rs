// SPDX-License-Identifier: GPL-3.0-only

//! The COSMIC application: state, messages and update logic.

#[cfg(debug_assertions)]
mod dev;
mod view;

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::time::Duration;

use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::keyboard::{Key, Modifiers};
use cosmic::iced::{Subscription, event, keyboard};
use cosmic::prelude::*;
use cosmic::widget::{self, about::About, menu, nav_bar};
use futures_util::{Stream, StreamExt, stream};

use keybox::backend::{
    Bus, Client, CollectionInfo, Error, ItemInfo, ObjectId, PromptOutcome, ServiceEvent,
};
use keybox::model::{ConnectionState, Store, display_line};

use crate::config::Config;
use crate::fl;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] = include_bytes!("../../resources/icons/hicolor/scalable/apps/icon.svg");
/// Collapse bursts of change signals (a lock emits one per item).
const REFRESH_DELAY: Duration = Duration::from_millis(150);
/// When to suggest that no unlock prompt may appear.
/// Below this window width (logical pixels) the entry list and the details
/// are shown one at a time.
const TWO_PANE_MIN_WIDTH: f32 = 960.0;
const UNLOCK_HINT_DELAY: Duration = Duration::from_secs(10);

/// Launch flags. Keybox takes no arguments; single-instance mode needs the type.
#[derive(Debug, Clone, Default)]
pub struct Flags;

impl cosmic::app::CosmicFlags for Flags {
    type SubCommand = String;
    type Args = Vec<String>;
}

pub struct App {
    core: cosmic::Core,
    about: About,
    config: Config,
    nav: nav_bar::Model,
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    toasts: widget::Toasts<Message>,

    connection: ConnectionState,
    client: Option<Client>,
    store: Option<Store>,
    /// Bumped on every connect attempt; late results of older attempts are dropped.
    connect_attempt: u64,

    selected_item: Option<ObjectId>,
    /// `Some` while the search field is open.
    search: Option<String>,
    search_id: widget::Id,
    show_technical: bool,
    window_width: f32,

    pending_refresh: PendingRefresh,
    unlock: Option<PendingUnlock>,
    /// Collection we asked to lock; checked after the next listing.
    verify_locked: Option<ObjectId>,
    dialog: Option<Dialog>,
    next_op: u64,
    #[cfg(debug_assertions)]
    dev_script: std::collections::VecDeque<dev::Step>,
}

#[derive(Default)]
struct PendingRefresh {
    scheduled: bool,
    collections: bool,
    items: HashSet<ObjectId>,
    single: HashSet<(ObjectId, ObjectId)>,
}

struct PendingUnlock {
    op: u64,
    collection: ObjectId,
    handle: cosmic::iced::task::Handle,
    show_hint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Dialog {
    ConfirmLock(ObjectId),
}

#[derive(Debug, Clone)]
pub enum Message {
    Connect,
    Connected(u64, Result<Client, Error>),
    CollectionsLoaded(u64, bool, Result<Vec<CollectionInfo>, Error>),
    ItemsLoaded(u64, ObjectId, Result<Vec<ItemInfo>, Error>),
    ItemLoaded(u64, ObjectId, ObjectId, Result<ItemInfo, Error>),
    Service(u64, ServiceEvent),
    EventsEnded(u64),
    FlushRefresh(u64),
    Refresh,

    SelectItem(ObjectId),
    Back,
    ToggleTechnical,

    OpenSearch,
    SearchInput(String),
    CloseSearch,

    RequestLock,
    ConfirmLock(ObjectId),
    Unlock,
    CancelUnlock,
    UnlockHint(u64),
    UnlockFinished(u64, Result<PromptOutcome, Error>),
    LockFinished(u64, ObjectId, Result<PromptOutcome, Error>),
    CloseDialog,

    Key(Modifiers, Key),
    ShowAbout,
    LaunchUrl(String),
    CloseToast(widget::ToastId),
    UpdateConfig(Config),
    Quit,
    #[cfg(debug_assertions)]
    DevNext,
    #[cfg(debug_assertions)]
    DevShot(std::path::PathBuf, cosmic::iced::window::Screenshot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    About,
    Refresh,
    Search,
    Lock,
    Unlock,
    Quit,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Message {
        match self {
            Self::About => Message::ShowAbout,
            Self::Refresh => Message::Refresh,
            Self::Search => Message::OpenSearch,
            Self::Lock => Message::RequestLock,
            Self::Unlock => Message::Unlock,
            Self::Quit => Message::Quit,
        }
    }
}

impl cosmic::Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;

    const APP_ID: &'static str = "dev.rabol.Keybox";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, _flags: Flags) -> (Self, Task<cosmic::Action<Message>>) {
        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license(env!("CARGO_PKG_LICENSE"));

        let config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) | Err((_, config)) => config,
            })
            .unwrap_or_default();

        let mut app = Self {
            core,
            about,
            config,
            nav: nav_bar::Model::default(),
            key_binds: key_binds(),
            toasts: widget::Toasts::new(Message::CloseToast),
            connection: ConnectionState::Connecting,
            client: None,
            store: None,
            connect_attempt: 0,
            selected_item: None,
            search: None,
            search_id: widget::Id::unique(),
            show_technical: false,
            window_width: f32::MAX,
            pending_refresh: PendingRefresh::default(),
            unlock: None,
            verify_locked: None,
            dialog: None,
            next_op: 0,
            #[cfg(debug_assertions)]
            dev_script: dev::script().into(),
        };
        let connect = app.connect();
        let title = app.update_title();
        #[cfg(debug_assertions)]
        let title = Task::batch([title, app.dev_schedule(Duration::from_millis(1500))]);
        (app, Task::batch([connect, title]))
    }

    fn header_start(&self) -> Vec<Element<'_, Message>> {
        vec![view::menu_bar(self)]
    }

    fn header_end(&self) -> Vec<Element<'_, Message>> {
        view::header_end(self)
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        (self.connection == ConnectionState::Connected && self.nav.iter().next().is_some())
            .then_some(&self.nav)
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Message>> {
        if !self.core.window.show_context {
            return None;
        }
        Some(context_drawer::about(
            &self.about,
            |url| Message::LaunchUrl(url.to_string()),
            Message::ShowAbout,
        ))
    }

    fn dialog(&self) -> Option<Element<'_, Message>> {
        view::dialog(self)
    }

    fn view(&self) -> Element<'_, Message> {
        widget::toaster(&self.toasts, view::main(self))
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
            event::listen_with(|event, status, _window| match event {
                cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    ..
                }) if status == event::Status::Ignored => Some(Message::Key(modifiers, key)),
                _ => None,
            }),
        ];
        if let Some(client) = &self.client {
            subscriptions.push(Subscription::run_with(
                EventsKey(client.clone()),
                service_events,
            ));
        }
        Subscription::batch(subscriptions)
    }

    fn on_escape(&mut self) -> Task<cosmic::Action<Message>> {
        if self.dialog.take().is_some() {
            return Task::none();
        }
        if self.search.is_some() {
            return self.update(Message::CloseSearch);
        }
        if self.single_pane() && self.selected_item.is_some() {
            self.selected_item = None;
        }
        Task::none()
    }

    fn on_window_resize(&mut self, id: cosmic::iced::window::Id, width: f32, _height: f32) {
        if Some(id) == self.core.main_window_id() {
            self.window_width = width;
        }
    }

    fn on_search(&mut self) -> Task<cosmic::Action<Message>> {
        self.update(Message::OpenSearch)
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<cosmic::Action<Message>> {
        self.nav.activate(id);
        self.selected_item = None;
        self.show_technical = false;
        self.update_title()
    }

    #[allow(clippy::too_many_lines)]
    fn update(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            Message::Connect => return self.connect(),

            Message::Connected(attempt, result) => {
                if attempt != self.connect_attempt {
                    return Task::none();
                }
                match result {
                    Ok(client) => {
                        self.store = Some(Store::new(client.generation()));
                        self.client = Some(client);
                        self.connection = ConnectionState::Connected;
                        return self.load_collections(false);
                    }
                    Err(error) => self.connection = ConnectionState::Failed(error),
                }
            }

            Message::CollectionsLoaded(generation, reload_items, result) => {
                let Some(store) = self.store.as_mut() else {
                    return Task::none();
                };
                store.set_collections(generation, result);
                if store.generation() != generation {
                    return Task::none();
                }
                self.rebuild_nav();
                let mut tasks = vec![self.update_title()];
                if let Some(verify) = self.verify_locked.take()
                    && let Some(c) = self.store.as_ref().and_then(|s| s.collection(&verify))
                    && c.locked == Some(false)
                {
                    let name = collection_name(c);
                    tasks.push(self.toast(fl!("lock-still-unlocked", keyring = name)));
                }
                let ids: Vec<ObjectId> = self
                    .store
                    .as_ref()
                    .map(|s| {
                        s.sorted_collections()
                            .iter()
                            .map(|c| c.id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for id in ids {
                    let loaded = self.store.as_ref().is_some_and(|s| s.items(&id).is_some());
                    if reload_items || !loaded {
                        tasks.push(self.load_items(&id));
                    }
                }
                return Task::batch(tasks);
            }

            Message::ItemsLoaded(generation, collection, result) => {
                if let Some(store) = self.store.as_mut() {
                    store.set_items(generation, &collection, result);
                }
                self.drop_missing_selection();
            }

            Message::ItemLoaded(generation, collection, item, result) => {
                if self.current_generation() != Some(generation) {
                    return Task::none();
                }
                match result {
                    Ok(info) => {
                        if let Some(store) = self.store.as_mut() {
                            store.update_item(generation, info);
                        }
                    }
                    Err(Error::NoSuchObject) => {
                        if let Some(store) = self.store.as_mut() {
                            store.remove_item(&collection, &item);
                        }
                        self.drop_missing_selection();
                    }
                    Err(_) => return self.load_items(&collection),
                }
            }

            Message::Service(generation, event) => {
                if self.current_generation() != Some(generation) {
                    return Task::none();
                }
                return self.on_service_event(generation, event);
            }

            Message::EventsEnded(generation) => {
                if self.current_generation() == Some(generation) {
                    self.disconnect(ConnectionState::Lost);
                }
            }

            Message::FlushRefresh(generation) => {
                let pending = std::mem::take(&mut self.pending_refresh);
                if self.current_generation() != Some(generation) {
                    return Task::none();
                }
                let mut tasks = Vec::new();
                if pending.collections {
                    tasks.push(self.load_collections(false));
                }
                for collection in &pending.items {
                    tasks.push(self.load_items(collection));
                }
                for (collection, item) in pending.single {
                    if !pending.items.contains(&collection) {
                        tasks.push(self.load_item(collection, item));
                    }
                }
                return Task::batch(tasks);
            }

            Message::Refresh => {
                if self.client.is_some() {
                    return self.load_collections(true);
                }
                return self.connect();
            }

            Message::SelectItem(id) => {
                if self.selected_item.as_ref() != Some(&id) {
                    self.show_technical = false;
                }
                self.selected_item = Some(id);
            }

            Message::Back => self.selected_item = None,

            Message::ToggleTechnical => self.show_technical = !self.show_technical,

            Message::OpenSearch => {
                if self.connection != ConnectionState::Connected {
                    return Task::none();
                }
                if self.search.is_none() {
                    self.search = Some(String::new());
                }
                return widget::text_input::focus(self.search_id.clone());
            }

            Message::SearchInput(text) => self.search = Some(text),

            Message::CloseSearch => self.search = None,

            Message::RequestLock => {
                if let Some(id) = self.active_collection_id() {
                    self.dialog = Some(Dialog::ConfirmLock(id));
                }
            }

            Message::ConfirmLock(collection) => {
                self.dialog = None;
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                let generation = client.generation();
                return cosmic::task::future(async move {
                    let result = client.lock(std::slice::from_ref(&collection), None).await;
                    Message::LockFinished(generation, collection, result)
                });
            }

            Message::LockFinished(generation, collection, result) => {
                if self.current_generation() != Some(generation) {
                    return Task::none();
                }
                match result {
                    Ok(_) => {
                        self.verify_locked = Some(collection.clone());
                        return Task::batch([
                            self.load_collections(false),
                            self.load_items(&collection),
                        ]);
                    }
                    Err(error) => return self.operation_failed(&error),
                }
            }

            Message::Unlock => {
                let (Some(client), Some(collection)) =
                    (self.client.clone(), self.active_collection_id())
                else {
                    return Task::none();
                };
                if self.unlock.is_some() {
                    return Task::none();
                }
                self.next_op += 1;
                let op = self.next_op;
                let target = collection.clone();
                // TODO: pass a window identifier once libcosmic can export one on Wayland.
                let (task, handle) = cosmic::task::future(async move {
                    let result = client.unlock(std::slice::from_ref(&target), None).await;
                    Message::UnlockFinished(op, result)
                })
                .abortable();
                self.unlock = Some(PendingUnlock {
                    op,
                    collection,
                    handle,
                    show_hint: false,
                });
                let hint = cosmic::task::future(async move {
                    tokio::time::sleep(UNLOCK_HINT_DELAY).await;
                    Message::UnlockHint(op)
                });
                return Task::batch([task, hint]);
            }

            Message::UnlockHint(op) => {
                if let Some(unlock) = self.unlock.as_mut()
                    && unlock.op == op
                {
                    unlock.show_hint = true;
                }
            }

            Message::CancelUnlock => {
                if let Some(unlock) = self.unlock.take() {
                    // Dropping the request dismisses the provider's prompt.
                    unlock.handle.abort();
                    return self.toast(fl!("unlock-cancelled"));
                }
            }

            Message::UnlockFinished(op, result) => {
                let Some(unlock) = self.unlock.take_if(|u| u.op == op) else {
                    return Task::none();
                };
                match result {
                    Ok(_) => {
                        return Task::batch([
                            self.load_collections(false),
                            self.load_items(&unlock.collection),
                        ]);
                    }
                    Err(Error::PromptDismissed) => return self.toast(fl!("unlock-cancelled")),
                    Err(error) => return self.operation_failed(&error),
                }
            }

            Message::CloseDialog => self.dialog = None,

            Message::Key(modifiers, key) => {
                for (bind, action) in &self.key_binds {
                    if bind.matches(modifiers, &key, None) {
                        return self.update(menu::action::MenuAction::message(action));
                    }
                }
            }

            Message::ShowAbout => {
                self.core.window.show_context = !self.core.window.show_context;
            }

            Message::LaunchUrl(url) => {
                let _ = open::that_detached(&url);
            }

            Message::CloseToast(id) => self.toasts.remove(id),

            Message::UpdateConfig(config) => self.config = config,

            Message::Quit => {
                if let Some(id) = self.core.main_window_id() {
                    return cosmic::iced::window::close(id);
                }
            }

            #[cfg(debug_assertions)]
            Message::DevNext => return self.dev_next(),

            #[cfg(debug_assertions)]
            Message::DevShot(path, shot) => {
                dev::save_png(&path, &shot);
                return self.dev_schedule(Duration::from_millis(100));
            }
        }
        Task::none()
    }
}

impl App {
    fn connect(&mut self) -> Task<cosmic::Action<Message>> {
        self.disconnect(ConnectionState::Connecting);
        self.connect_attempt += 1;
        let attempt = self.connect_attempt;
        let bus = session_bus();
        cosmic::task::future(
            async move { Message::Connected(attempt, Client::connect(&bus).await) },
        )
    }

    /// Drops the client and everything derived from it.
    fn disconnect(&mut self, state: ConnectionState) {
        if let Some(unlock) = self.unlock.take() {
            unlock.handle.abort();
        }
        self.connection = state;
        self.client = None;
        self.store = None;
        self.selected_item = None;
        self.verify_locked = None;
        self.dialog = None;
        self.pending_refresh = PendingRefresh::default();
        self.nav.clear();
    }

    /// Whether to show the entry list and details one at a time.
    fn single_pane(&self) -> bool {
        self.core.is_condensed() || self.window_width < TWO_PANE_MIN_WIDTH
    }

    fn current_generation(&self) -> Option<u64> {
        self.client.as_ref().map(Client::generation)
    }

    fn load_collections(&self, reload_items: bool) -> Task<cosmic::Action<Message>> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        cosmic::task::future(async move {
            let result = client.collections().await;
            Message::CollectionsLoaded(client.generation(), reload_items, result)
        })
    }

    fn load_items(&mut self, collection: &ObjectId) -> Task<cosmic::Action<Message>> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        if let Some(store) = self.store.as_mut() {
            store.begin_items(collection);
        }
        let collection = collection.clone();
        cosmic::task::future(async move {
            let result = client.items(&collection).await;
            Message::ItemsLoaded(client.generation(), collection, result)
        })
    }

    fn load_item(&self, collection: ObjectId, item: ObjectId) -> Task<cosmic::Action<Message>> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        cosmic::task::future(async move {
            let result = client.item(&collection, &item).await;
            Message::ItemLoaded(client.generation(), collection, item, result)
        })
    }

    fn on_service_event(
        &mut self,
        generation: u64,
        event: ServiceEvent,
    ) -> Task<cosmic::Action<Message>> {
        let pending = &mut self.pending_refresh;
        match event {
            ServiceEvent::OwnerChanged => {
                self.disconnect(ConnectionState::Lost);
                return Task::none();
            }
            ServiceEvent::CollectionCreated(_) | ServiceEvent::CollectionDeleted(_) => {
                pending.collections = true;
            }
            ServiceEvent::CollectionChanged(collection) => {
                pending.collections = true;
                pending.items.insert(collection);
            }
            ServiceEvent::ItemCreated { collection, .. } => {
                pending.items.insert(collection);
            }
            ServiceEvent::ItemDeleted { collection, item } => {
                if let Some(store) = self.store.as_mut() {
                    store.remove_item(&collection, &item);
                }
                self.drop_missing_selection();
                self.pending_refresh.items.insert(collection);
            }
            ServiceEvent::ItemChanged { collection, item } => {
                pending.single.insert((collection, item));
            }
            ServiceEvent::PropertiesChanged(object) => {
                let store = self.store.as_ref();
                if store.and_then(|s| s.collection(&object)).is_some() {
                    pending.collections = true;
                    pending.items.insert(object);
                } else if let Some(item) = store.and_then(|s| s.item(&object)) {
                    pending.single.insert((item.collection.clone(), object));
                }
            }
        }
        if self.pending_refresh.scheduled {
            return Task::none();
        }
        self.pending_refresh.scheduled = true;
        cosmic::task::future(async move {
            tokio::time::sleep(REFRESH_DELAY).await;
            Message::FlushRefresh(generation)
        })
    }

    /// Rebuilds the keyring sidebar, keeping the active keyring if it still exists.
    fn rebuild_nav(&mut self) {
        let active = self.active_collection_id();
        self.nav.clear();
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let mut to_activate = None;
        for collection in store.sorted_collections() {
            let icon = match collection.locked {
                Some(true) => "changes-prevent-symbolic",
                Some(false) => "changes-allow-symbolic",
                None => "dialog-warning-symbolic",
            };
            let id = self
                .nav
                .insert()
                .text(collection_name(collection))
                .icon(widget::icon::from_name(icon))
                .data::<ObjectId>(collection.id.clone())
                .id();
            if active.as_ref() == Some(&collection.id)
                || (active.is_none() && to_activate.is_none())
            {
                to_activate = Some(id);
            }
        }
        if let Some(id) = to_activate.or_else(|| self.nav.iter().next()) {
            self.nav.activate(id);
        }
        if active != self.active_collection_id() {
            self.selected_item = None;
        }
    }

    fn active_collection_id(&self) -> Option<ObjectId> {
        self.nav.active_data::<ObjectId>().cloned()
    }

    fn active_collection(&self) -> Option<&CollectionInfo> {
        let id = self.nav.active_data::<ObjectId>()?;
        self.store.as_ref()?.collection(id)
    }

    fn selected_item_info(&self) -> Option<&ItemInfo> {
        self.store.as_ref()?.item(self.selected_item.as_ref()?)
    }

    fn drop_missing_selection(&mut self) {
        if self.selected_item.is_some() && self.selected_item_info().is_none() {
            self.selected_item = None;
        }
    }

    fn toast(&mut self, text: String) -> Task<cosmic::Action<Message>> {
        self.toasts
            .push(widget::Toast::new(text))
            .map(cosmic::Action::App)
    }

    fn operation_failed(&mut self, error: &Error) -> Task<cosmic::Action<Message>> {
        let text = match error {
            Error::Unsupported => fl!("operation-unsupported"),
            other => fl!("operation-failed", reason = error_reason(other)),
        };
        self.toast(text)
    }

    fn update_title(&mut self) -> Task<cosmic::Action<Message>> {
        let mut title = fl!("app-title");
        if let Some(collection) = self.active_collection() {
            title.push_str(" — ");
            title.push_str(&collection_name(collection));
        }
        match self.core.main_window_id() {
            Some(id) => self.set_window_title(title, id),
            None => Task::none(),
        }
    }
}

fn session_bus() -> Bus {
    #[cfg(debug_assertions)]
    if let Some(address) = dev::bus_override() {
        return Bus::Address(address);
    }
    Bus::Session
}

#[cfg(debug_assertions)]
impl App {
    fn dev_schedule(&self, delay: Duration) -> Task<cosmic::Action<Message>> {
        if self.dev_script.is_empty() {
            return Task::none();
        }
        cosmic::task::future(async move {
            tokio::time::sleep(delay).await;
            Message::DevNext
        })
    }

    fn dev_next(&mut self) -> Task<cosmic::Action<Message>> {
        use cosmic::Application;
        use dev::Step;
        let Some(step) = self.dev_script.pop_front() else {
            return Task::none();
        };
        let settle = Duration::from_millis(400);
        let task = match step {
            Step::Wait(delay) => return self.dev_schedule(delay),
            Step::Keyring(label) => {
                let id = self
                    .nav
                    .iter()
                    .find(|id| self.nav.text(*id) == Some(label.as_str()));
                match id {
                    Some(id) => cosmic::Application::on_nav_select(self, id),
                    None => Task::none(),
                }
            }
            Step::Select(label) => {
                let found = self.store.as_ref().and_then(|store| {
                    store
                        .search("")
                        .matches
                        .into_iter()
                        .find(|i| i.label.as_deref() == Some(label.as_str()))
                        .map(|i| (i.collection.clone(), i.id.clone()))
                });
                if let Some((collection, item)) = found {
                    let nav_id = self
                        .nav
                        .iter()
                        .find(|id| self.nav.data::<ObjectId>(*id) == Some(&collection));
                    if let Some(nav_id) = nav_id {
                        self.nav.activate(nav_id);
                    }
                    self.selected_item = Some(item);
                }
                Task::none()
            }
            Step::Search(text) => {
                self.search = Some(text);
                Task::none()
            }
            Step::SearchClose => self.update(Message::CloseSearch),
            Step::Size(width, height) => match self.core.main_window_id() {
                Some(id) => Task::batch([
                    cosmic::iced::window::maximize(id, false),
                    cosmic::iced::window::resize(id, cosmic::iced::Size::new(width, height)),
                ]),
                None => Task::none(),
            },
            Step::Unlock => self.update(Message::Unlock),
            Step::Lock => self.update(Message::RequestLock),
            Step::Confirm => match self.dialog.clone() {
                Some(Dialog::ConfirmLock(id)) => self.update(Message::ConfirmLock(id)),
                None => Task::none(),
            },
            Step::Escape => cosmic::Application::on_escape(self),
            Step::Back => self.update(Message::Back),
            Step::Technical => self.update(Message::ToggleTechnical),
            Step::SinglePane => {
                self.window_width = 0.0;
                Task::none()
            }
            Step::Shot(path) => {
                return match self.core.main_window_id() {
                    Some(id) => cosmic::iced::window::screenshot(id)
                        .map(move |shot| cosmic::Action::App(Message::DevShot(path.clone(), shot))),
                    None => Task::none(),
                };
            }
            Step::Quit => return self.update(Message::Quit),
        };
        Task::batch([task, self.dev_schedule(settle)])
    }
}

/// A keyring's display name: its label made safe, or a fallback.
fn collection_name(collection: &CollectionInfo) -> String {
    display_line(collection.label.as_deref(), 80).unwrap_or_else(|| fl!("keyring-untitled"))
}

/// A short, user-facing reason for an error. Never includes service text.
fn error_reason(error: &Error) -> String {
    match error {
        Error::NoService => fl!("error-no-service"),
        Error::ActivationFailed => fl!("error-activation"),
        Error::PermissionDenied => fl!("error-permission"),
        Error::EncryptionUnavailable => fl!("error-encryption"),
        Error::Locked => fl!("error-locked"),
        Error::NoSuchObject => fl!("error-no-object"),
        Error::Stale => fl!("error-stale"),
        Error::PromptDismissed => fl!("error-dismissed"),
        Error::Disconnected => fl!("error-disconnected"),
        Error::Unsupported => fl!("error-unsupported"),
        Error::Timeout => fl!("error-timeout"),
        Error::SessionLost => fl!("error-session"),
        Error::Protocol => fl!("error-protocol"),
        Error::Service { name } => fl!("error-service", name = name.clone()),
        Error::Crypto(_) => fl!("error-crypto"),
    }
}

fn key_binds() -> HashMap<menu::KeyBind, MenuAction> {
    use menu::key_bind::Modifier;
    let ctrl = |c: &str, action| {
        (
            menu::KeyBind {
                modifiers: vec![Modifier::Ctrl],
                key: Key::Character(c.into()),
            },
            action,
        )
    };
    HashMap::from([
        ctrl("f", MenuAction::Search),
        ctrl("r", MenuAction::Refresh),
        ctrl("q", MenuAction::Quit),
        (
            menu::KeyBind {
                modifiers: vec![],
                key: Key::Named(keyboard::key::Named::F5),
            },
            MenuAction::Refresh,
        ),
    ])
}

/// Subscription identity: one event stream per connection generation.
struct EventsKey(Client);

impl Hash for EventsKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.generation().hash(state);
    }
}

fn service_events(key: &EventsKey) -> impl Stream<Item = Message> + use<> {
    let client = key.0.clone();
    let generation = client.generation();
    stream::once(async move { client.events().await }).flat_map(move |result| match result {
        Ok(events) => events
            .map(move |event| Message::Service(generation, event))
            .chain(stream::once(
                async move { Message::EventsEnded(generation) },
            ))
            .boxed(),
        Err(_) => stream::once(async move { Message::EventsEnded(generation) }).boxed(),
    })
}
