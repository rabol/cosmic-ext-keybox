// SPDX-License-Identifier: GPL-3.0-only

//! The COSMIC application: state, messages and update logic.

#[cfg(debug_assertions)]
mod dev;
mod editor;
mod secrets;
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
    Bus, Client, CollectionInfo, Error, ItemInfo, ObjectId, PromptOutcome, Provider, ServiceEvent,
};
use keybox::model::{ConnectionState, Store, display_line};

use editor::{Editor, TypedSecret};
use secrets::{ClipboardGuard, Intent, SecretSlot, SecretState};

use crate::config::Config;
use crate::fl;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] = include_bytes!("../../resources/icons/hicolor/scalable/apps/icon.svg");
/// Collapse bursts of change signals (a lock emits one per item).
const REFRESH_DELAY: Duration = Duration::from_millis(150);
/// Below this window width (logical pixels) the entry list and the details
/// are shown one at a time.
const TWO_PANE_MIN_WIDTH: f32 = 960.0;
/// When to suggest that no unlock prompt may appear.
const UNLOCK_HINT_DELAY: Duration = Duration::from_secs(10);
/// Choices for how long a revealed secret stays visible, in seconds.
const REVEAL_CHOICES: [u32; 4] = [10, 30, 60, 120];

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
    config_handler: Option<cosmic_config::Config>,
    context_page: ContextPage,
    reveal_options: Vec<String>,
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
    /// The scrollable entry list, for keeping keyboard selection in view.
    entries_scroll_id: widget::Id,
    show_technical: bool,
    window_width: f32,
    window_focused: bool,

    secret: SecretState,
    clipboard: Option<ClipboardGuard>,
    /// Reveal or copy to resume once an unlock completes.
    after_unlock: Option<(ObjectId, Intent)>,

    pending_refresh: PendingRefresh,
    unlock: Option<PendingUnlock>,
    /// Collection we asked to lock; checked after the next listing.
    verify_locked: Option<ObjectId>,
    dialog: Option<Dialog>,
    dialog_input_id: widget::Id,
    /// The entry being created or edited.
    editor: Option<Editor>,
    /// A change to stored data waiting for the service.
    write: Option<PendingWrite>,
    /// Keyring to show once the next listing contains it.
    activate_collection: Option<ObjectId>,
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

/// A change to stored data in progress. Only one runs at a time.
pub struct PendingWrite {
    op: u64,
    handle: cosmic::iced::task::Handle,
    show_hint: bool,
    /// Started by the entry editor.
    editor: bool,
}

/// Results of changes to stored data.
#[derive(Debug, Clone)]
pub enum WriteDone {
    Entry(editor::SaveOutcome),
    EntryDeleted(ObjectId, Result<PromptOutcome, Error>),
    KeyringCreated(Result<ObjectId, Error>),
    KeyringRenamed(Result<(), Error>),
    KeyringDeleted(Result<PromptOutcome, Error>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Dialog {
    ConfirmLock(ObjectId),
    ConfirmCopy(ObjectId),
    DeleteEntry(ObjectId),
    NewKeyring(String),
    RenameKeyring(ObjectId, String),
    DeleteKeyring(ObjectId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPage {
    About,
    Settings,
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

    RevealSecret,
    ToggleReveal,
    CopySecret,
    HideSecret,
    SecretFetched(u64, SecretSlot),
    ConfirmCopy(ObjectId),
    CancelCopy,
    Tick,
    WindowFocus(bool),
    ClipboardChecked(u64, Option<bool>),
    RevealCheck(ObjectId, Result<ItemInfo, Error>),

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
    DialogInput(String),

    NewEntry,
    EditEntry,
    EditorLabel(String),
    EditorSecret(TypedSecret),
    EditorSecretVisible,
    EditorReplaceSecret(bool),
    EditorAttributeKey(usize, String),
    EditorAttributeValue(usize, String),
    EditorAddAttribute,
    EditorRemoveAttribute(usize),
    EditorSave,
    EditorChecked(u64, Result<editor::Check, Error>),
    EditorConfirm,
    EditorReload,
    EditorOpenExisting(ObjectId),
    EditorCancel,
    RequestDeleteEntry,
    ConfirmDeleteEntry(ObjectId),
    RequestNewKeyring,
    ConfirmNewKeyring,
    RequestRenameKeyring,
    ConfirmRenameKeyring(ObjectId),
    RequestDeleteKeyring,
    ConfirmDeleteKeyring(ObjectId),
    WriteFinished(u64, WriteDone),
    WriteHint(u64),
    CancelWrite,

    Key(Modifiers, Key, Option<keyboard::key::Physical>),
    ToggleContextPage(ContextPage),
    SetRevealChoice(usize),
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
    Settings,
    ToggleReveal,
    CopySecret,
    Refresh,
    Search,
    Lock,
    Unlock,
    NewEntry,
    EditEntry,
    DeleteEntry,
    NewKeyring,
    RenameKeyring,
    DeleteKeyring,
    Quit,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Message {
        match self {
            Self::About => Message::ToggleContextPage(ContextPage::About),
            Self::Settings => Message::ToggleContextPage(ContextPage::Settings),
            Self::ToggleReveal => Message::ToggleReveal,
            Self::CopySecret => Message::CopySecret,
            Self::Refresh => Message::Refresh,
            Self::Search => Message::OpenSearch,
            Self::Lock => Message::RequestLock,
            Self::Unlock => Message::Unlock,
            Self::NewEntry => Message::NewEntry,
            Self::EditEntry => Message::EditEntry,
            Self::DeleteEntry => Message::RequestDeleteEntry,
            Self::NewKeyring => Message::RequestNewKeyring,
            Self::RenameKeyring => Message::RequestRenameKeyring,
            Self::DeleteKeyring => Message::RequestDeleteKeyring,
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

        let config_handler = cosmic_config::Config::new(Self::APP_ID, Config::VERSION).ok();
        let config = config_handler
            .as_ref()
            .map(|context| match Config::get_entry(context) {
                Ok(config) | Err((_, config)) => config,
            })
            .unwrap_or_default();
        let reveal_options = REVEAL_CHOICES
            .iter()
            .map(|s| fl!("settings-seconds", seconds = s))
            .collect();

        let mut app = Self {
            core,
            about,
            config,
            config_handler,
            context_page: ContextPage::About,
            reveal_options,
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
            entries_scroll_id: widget::Id::unique(),
            show_technical: false,
            window_width: f32::MAX,
            window_focused: true,
            secret: SecretState::Hidden,
            clipboard: None,
            after_unlock: None,
            pending_refresh: PendingRefresh::default(),
            unlock: None,
            verify_locked: None,
            dialog: None,
            dialog_input_id: widget::Id::unique(),
            editor: None,
            write: None,
            activate_collection: None,
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
        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::LaunchUrl(url.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
            ContextPage::Settings => context_drawer::context_drawer(
                view::settings(self),
                Message::ToggleContextPage(ContextPage::Settings),
            )
            .title(fl!("settings")),
        })
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
                    physical_key,
                    ..
                }) if status == event::Status::Ignored => {
                    Some(Message::Key(modifiers, key, Some(physical_key)))
                }
                cosmic::iced::Event::Window(cosmic::iced::window::Event::Focused) => {
                    Some(Message::WindowFocus(true))
                }
                cosmic::iced::Event::Window(cosmic::iced::window::Event::Unfocused) => {
                    Some(Message::WindowFocus(false))
                }
                _ => None,
            }),
        ];
        if self.needs_tick() {
            subscriptions
                .push(cosmic::iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick));
        }
        if let Some(client) = &self.client {
            subscriptions.push(Subscription::run_with(
                EventsKey(client.clone()),
                service_events,
            ));
        }
        Subscription::batch(subscriptions)
    }

    fn on_escape(&mut self) -> Task<cosmic::Action<Message>> {
        if self.dialog.is_some() {
            self.cancel_copy();
            self.dialog = None;
            return Task::none();
        }
        if self.search.is_some() {
            return self.update(Message::CloseSearch);
        }
        if let Some(editor) = &self.editor {
            // Never throw away typed changes on a stray key press.
            if !editor.is_dirty() && self.write.is_none() {
                self.editor = None;
            }
            return Task::none();
        }
        if self.single_pane() && self.selected_item.is_some() {
            self.selected_item = None;
            self.hide_secret();
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
        self.hide_secret();
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
                self.hide_secret_unless(Some(&id));
                self.selected_item = Some(id);
            }

            Message::Back => {
                self.selected_item = None;
                self.hide_secret();
            }

            Message::RevealSecret => return self.request_secret(Intent::Reveal),
            Message::ToggleReveal => {
                if self.selected_item.is_none() {
                    return Task::none();
                }
                if matches!(self.secret, SecretState::Shown { .. }) {
                    self.hide_secret();
                } else {
                    return self.request_secret(Intent::Reveal);
                }
            }
            Message::CopySecret => return self.request_secret(Intent::Copy),
            Message::HideSecret => self.hide_secret(),
            Message::SecretFetched(op, slot) => return self.on_secret_fetched(op, &slot),
            Message::ConfirmCopy(item) => return self.confirm_copy(&item),
            Message::CancelCopy => self.cancel_copy(),
            Message::Tick => return self.tick(),
            Message::WindowFocus(focused) => return self.on_focus_changed(focused),
            Message::ClipboardChecked(op, ours) => return self.on_clipboard_checked(op, ours),
            Message::RevealCheck(item, result) => self.on_reveal_check(&item, result),

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
                if let Some(collection) = self.active_collection_id() {
                    return self.start_unlock(collection);
                }
            }

            Message::UnlockHint(op) => {
                if let Some(unlock) = self.unlock.as_mut()
                    && unlock.op == op
                {
                    unlock.show_hint = true;
                }
            }

            Message::CancelUnlock => {
                self.after_unlock = None;
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
                let resume = self.after_unlock.take();
                match result {
                    Ok(_) => {
                        let mut tasks = vec![
                            self.load_collections(false),
                            self.load_items(&unlock.collection),
                        ];
                        if let Some((item, intent)) = resume
                            && self.selected_item.as_ref() == Some(&item)
                        {
                            tasks.push(self.request_secret(intent));
                        }
                        return Task::batch(tasks);
                    }
                    Err(Error::PromptDismissed) => return self.toast(fl!("unlock-cancelled")),
                    Err(error) => return self.operation_failed(&error),
                }
            }

            Message::CloseDialog => self.dialog = None,

            Message::DialogInput(text) => {
                if let Some(Dialog::NewKeyring(name) | Dialog::RenameKeyring(_, name)) =
                    &mut self.dialog
                {
                    *name = text;
                }
            }

            Message::NewEntry => return self.open_new_entry(),
            Message::EditEntry => return self.open_edit_entry(),
            Message::EditorCancel => self.close_editor(),
            Message::EditorSave => return self.editor_save(),
            Message::EditorChecked(op, result) => return self.editor_checked(op, result),
            Message::EditorConfirm => return self.editor_confirm(),
            Message::EditorReload => return self.editor_reload(),
            Message::EditorOpenExisting(item) => return self.editor_open_existing(item),
            Message::EditorLabel(_)
            | Message::EditorSecret(_)
            | Message::EditorSecretVisible
            | Message::EditorReplaceSecret(_)
            | Message::EditorAttributeKey(..)
            | Message::EditorAttributeValue(..)
            | Message::EditorAddAttribute
            | Message::EditorRemoveAttribute(_) => self.edit_field(message),

            Message::RequestDeleteEntry => {
                if self.editor.is_none()
                    && self.write.is_none()
                    && let Some(info) = self.selected_item_info()
                {
                    if info.locked == Some(false) {
                        self.dialog = Some(Dialog::DeleteEntry(info.id.clone()));
                    } else {
                        return self.toast(fl!("editor-unlock-first"));
                    }
                }
            }

            Message::ConfirmDeleteEntry(item) => {
                self.dialog = None;
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                return self.start_write(false, async move {
                    let result = client.delete_item(&item, None).await;
                    WriteDone::EntryDeleted(item, result)
                });
            }

            Message::RequestNewKeyring => {
                if self.connection == ConnectionState::Connected && self.write.is_none() {
                    self.dialog = Some(Dialog::NewKeyring(String::new()));
                    return widget::text_input::focus(self.dialog_input_id.clone());
                }
            }

            Message::ConfirmNewKeyring => {
                let Some(Dialog::NewKeyring(name)) = self.dialog.take() else {
                    return Task::none();
                };
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                if name.trim().is_empty() {
                    self.dialog = Some(Dialog::NewKeyring(name));
                    return Task::none();
                }
                return self.start_write(false, async move {
                    WriteDone::KeyringCreated(client.create_collection(&name, None).await)
                });
            }

            Message::RequestRenameKeyring => {
                if self.write.is_none()
                    && let Some(collection) = self.active_collection()
                {
                    if collection.locked != Some(false) {
                        return self.toast(fl!("keyring-unlock-first"));
                    }
                    self.dialog = Some(Dialog::RenameKeyring(
                        collection.id.clone(),
                        collection.label.clone().unwrap_or_default(),
                    ));
                    return widget::text_input::focus(self.dialog_input_id.clone());
                }
            }

            Message::ConfirmRenameKeyring(collection) => {
                let Some(Dialog::RenameKeyring(_, name)) = self.dialog.take() else {
                    return Task::none();
                };
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                if name.trim().is_empty() {
                    self.dialog = Some(Dialog::RenameKeyring(collection, name));
                    return Task::none();
                }
                return self.start_write(false, async move {
                    WriteDone::KeyringRenamed(client.set_collection_label(&collection, &name).await)
                });
            }

            Message::RequestDeleteKeyring => {
                if self.write.is_none()
                    && let Some(collection) = self.active_collection()
                {
                    match self.keyring_deletable(collection) {
                        Ok(()) => self.dialog = Some(Dialog::DeleteKeyring(collection.id.clone())),
                        Err(reason) => return self.toast(reason),
                    }
                }
            }

            Message::ConfirmDeleteKeyring(collection) => {
                self.dialog = None;
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                return self.start_write(false, async move {
                    let result = client.delete_empty_collection(&collection, None).await;
                    WriteDone::KeyringDeleted(result)
                });
            }

            Message::WriteFinished(op, done) => {
                if self.write.as_ref().is_none_or(|w| w.op != op) {
                    return Task::none();
                }
                self.write = None;
                return self.on_write_finished(done);
            }

            Message::WriteHint(op) => {
                if let Some(write) = self.write.as_mut()
                    && write.op == op
                {
                    write.show_hint = true;
                }
            }

            Message::CancelWrite => {
                if self.write.is_some() {
                    self.cancel_write();
                    return self.toast(fl!("write-cancelled"));
                }
            }

            Message::Key(modifiers, key, physical) => {
                if let Some(task) = self.navigation_key(modifiers, &key) {
                    return task;
                }
                for (bind, action) in &self.key_binds {
                    if bind.matches(modifiers, &key, physical.as_ref()) {
                        return self.update(menu::action::MenuAction::message(action));
                    }
                }
            }

            Message::ToggleContextPage(page) => {
                if self.context_page == page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = page;
                    self.core.window.show_context = true;
                }
            }

            Message::SetRevealChoice(index) => {
                if let (Some(&seconds), Some(handler)) =
                    (REVEAL_CHOICES.get(index), self.config_handler.as_ref())
                {
                    let _ = self.config.set_reveal_seconds(handler, seconds);
                }
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

    fn start_unlock(&mut self, collection: ObjectId) -> Task<cosmic::Action<Message>> {
        let Some(client) = self.client.clone() else {
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
        Task::batch([task, hint])
    }

    /// Starts a change to stored data. The provider may show a prompt;
    /// until it answers, a banner offers Cancel, which drops the request
    /// (dismissing the prompt).
    fn start_write(
        &mut self,
        editor: bool,
        work: impl std::future::Future<Output = WriteDone> + Send + 'static,
    ) -> Task<cosmic::Action<Message>> {
        if self.write.is_some() {
            return Task::none();
        }
        self.next_op += 1;
        let op = self.next_op;
        let (task, handle) =
            cosmic::task::future(async move { Message::WriteFinished(op, work.await) }).abortable();
        self.write = Some(PendingWrite {
            op,
            handle,
            show_hint: false,
            editor,
        });
        let hint = cosmic::task::future(async move {
            tokio::time::sleep(editor::WRITE_HINT_DELAY).await;
            Message::WriteHint(op)
        });
        Task::batch([task, hint])
    }

    /// Stops waiting for a change. The service may still have made it.
    fn cancel_write(&mut self) {
        if let Some(write) = self.write.take() {
            write.handle.abort();
            if write.editor
                && let Some(editor) = self.editor.as_mut()
            {
                editor.stage = editor::Stage::Editing;
                editor.notice = Some(editor::Notice::Failed(fl!("write-cancelled")));
            }
        }
    }

    fn on_write_finished(&mut self, done: WriteDone) -> Task<cosmic::Action<Message>> {
        match done {
            WriteDone::Entry(outcome) => self.editor_saved(outcome),
            WriteDone::EntryDeleted(item, result) => match result {
                Ok(_) => {
                    let collection = self
                        .store
                        .as_ref()
                        .and_then(|s| s.item(&item))
                        .map(|i| i.collection.clone());
                    let mut tasks = vec![self.toast(fl!("entry-deleted"))];
                    if let Some(collection) = collection {
                        if let Some(store) = self.store.as_mut() {
                            store.remove_item(&collection, &item);
                        }
                        tasks.push(self.load_items(&collection));
                    }
                    self.drop_missing_selection();
                    Task::batch(tasks)
                }
                Err(Error::PromptDismissed) => self.toast(fl!("write-dismissed")),
                Err(error) => self.operation_failed(&error),
            },
            WriteDone::KeyringCreated(result) => match result {
                Ok(id) => {
                    self.activate_collection = Some(id);
                    let toast = self.toast(fl!("keyring-created"));
                    Task::batch([self.load_collections(false), toast])
                }
                Err(Error::PromptDismissed) => self.toast(fl!("write-dismissed")),
                Err(error) => self.operation_failed(&error),
            },
            WriteDone::KeyringRenamed(result) => match result {
                Ok(()) => Task::batch([self.load_collections(false), self.update_title()]),
                Err(error) => self.operation_failed(&error),
            },
            WriteDone::KeyringDeleted(result) => match result {
                Ok(_) => {
                    let toast = self.toast(fl!("keyring-deleted"));
                    Task::batch([self.load_collections(false), toast])
                }
                Err(Error::PromptDismissed) => self.toast(fl!("write-dismissed")),
                Err(error) => self.operation_failed(&error),
            },
        }
    }

    /// Applies a change to one of the editor's fields.
    fn edit_field(&mut self, message: Message) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.stage != editor::Stage::Editing {
            return;
        }
        match message {
            Message::EditorLabel(text) => editor.label = text,
            Message::EditorSecret(text) => {
                // Shares the message's wiping buffer; the old value is wiped.
                editor.secret = zeroize::Zeroizing::new(text.take());
            }
            Message::EditorSecretVisible => editor.secret_hidden = !editor.secret_hidden,
            Message::EditorReplaceSecret(on) => {
                editor.replace_secret = on;
                if !on {
                    editor.secret = zeroize::Zeroizing::new(String::new());
                    editor.secret_hidden = true;
                }
            }
            Message::EditorAttributeKey(index, text) => {
                if let Some(row) = editor.attributes.get_mut(index) {
                    row.key = text;
                }
            }
            Message::EditorAttributeValue(index, text) => {
                if let Some(row) = editor.attributes.get_mut(index) {
                    row.value = text;
                }
            }
            Message::EditorAddAttribute => editor.attributes.push(editor::AttributeRow {
                key: String::new(),
                value: String::new(),
            }),
            Message::EditorRemoveAttribute(index) => {
                if index < editor.attributes.len() {
                    editor.attributes.remove(index);
                }
            }
            _ => return,
        }
        editor.touched();
    }

    /// Whether the keyring may be deleted from Keybox, or why not.
    fn keyring_deletable(&self, collection: &CollectionInfo) -> Result<(), String> {
        if collection.is_session {
            return Err(fl!("keyring-delete-session"));
        }
        if collection.is_default != Some(false) {
            return Err(fl!("keyring-delete-default"));
        }
        if collection.locked != Some(false) {
            return Err(fl!("keyring-unlock-first"));
        }
        match self.store.as_ref().and_then(|s| s.items(&collection.id)) {
            Some(keybox::model::Load::Loaded(items)) if items.is_empty() => Ok(()),
            Some(keybox::model::Load::Loaded(_)) => Err(fl!("keyring-delete-not-empty")),
            _ => Err(fl!("keyring-delete-not-loaded")),
        }
    }

    /// Whether changes to stored data were never tested with this provider.
    fn writes_untested(&self) -> bool {
        !matches!(
            self.client.as_ref().and_then(Client::provider),
            Some(Provider::Oo7 | Provider::GnomeKeyring)
        )
    }

    /// Drops the client and everything derived from it.
    fn disconnect(&mut self, state: ConnectionState) {
        self.hide_secret();
        if let Some(unlock) = self.unlock.take() {
            unlock.handle.abort();
        }
        if let Some(write) = self.write.take() {
            write.handle.abort();
        }
        // Typed changes cannot be saved to a different connection.
        self.editor = None;
        self.activate_collection = None;
        self.connection = state;
        self.client = None;
        self.store = None;
        self.selected_item = None;
        self.verify_locked = None;
        self.dialog = None;
        self.pending_refresh = PendingRefresh::default();
        self.nav.clear();
    }

    /// Arrow keys move through the shown entries; Ctrl+Page Up/Down switch
    /// keyrings. Only called for keys no widget handled.
    fn navigation_key(
        &mut self,
        modifiers: Modifiers,
        key: &Key,
    ) -> Option<Task<cosmic::Action<Message>>> {
        use keyboard::key::Named;
        if self.dialog.is_some()
            || self.editor.is_some()
            || self.connection != ConnectionState::Connected
        {
            return None;
        }
        let Key::Named(named) = key else {
            return None;
        };
        if modifiers.control() && !modifiers.shift() && !modifiers.alt() {
            let step: isize = match named {
                Named::PageDown => 1,
                Named::PageUp => -1,
                _ => return None,
            };
            let ids: Vec<_> = self.nav.iter().collect();
            let current = ids.iter().position(|id| *id == self.nav.active())?;
            let next = current
                .checked_add_signed(step)
                .filter(|n| *n < ids.len())?;
            return Some(cosmic::Application::on_nav_select(self, ids[next]));
        }
        if !modifiers.is_empty() {
            return None;
        }
        let entries = self.visible_entry_ids();
        if entries.is_empty() {
            return None;
        }
        let current = self
            .selected_item
            .as_ref()
            .and_then(|s| entries.iter().position(|e| e == s));
        let last = entries.len() - 1;
        let next = match (named, current) {
            (Named::ArrowDown, None) | (Named::Home, _) => 0,
            (Named::ArrowUp, None) | (Named::End, _) => last,
            (Named::ArrowDown, Some(i)) => (i + 1).min(last),
            (Named::ArrowUp, Some(i)) => i.saturating_sub(1),
            _ => return None,
        };
        let id = entries[next].clone();
        let select = cosmic::Application::update(self, Message::SelectItem(id));
        // Approximate: rows differ in height, but this keeps the selection near view.
        #[allow(clippy::cast_precision_loss)]
        let y = if last == 0 {
            0.0
        } else {
            next as f32 / last as f32
        };
        let scroll = cosmic::iced::widget::scrollable::snap_to(
            self.entries_scroll_id.clone(),
            cosmic::iced::widget::scrollable::RelativeOffset {
                x: None,
                y: Some(y),
            },
        );
        Some(Task::batch([select, scroll]))
    }

    /// Entries in the order the list currently shows them.
    fn visible_entry_ids(&self) -> Vec<ObjectId> {
        let Some(store) = self.store.as_ref() else {
            return Vec::new();
        };
        match self.search.as_deref().filter(|q| !q.trim().is_empty()) {
            Some(query) => store
                .search(query)
                .matches
                .iter()
                .map(|i| i.id.clone())
                .collect(),
            None => self
                .active_collection_id()
                .and_then(|c| store.visible_items(&c))
                .map(|v| v.items.iter().map(|i| i.id.clone()).collect())
                .unwrap_or_default(),
        }
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
        let check = self.check_secret_on_event(&event);
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
                self.editor_on_item_event(&item, true);
                if let Some(store) = self.store.as_mut() {
                    store.remove_item(&collection, &item);
                }
                self.drop_missing_selection();
                self.pending_refresh.items.insert(collection);
            }
            ServiceEvent::ItemChanged { collection, item } => {
                self.editor_on_item_event(&item, false);
                self.pending_refresh.single.insert((collection, item));
            }
            ServiceEvent::PropertiesChanged(object) => {
                let store = self.store.as_ref();
                if store.and_then(|s| s.collection(&object)).is_some() {
                    pending.collections = true;
                    pending.items.insert(object);
                } else if let Some(item) = store.and_then(|s| s.item(&object)) {
                    let collection = item.collection.clone();
                    self.editor_on_item_event(&object, false);
                    self.pending_refresh.single.insert((collection, object));
                }
            }
        }
        if self.pending_refresh.scheduled {
            return check;
        }
        self.pending_refresh.scheduled = true;
        let flush = cosmic::task::future(async move {
            tokio::time::sleep(REFRESH_DELAY).await;
            Message::FlushRefresh(generation)
        });
        Task::batch([check, flush])
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
            if self.activate_collection.as_ref() == Some(&collection.id) {
                self.activate_collection = None;
                to_activate = Some(id);
            } else if active.as_ref() == Some(&collection.id)
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
            self.hide_secret();
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

    #[allow(clippy::too_many_lines)]
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
                Some(Dialog::ConfirmCopy(id)) => self.update(Message::ConfirmCopy(id)),
                Some(Dialog::DeleteEntry(id)) => self.update(Message::ConfirmDeleteEntry(id)),
                Some(Dialog::NewKeyring(_)) => self.update(Message::ConfirmNewKeyring),
                Some(Dialog::RenameKeyring(id, _)) => {
                    self.update(Message::ConfirmRenameKeyring(id))
                }
                Some(Dialog::DeleteKeyring(id)) => self.update(Message::ConfirmDeleteKeyring(id)),
                None => Task::none(),
            },
            Step::Message(message) => self.update(*message),
            Step::Status => {
                let editor = self.editor.as_ref().map(|e| {
                    let notice = e.notice.as_ref().map(|n| match n {
                        editor::Notice::Duplicates { count, .. } => format!("Duplicates({count})"),
                        editor::Notice::Conflict => "Conflict".into(),
                        editor::Notice::Deleted => "Deleted".into(),
                        editor::Notice::Locked => "Locked".into(),
                        editor::Notice::Invalid(_) => "Invalid".into(),
                        editor::Notice::Failed(_) => "Failed".into(),
                    });
                    format!(
                        "stage={:?} notice={notice:?} changed_elsewhere={}",
                        e.stage, e.changed_elsewhere
                    )
                });
                let dialog = self.dialog.as_ref().map(|d| match d {
                    Dialog::ConfirmLock(_) => "ConfirmLock",
                    Dialog::ConfirmCopy(_) => "ConfirmCopy",
                    Dialog::DeleteEntry(_) => "DeleteEntry",
                    Dialog::NewKeyring(_) => "NewKeyring",
                    Dialog::RenameKeyring(..) => "RenameKeyring",
                    Dialog::DeleteKeyring(_) => "DeleteKeyring",
                });
                eprintln!(
                    "KEYBOX_DEV_SCRIPT: status editor={editor:?} dialog={dialog:?} write={} selected={}",
                    self.write.is_some(),
                    self.selected_item.is_some(),
                );
                Task::none()
            }
            Step::Field(field, text) => self.update(match field {
                dev::Field::Label => Message::EditorLabel(text),
                dev::Field::Secret => Message::EditorSecret(TypedSecret::new(text)),
                dev::Field::AttributeKey(i) => Message::EditorAttributeKey(i, text),
                dev::Field::AttributeValue(i) => Message::EditorAttributeValue(i, text),
                dev::Field::Dialog => Message::DialogInput(text),
            }),
            Step::Reveal => self.update(Message::RevealSecret),
            Step::Copy => self.update(Message::CopySecret),
            Step::Hide => self.update(Message::HideSecret),
            Step::Focus(focused) => self.update(Message::WindowFocus(focused)),
            Step::Key(spec) => match dev::parse_key(&spec) {
                Some((modifiers, key)) => self.update(Message::Key(modifiers, key, None)),
                None => Task::none(),
            },
            Step::Settings => self.update(Message::ToggleContextPage(ContextPage::Settings)),
            Step::RevealChoice(index) => self.update(Message::SetRevealChoice(index)),
            Step::ClipSet(text) => cosmic::iced::clipboard::write(text),
            Step::ClipIs(expected) => cosmic::iced::clipboard::read().map(move |content| {
                eprintln!(
                    "KEYBOX_DEV_SCRIPT: clipboard equals {expected:?}: {}",
                    content.as_deref() == Some(expected.as_str())
                );
                cosmic::Action::None
            }),
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
        Error::NotEmpty => fl!("error-not-empty"),
        Error::NotApplied => fl!("error-not-applied"),
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
    let ctrl_shift = |c: &str, action| {
        (
            menu::KeyBind {
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
                key: Key::Character(c.into()),
            },
            action,
        )
    };
    HashMap::from([
        ctrl_shift("e", MenuAction::ToggleReveal),
        ctrl_shift("c", MenuAction::CopySecret),
        ctrl(",", MenuAction::Settings),
        ctrl("f", MenuAction::Search),
        ctrl("r", MenuAction::Refresh),
        ctrl("q", MenuAction::Quit),
        ctrl("n", MenuAction::NewEntry),
        (
            menu::KeyBind {
                modifiers: vec![],
                key: Key::Named(keyboard::key::Named::F2),
            },
            MenuAction::EditEntry,
        ),
        (
            menu::KeyBind {
                modifiers: vec![],
                key: Key::Named(keyboard::key::Named::Delete),
            },
            MenuAction::DeleteEntry,
        ),
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
