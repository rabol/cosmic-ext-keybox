// SPDX-License-Identifier: GPL-3.0-only

//! Views. All service-provided text (labels, attributes) is shown as plain
//! text: no markup, links or other interpretation.

use std::time::SystemTime;

use cosmic::iced::widget::text::Wrapping;
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget::{self, menu};

use keybox::backend::{CollectionInfo, Error, ItemInfo};
use keybox::model::{ConnectionState, Load, display_line};

use super::editor::{Editor, Notice, Stage};
use super::secrets::SecretState;
use super::{App, Dialog, MenuAction, Message, REVEAL_CHOICES, collection_name, error_reason};
use crate::fl;
use keybox::secret::SecretValue;

/// Longest label shown in lists.
const LIST_LABEL_CHARS: usize = 120;
/// Longest label or attribute value shown in the details pane.
const DETAIL_TEXT_CHARS: usize = 2000;

#[allow(clippy::too_many_lines)]
pub fn menu_bar(app: &App) -> Element<'_, Message> {
    let connected = app.connection == ConnectionState::Connected;
    let locked = app.active_collection().and_then(|c| c.locked);
    let idle = connected && app.editor.is_none() && app.write.is_none();
    let entry_unlocked = app
        .selected_item_info()
        .is_some_and(|i| i.locked == Some(false));
    let deletable = app
        .active_collection()
        .is_some_and(|c| app.keyring_deletable(c).is_ok());
    let item = |label: String, action, enabled: bool| {
        if enabled {
            menu::Item::Button(label, None, action)
        } else {
            menu::Item::ButtonDisabled(label, None, action)
        }
    };
    menu::bar(vec![
        menu::Tree::with_children(
            menu::root(fl!("menu-file")).apply(Element::from),
            menu::items(
                &app.key_binds,
                vec![
                    item(fl!("refresh"), MenuAction::Refresh, true),
                    item(fl!("search"), MenuAction::Search, connected),
                    menu::Item::Divider,
                    item(fl!("quit"), MenuAction::Quit, true),
                ],
            ),
        ),
        menu::Tree::with_children(
            menu::root(fl!("menu-keyring")).apply(Element::from),
            menu::items(
                &app.key_binds,
                vec![
                    item(
                        fl!("unlock-keyring"),
                        MenuAction::Unlock,
                        connected && locked == Some(true) && app.unlock.is_none(),
                    ),
                    item(
                        fl!("lock-keyring"),
                        MenuAction::Lock,
                        connected && locked == Some(false),
                    ),
                    menu::Item::Divider,
                    item(fl!("menu-new-keyring"), MenuAction::NewKeyring, idle),
                    item(
                        fl!("menu-rename-keyring"),
                        MenuAction::RenameKeyring,
                        idle && locked == Some(false),
                    ),
                    item(
                        fl!("menu-delete-keyring"),
                        MenuAction::DeleteKeyring,
                        idle && deletable,
                    ),
                ],
            ),
        ),
        menu::Tree::with_children(
            menu::root(fl!("menu-entry")).apply(Element::from),
            menu::items(
                &app.key_binds,
                vec![
                    item(
                        fl!("menu-reveal-hide"),
                        MenuAction::ToggleReveal,
                        connected && app.selected_item.is_some(),
                    ),
                    item(
                        fl!("menu-copy-secret"),
                        MenuAction::CopySecret,
                        connected && app.selected_item.is_some(),
                    ),
                    menu::Item::Divider,
                    item(
                        fl!("menu-new-entry"),
                        MenuAction::NewEntry,
                        idle && locked == Some(false),
                    ),
                    item(
                        fl!("menu-edit-entry"),
                        MenuAction::EditEntry,
                        idle && entry_unlocked,
                    ),
                    item(
                        fl!("menu-delete-entry"),
                        MenuAction::DeleteEntry,
                        idle && entry_unlocked,
                    ),
                ],
            ),
        ),
        menu::Tree::with_children(
            menu::root(fl!("menu-view")).apply(Element::from),
            menu::items(
                &app.key_binds,
                vec![
                    item(fl!("settings-menu"), MenuAction::Settings, true),
                    item(fl!("about"), MenuAction::About, true),
                ],
            ),
        ),
    ])
    .into()
}

pub fn header_end(app: &App) -> Vec<Element<'_, Message>> {
    let mut elements = Vec::new();

    let status = match &app.connection {
        ConnectionState::Connecting => fl!("status-connecting"),
        ConnectionState::Connected => {
            let provider = app
                .client
                .as_ref()
                .and_then(keybox::backend::Client::provider)
                .map_or_else(|| fl!("secret-service"), |p| p.display_name().to_owned());
            fl!("status-connected", provider = provider)
        }
        ConnectionState::Failed(_) | ConnectionState::Lost => fl!("status-disconnected"),
    };
    elements.push(
        widget::text::caption(status)
            .apply(widget::container)
            .padding([0, 8])
            .into(),
    );

    if app.connection == ConnectionState::Connected {
        match &app.search {
            Some(query) => elements.push(
                widget::search_input(fl!("search-placeholder"), query)
                    .id(app.search_id.clone())
                    .on_input(Message::SearchInput)
                    .on_clear(Message::CloseSearch)
                    .width(Length::Fixed(240.0))
                    .into(),
            ),
            None => elements.push(
                widget::tooltip(
                    widget::button::icon(widget::icon::from_name("system-search-symbolic"))
                        .on_press(Message::OpenSearch),
                    widget::text(fl!("search")),
                    widget::tooltip::Position::Bottom,
                )
                .into(),
            ),
        }
    }

    elements.push(
        widget::tooltip(
            widget::button::icon(widget::icon::from_name("view-refresh-symbolic"))
                .on_press(Message::Refresh),
            widget::text(fl!("refresh")),
            widget::tooltip::Position::Bottom,
        )
        .into(),
    );
    elements
}

#[allow(clippy::too_many_lines)]
pub fn dialog(app: &App) -> Option<Element<'_, Message>> {
    match app.dialog.as_ref()? {
        Dialog::ConfirmLock(id) => {
            let name = app
                .store
                .as_ref()
                .and_then(|s| s.collection(id))
                .map_or_else(|| fl!("keyring-untitled"), collection_name);
            Some(
                widget::dialog()
                    .title(fl!("lock-dialog-title", keyring = name))
                    .body(if cfg!(feature = "builtin-prompter") {
                        fl!("lock-dialog-body-builtin")
                    } else {
                        fl!("lock-dialog-body")
                    })
                    .icon(widget::icon::from_name("changes-prevent-symbolic").size(64))
                    .primary_action(
                        widget::button::destructive(fl!("lock-dialog-confirm"))
                            .on_press(Message::ConfirmLock(id.clone())),
                    )
                    .secondary_action(
                        widget::button::standard(fl!("cancel")).on_press(Message::CloseDialog),
                    )
                    .into(),
            )
        }
        Dialog::ConfirmCopy(id) => Some(
            widget::dialog()
                .title(fl!("copy-control-title"))
                .body(fl!("copy-control-body"))
                .icon(widget::icon::from_name("edit-copy-symbolic").size(64))
                .primary_action(
                    widget::button::suggested(fl!("copy-control-confirm"))
                        .on_press(Message::ConfirmCopy(id.clone())),
                )
                .secondary_action(
                    widget::button::standard(fl!("cancel")).on_press(Message::CancelCopy),
                )
                .into(),
        ),
        Dialog::DeleteEntry(id) => {
            let store = app.store.as_ref()?;
            let item = store.item(id)?;
            let keyring = store
                .collection(&item.collection)
                .map_or_else(|| fl!("keyring-untitled"), collection_name);
            let mut body = fl!("delete-entry-body", keyring = keyring);
            if app.writes_untested() {
                body = format!("{body}\n\n{}", fl!("writes-untested"));
            }
            Some(
                widget::dialog()
                    .title(fl!("delete-entry-title", entry = entry_label(item, 80)))
                    .body(body)
                    .icon(widget::icon::from_name("edit-delete-symbolic").size(64))
                    .primary_action(
                        widget::button::destructive(fl!("delete-entry-confirm"))
                            .on_press(Message::ConfirmDeleteEntry(id.clone())),
                    )
                    .secondary_action(
                        widget::button::standard(fl!("cancel")).on_press(Message::CloseDialog),
                    )
                    .into(),
            )
        }
        Dialog::NewKeyring(name) => {
            let valid = !name.trim().is_empty();
            let mut body = fl!("new-keyring-body");
            if app.writes_untested() {
                body = format!("{body}\n\n{}", fl!("writes-untested"));
            }
            Some(
                widget::dialog()
                    .title(fl!("new-keyring-title"))
                    .body(body)
                    .control(
                        widget::text_input(fl!("keyring-name"), name.as_str())
                            .label(fl!("keyring-name"))
                            .id(app.dialog_input_id.clone())
                            .on_input(Message::DialogInput)
                            .on_submit_maybe(valid.then_some(|_| Message::ConfirmNewKeyring)),
                    )
                    .primary_action(
                        widget::button::suggested(fl!("new-keyring-confirm"))
                            .on_press_maybe(valid.then_some(Message::ConfirmNewKeyring)),
                    )
                    .secondary_action(
                        widget::button::standard(fl!("cancel")).on_press(Message::CloseDialog),
                    )
                    .into(),
            )
        }
        Dialog::RenameKeyring(id, name) => {
            let valid = !name.trim().is_empty();
            let confirm = Message::ConfirmRenameKeyring(id.clone());
            let submit = confirm.clone();
            Some(
                widget::dialog()
                    .title(fl!("rename-keyring-title"))
                    .control(
                        widget::text_input(fl!("keyring-name"), name.as_str())
                            .label(fl!("keyring-name"))
                            .id(app.dialog_input_id.clone())
                            .on_input(Message::DialogInput)
                            .on_submit_maybe(valid.then_some(move |_| submit.clone())),
                    )
                    .primary_action(
                        widget::button::suggested(fl!("rename-keyring-confirm"))
                            .on_press_maybe(valid.then_some(confirm)),
                    )
                    .secondary_action(
                        widget::button::standard(fl!("cancel")).on_press(Message::CloseDialog),
                    )
                    .into(),
            )
        }
        Dialog::DeleteKeyring(id) => {
            let name = app
                .store
                .as_ref()
                .and_then(|s| s.collection(id))
                .map_or_else(|| fl!("keyring-untitled"), collection_name);
            Some(
                widget::dialog()
                    .title(fl!("delete-keyring-title", keyring = name))
                    .body(fl!("delete-keyring-body"))
                    .icon(widget::icon::from_name("edit-delete-symbolic").size(64))
                    .primary_action(
                        widget::button::destructive(fl!("delete-keyring-confirm"))
                            .on_press(Message::ConfirmDeleteKeyring(id.clone())),
                    )
                    .secondary_action(
                        widget::button::standard(fl!("cancel")).on_press(Message::CloseDialog),
                    )
                    .into(),
            )
        }
    }
}

pub fn settings(app: &App) -> Element<'_, Message> {
    let selected = REVEAL_CHOICES
        .iter()
        .position(|&s| s == app.config.reveal_seconds);
    widget::column::with_capacity(2)
        .push(
            widget::settings::section()
                .title(fl!("settings-secrets"))
                .add(widget::settings::flex_item(
                    fl!("settings-reveal-duration"),
                    widget::dropdown(&app.reveal_options, selected, Message::SetRevealChoice),
                )),
        )
        .push(
            widget::settings::section()
                .title(fl!("settings-clipboard"))
                .add(widget::text::body(fl!(
                    "settings-clipboard-body",
                    seconds = super::secrets::CLIPBOARD_CLEAR_AFTER.as_secs()
                ))),
        )
        .spacing(16)
        .into()
}

pub fn main(app: &App) -> Element<'_, Message> {
    match &app.connection {
        ConnectionState::Connecting => status_page(
            "network-transmit-receive-symbolic",
            fl!("connecting-title"),
            None,
            None,
        ),
        ConnectionState::Failed(error) => {
            let (title, body) = match error {
                Error::NoService => (fl!("no-service-title"), fl!("no-service-body")),
                Error::ActivationFailed => (
                    fl!("activation-failed-title"),
                    fl!("activation-failed-body"),
                ),
                Error::PermissionDenied => (
                    fl!("permission-denied-title"),
                    fl!("permission-denied-body"),
                ),
                other => (
                    fl!("connection-failed-title"),
                    fl!("connection-failed-body", reason = error_reason(other)),
                ),
            };
            status_page(
                "dialog-error-symbolic",
                title,
                Some(body),
                Some((fl!("retry"), Message::Connect)),
            )
        }
        ConnectionState::Lost => status_page(
            "dialog-warning-symbolic",
            fl!("connection-lost-title"),
            Some(fl!("connection-lost-body")),
            Some((fl!("reconnect"), Message::Connect)),
        ),
        ConnectionState::Connected => connected(app),
    }
}

fn status_page(
    icon: &'static str,
    title: String,
    body: Option<String>,
    action: Option<(String, Message)>,
) -> Element<'static, Message> {
    let mut column = widget::column::with_capacity(4)
        .push(widget::icon::from_name(icon).size(64))
        .push(widget::text::title3(title))
        .spacing(12)
        .align_x(Alignment::Center)
        .max_width(480);
    if let Some(body) = body {
        column = column.push(widget::text::body(body).align_x(Alignment::Center));
    }
    if let Some((label, message)) = action {
        column = column.push(widget::button::suggested(label).on_press(message));
    }
    widget::container(column)
        .center(Length::Fill)
        .padding(16)
        .into()
}

fn connected(app: &App) -> Element<'_, Message> {
    let Some(store) = app.store.as_ref() else {
        return status_page(
            "dialog-error-symbolic",
            fl!("status-disconnected"),
            None,
            None,
        );
    };
    match store.collections() {
        Load::Loading => return centered_text(fl!("keyrings-loading")),
        Load::Failed(error) => {
            return status_page(
                "dialog-error-symbolic",
                fl!("keyrings-failed", reason = error_reason(error)),
                None,
                Some((fl!("retry"), Message::Refresh)),
            );
        }
        Load::Loaded(list) if list.is_empty() => return centered_text(fl!("keyrings-none")),
        Load::Loaded(_) => {}
    }

    let mut column = widget::column::with_capacity(3);
    if let Some(unlock) = &app.unlock {
        column = column.push(unlock_banner(unlock.show_hint));
    }
    if let Some(write) = &app.write {
        column = column.push(write_banner(write.show_hint));
    }
    if let Some(editor) = &app.editor {
        return column
            .push(editor_view(app, editor))
            .height(Length::Fill)
            .into();
    }

    let searching = app.search.as_deref().is_some_and(|q| !q.trim().is_empty());
    let list = if searching {
        search_pane(app)
    } else {
        keyring_pane(app)
    };
    let details = details_pane(app);

    let body: Element<_> = if app.single_pane() {
        if app.selected_item_info().is_some() {
            details
        } else {
            list
        }
    } else {
        widget::row::with_capacity(3)
            .push(
                widget::container(list)
                    .width(Length::FillPortion(2))
                    .height(Length::Fill),
            )
            .push(widget::divider::vertical::default())
            .push(
                widget::container(details)
                    .width(Length::FillPortion(3))
                    .height(Length::Fill),
            )
            .into()
    };

    column.push(body).height(Length::Fill).into()
}

fn write_banner(show_hint: bool) -> Element<'static, Message> {
    let mut text = widget::column::with_capacity(2)
        .push(widget::text::body(fl!("write-waiting")))
        .spacing(4)
        .width(Length::Fill);
    if show_hint {
        text = text.push(widget::text::caption(fl!("write-waiting-hint")));
    }
    widget::row::with_capacity(2)
        .push(text)
        .push(widget::button::standard(fl!("cancel")).on_press(Message::CancelWrite))
        .spacing(12)
        .align_y(Alignment::Center)
        .apply(widget::container)
        .padding(12)
        .class(cosmic::theme::Container::Card)
        .apply(widget::container)
        .padding([8, 12])
        .into()
}

fn unlock_banner(show_hint: bool) -> Element<'static, Message> {
    let mut text = widget::column::with_capacity(2)
        .push(widget::text::body(fl!("unlock-waiting")))
        .spacing(4)
        .width(Length::Fill);
    if show_hint {
        text = text.push(widget::text::caption(fl!("unlock-waiting-hint")));
    }
    widget::row::with_capacity(2)
        .push(text)
        .push(widget::button::standard(fl!("cancel")).on_press(Message::CancelUnlock))
        .spacing(12)
        .align_y(Alignment::Center)
        .apply(widget::container)
        .padding(12)
        .class(cosmic::theme::Container::Card)
        .apply(widget::container)
        .padding([8, 12])
        .into()
}

fn centered_text(text: String) -> Element<'static, Message> {
    widget::container(widget::text::body(text).align_x(Alignment::Center))
        .center(Length::Fill)
        .padding(16)
        .into()
}

fn keyring_pane(app: &App) -> Element<'_, Message> {
    let Some(collection) = app.active_collection() else {
        return centered_text(fl!("select-keyring"));
    };
    let store = app.store.as_ref().expect("connected");

    let mut status = Vec::new();
    status.push(match collection.locked {
        Some(true) => fl!("keyring-locked"),
        Some(false) => fl!("keyring-unlocked"),
        None => fl!("keyring-unavailable"),
    });
    if collection.is_default == Some(true) {
        status.push(fl!("keyring-default"));
    }

    let action: Option<Element<_>> = match collection.locked {
        Some(true) => Some(
            widget::button::suggested(fl!("unlock-keyring"))
                .on_press_maybe(app.unlock.is_none().then_some(Message::Unlock))
                .into(),
        ),
        Some(false) => Some(
            widget::row::with_capacity(2)
                .push(
                    widget::button::standard(fl!("new-entry"))
                        .leading_icon(widget::icon::from_name("list-add-symbolic"))
                        .on_press_maybe(app.write.is_none().then_some(Message::NewEntry)),
                )
                .push(widget::button::standard(fl!("lock-keyring")).on_press(Message::RequestLock))
                .spacing(8)
                // Narrow list panes put the second button on its own line
                // instead of wrapping its label inside the button.
                .wrap()
                .vertical_spacing(8)
                .into(),
        ),
        None => None,
    };

    let mut header = widget::column::with_capacity(3)
        .push(widget::text::title4(collection_name(collection)).wrapping(Wrapping::WordOrGlyph))
        .push(widget::text::caption(status.join(" · ")))
        .spacing(4);
    if let Some(action) = action {
        header = header.push(widget::container(action).padding([4, 0, 0, 0]));
    }

    let content: Element<_> = match store.items(&collection.id) {
        None | Some(Load::Loading) => centered_text(fl!("entries-loading")),
        Some(Load::Failed(error)) => {
            centered_text(fl!("entries-failed", reason = error_reason(error)))
        }
        Some(Load::Loaded(_)) => {
            let visible = store.visible_items(&collection.id).expect("loaded");
            if visible.items.is_empty() && visible.hidden > 0 {
                locked_placeholder()
            } else if visible.items.is_empty() {
                widget::column::with_capacity(2)
                    .push(widget::text::body(fl!("entries-none")))
                    .push(widget::text::caption(fl!("entries-none-note")))
                    .spacing(8)
                    .apply(widget::container)
                    .center(Length::Fill)
                    .padding(16)
                    .into()
            } else {
                let mut column = widget::column::with_capacity(2).spacing(8);
                if visible.hidden > 0 {
                    column = column.push(widget::text::caption(fl!(
                        "entries-hidden",
                        count = visible.hidden
                    )));
                }
                column
                    .push(entry_list(app, visible.items.into_iter(), false))
                    .apply(widget::scrollable)
                    .id(app.entries_scroll_id.clone())
                    .height(Length::Fill)
                    .into()
            }
        }
    };

    widget::column::with_capacity(2)
        .push(header)
        .push(content)
        .spacing(12)
        .padding([12, 16])
        .height(Length::Fill)
        .into()
}

fn locked_placeholder() -> Element<'static, Message> {
    widget::column::with_capacity(3)
        .push(widget::icon::from_name("changes-prevent-symbolic").size(48))
        .push(widget::text::title4(fl!("entries-locked")))
        .push(widget::text::body(fl!("entries-locked-body")).align_x(Alignment::Center))
        .spacing(8)
        .align_x(Alignment::Center)
        .max_width(420)
        .apply(widget::container)
        .center(Length::Fill)
        .padding(16)
        .into()
}

fn search_pane(app: &App) -> Element<'_, Message> {
    let store = app.store.as_ref().expect("connected");
    let result = store.search(app.search.as_deref().unwrap_or_default());

    let mut column = widget::column::with_capacity(3)
        .push(widget::text::title4(fl!(
            "search-results",
            count = result.matches.len()
        )))
        .spacing(8);

    if result.is_limited() {
        let mut reasons = Vec::new();
        if result.undisclosed > 0 {
            reasons.push(fl!("search-limited-locked", count = result.undisclosed));
        }
        if result.failed_collections > 0 {
            reasons.push(fl!(
                "search-limited-failed",
                count = result.failed_collections
            ));
        }
        if result.loading_collections > 0 {
            reasons.push(fl!(
                "search-limited-loading",
                count = result.loading_collections
            ));
        }
        column = column.push(widget::text::caption(fl!(
            "search-limited",
            reasons = reasons.join(", ")
        )));
    }

    if !result.matches.is_empty() {
        column = column.push(entry_list(app, result.matches.into_iter(), true));
    }

    column
        .apply(widget::scrollable)
        .id(app.entries_scroll_id.clone())
        .height(Length::Fill)
        .apply(widget::container)
        .padding([12, 16])
        .into()
}

fn entry_list<'a>(
    app: &'a App,
    items: impl Iterator<Item = &'a ItemInfo>,
    show_keyring: bool,
) -> Element<'a, Message> {
    let store = app.store.as_ref().expect("connected");
    let mut list = widget::list_column();
    for item in items {
        let label = entry_label(item, LIST_LABEL_CHARS);
        let mut text = widget::column::with_capacity(2)
            .push(widget::text::body(label).wrapping(Wrapping::WordOrGlyph));
        if show_keyring {
            let keyring = store
                .collection(&item.collection)
                .map_or_else(|| fl!("keyring-untitled"), collection_name);
            text = text.push(widget::text::caption(keyring));
        }
        let mut row = widget::row::with_capacity(2)
            .push(text.width(Length::Fill))
            .align_y(Alignment::Center)
            .spacing(8);
        if item.locked == Some(true) {
            row = row.push(widget::icon::from_name("changes-prevent-symbolic").size(16));
        }
        list = list.add(
            widget::list::button(row)
                .selected(app.selected_item.as_ref() == Some(&item.id))
                .on_press(Message::SelectItem(item.id.clone())),
        );
    }
    list.into()
}

fn entry_label(item: &ItemInfo, max: usize) -> String {
    match &item.label {
        Some(label) => display_line(Some(label), max).unwrap_or_else(|| fl!("entry-untitled")),
        None => fl!("entry-locked"),
    }
}

#[allow(clippy::too_many_lines)]
fn details_pane(app: &App) -> Element<'_, Message> {
    let Some(item) = app.selected_item_info() else {
        return centered_text(fl!("select-entry"));
    };
    let store = app.store.as_ref().expect("connected");
    let collection: Option<&CollectionInfo> = store.collection(&item.collection);

    let mut column = widget::column::with_capacity(8).spacing(16);

    if app.single_pane() {
        column = column.push(
            widget::button::text(fl!("back"))
                .leading_icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(Message::Back),
        );
    }

    column = column.push(
        widget::text::title3(entry_label(item, DETAIL_TEXT_CHARS)).wrapping(Wrapping::WordOrGlyph),
    );
    if let Some(label) = &item.label {
        let count = label.chars().count();
        if count > DETAIL_TEXT_CHARS {
            column = column.push(widget::text::caption(fl!(
                "details-label-shortened",
                count = count
            )));
        }
    }
    if let Some(error) = &item.error {
        column = column.push(widget::text::body(fl!(
            "details-unavailable",
            reason = error_reason(error)
        )));
    }

    if item.locked == Some(false) {
        let idle = app.write.is_none();
        column = column.push(
            widget::row::with_capacity(2)
                .push(
                    widget::button::standard(fl!("edit-entry"))
                        .leading_icon(widget::icon::from_name("document-edit-symbolic"))
                        .on_press_maybe(
                            (idle && item.attributes.is_some()).then_some(Message::EditEntry),
                        ),
                )
                .push(
                    widget::button::destructive(fl!("delete-entry"))
                        .leading_icon(widget::icon::from_name("edit-delete-symbolic"))
                        .on_press_maybe(idle.then_some(Message::RequestDeleteEntry)),
                )
                .spacing(8),
        );
    }

    column = column.push(secret_section(app));

    let mut info = widget::settings::section()
        .add(widget::settings::item(
            fl!("details-keyring"),
            widget::text::body(collection.map_or_else(|| fl!("keyring-untitled"), collection_name)),
        ))
        .add(widget::settings::item(
            fl!("details-status"),
            widget::text::body(match item.locked {
                Some(true) => fl!("keyring-locked"),
                Some(false) => fl!("keyring-unlocked"),
                None => fl!("keyring-unavailable"),
            }),
        ));
    if let Some(created) = item.created {
        info = info.add(widget::settings::item(
            fl!("details-created"),
            widget::text::body(format_time(created)),
        ));
    }
    if let Some(modified) = item.modified {
        info = info.add(widget::settings::item(
            fl!("details-modified"),
            widget::text::body(format_time(modified)),
        ));
    }
    if let Some(content_type) = item
        .attributes
        .as_ref()
        .and_then(|a| a.get("xdg:content-type"))
    {
        info = info.add(widget::settings::item(
            fl!("details-content-type"),
            widget::text::body(display_line(Some(content_type), 100).unwrap_or_default()),
        ));
    }
    column = column.push(info);

    let mut attributes = widget::settings::section().title(fl!("details-attributes"));
    match &item.attributes {
        None => {
            attributes = attributes.add(widget::text::body(fl!("details-attributes-hidden")));
        }
        Some(map) if map.is_empty() => {
            attributes = attributes.add(widget::text::body(fl!("details-no-attributes")));
        }
        Some(map) => {
            for (key, value) in map {
                attributes = attributes.add(
                    widget::column::with_capacity(2)
                        .push(widget::text::caption(
                            display_line(Some(key), 200).unwrap_or_default(),
                        ))
                        .push(
                            widget::selectable_text(
                                display_line(Some(value), DETAIL_TEXT_CHARS).unwrap_or_default(),
                            )
                            .wrapping(Wrapping::WordOrGlyph),
                        )
                        .spacing(2),
                );
            }
        }
    }
    column = column.push(attributes);

    column = column.push(
        widget::button::text(fl!("details-technical"))
            .trailing_icon(widget::icon::from_name(if app.show_technical {
                "go-up-symbolic"
            } else {
                "go-down-symbolic"
            }))
            .on_press(Message::ToggleTechnical),
    );
    if app.show_technical {
        column = column.push(
            widget::settings::section()
                .add(
                    widget::column::with_capacity(2)
                        .push(widget::text::caption(fl!("details-object-path")))
                        .push(widget::selectable_text(item.id.path().to_owned())),
                )
                .add(
                    widget::column::with_capacity(2)
                        .push(widget::text::caption(fl!("details-keyring-path")))
                        .push(widget::selectable_text(item.collection.path().to_owned())),
                ),
        );
    }

    column
        .padding([12, 16])
        .apply(widget::scrollable)
        .height(Length::Fill)
        .into()
}

/// The Secret section of the details pane. The secret is never selectable
/// text, so copies go through Copy and its clipboard clearing.
fn secret_section(app: &App) -> Element<'_, Message> {
    let selected = app.selected_item.as_ref();
    let state = match &app.secret {
        state if state.item() == selected => state,
        _ => &SecretState::Hidden,
    };
    let fetching = matches!(state, SecretState::Fetching { .. });
    let waiting = fetching || app.unlock.is_some();

    let reveal = widget::button::standard(fl!("secret-reveal"))
        .on_press_maybe((!waiting).then_some(Message::RevealSecret));
    let hide = widget::button::standard(fl!("secret-hide")).on_press(Message::HideSecret);
    let copy = |label: String, enabled: bool| {
        widget::button::standard(label)
            .leading_icon(widget::icon::from_name("edit-copy-symbolic"))
            .on_press_maybe((enabled && !waiting).then_some(Message::CopySecret))
    };

    let mut body = widget::column::with_capacity(3).spacing(8);
    let mut buttons = widget::row::with_capacity(2).spacing(8);

    match state {
        SecretState::Hidden | SecretState::AwaitingCopyConfirm { .. } => {
            body = body.push(widget::text::body(fl!("secret-hidden")));
            buttons = buttons.push(reveal).push(copy(fl!("secret-copy"), true));
        }
        SecretState::Fetching { .. } => {
            body = body.push(widget::text::body(fl!("secret-retrieving")));
            buttons = buttons.push(reveal).push(copy(fl!("secret-copy"), false));
        }
        SecretState::Failed { error, .. } => {
            body = body.push(widget::text::body(fl!(
                "secret-failed",
                reason = error_reason(error)
            )));
            buttons = buttons.push(reveal).push(copy(fl!("secret-copy"), true));
        }
        SecretState::Shown {
            value,
            content_type,
            expires_at,
            ..
        } => {
            let remaining = expires_at
                .saturating_duration_since(std::time::Instant::now())
                .as_secs()
                + 1;
            let content_type =
                display_line(Some(content_type), 100).unwrap_or_else(|| fl!("secret-unknown-type"));
            match value {
                SecretValue::Text(text) => {
                    if text.is_empty() {
                        body = body.push(widget::text::body(fl!("secret-empty")));
                    } else {
                        body = body.push(secret_text(text.expose().to_owned()));
                    }
                    buttons = buttons.push(hide).push(copy(fl!("secret-copy"), true));
                }
                SecretValue::TextWithControl(text) => {
                    body = body
                        .push(secret_text(text.escaped().expose().to_owned()))
                        .push(widget::text::caption(fl!("secret-control-note")));
                    buttons = buttons
                        .push(hide)
                        .push(copy(fl!("secret-copy-exact"), true));
                }
                SecretValue::Binary { len } => {
                    body = body.push(widget::text::body(fl!("secret-binary", bytes = len)));
                    buttons = buttons.push(hide).push(copy(fl!("secret-copy"), false));
                }
            }
            body = body.push(widget::text::caption(fl!(
                "secret-hides-in",
                seconds = remaining,
                content_type = content_type
            )));
        }
    }

    widget::settings::section()
        .title(fl!("secret-title"))
        .add(body.push(buttons))
        .into()
}

/// Shows secret text in a monospace font. Deliberately not selectable.
fn secret_text(text: String) -> Element<'static, Message> {
    widget::text::body(text)
        .font(cosmic::font::mono())
        .wrapping(Wrapping::Glyph)
        .into()
}

fn format_time(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// The form for creating or editing an entry.
#[allow(clippy::too_many_lines)]
fn editor_view<'a>(app: &'a App, editor: &'a Editor) -> Element<'a, Message> {
    let store = app.store.as_ref();
    let editing = editor.stage == Stage::Editing;
    let busy = !editing || app.write.is_some();
    let field = |enabled: bool| enabled && !busy;

    let title = match editor.target.item() {
        None => fl!(
            "editor-new-title",
            keyring = store
                .and_then(|s| s.collection(editor.target.collection()))
                .map_or_else(|| fl!("keyring-untitled"), collection_name)
        ),
        Some(item) => fl!(
            "editor-edit-title",
            entry = store
                .and_then(|s| s.item(item))
                .map_or_else(|| fl!("entry-untitled"), |i| entry_label(i, 80))
        ),
    };

    let mut column = widget::column::with_capacity(8)
        .push(widget::text::title3(title).wrapping(Wrapping::WordOrGlyph))
        .spacing(16);

    if app.writes_untested() {
        column = column.push(widget::text::caption(fl!("writes-untested")));
    }
    if editor.changed_elsewhere && editor.notice.is_none() {
        column = column.push(notice_card(
            fl!("editor-changed-elsewhere"),
            vec![(fl!("editor-reload"), Message::EditorReload, false)],
        ));
    }
    if let Some(notice) = &editor.notice {
        column = column.push(match notice {
            Notice::Duplicates { count, single } => {
                let mut actions = vec![(fl!("editor-save-anyway"), Message::EditorConfirm, true)];
                if let Some(item) = single
                    && editor.is_new()
                {
                    actions.push((
                        fl!("editor-edit-existing"),
                        Message::EditorOpenExisting(item.clone()),
                        false,
                    ));
                }
                notice_card(fl!("editor-duplicates", count = count), actions)
            }
            Notice::Conflict => notice_card(
                fl!("editor-conflict"),
                vec![
                    (fl!("editor-overwrite"), Message::EditorConfirm, true),
                    (fl!("editor-reload"), Message::EditorReload, false),
                ],
            ),
            Notice::Deleted => notice_card(fl!("editor-deleted"), Vec::new()),
            Notice::Locked => notice_card(fl!("editor-locked"), Vec::new()),
            Notice::Invalid(text) | Notice::Failed(text) => notice_card(text.clone(), Vec::new()),
        });
    }

    let label = on_input_maybe(
        widget::text_input(fl!("editor-label-placeholder"), editor.label.as_str())
            .label(fl!("editor-label"))
            .id(editor.label_id.clone()),
        field(true).then_some(Message::EditorLabel),
    );
    column = column.push(label);

    // Secret
    let mut secret = widget::column::with_capacity(4).spacing(8);
    if !editor.is_new() {
        secret = secret
            .push(
                widget::checkbox(editor.replace_secret)
                    .label(fl!("editor-replace-secret"))
                    .on_toggle_maybe(field(true).then_some(Message::EditorReplaceSecret)),
            )
            .push(widget::text::caption(fl!("editor-replace-secret-note")));
    }
    if editor.replace_secret {
        secret = secret.push(on_input_maybe(
            widget::secure_input(
                fl!("editor-secret"),
                editor.secret.as_str(),
                field(true).then_some(Message::EditorSecretVisible),
                editor.secret_hidden,
            )
            .label(fl!("editor-secret")),
            field(true)
                .then_some(|text| Message::EditorSecret(super::editor::TypedSecret::new(text))),
        ));
        let note = if editor.secret.is_empty() && editor.is_dirty() {
            fl!("editor-secret-empty")
        } else {
            fl!("editor-secret-note")
        };
        secret = secret.push(widget::text::caption(note));
    }
    column = column.push(secret);

    // Attributes
    let mut attributes = widget::column::with_capacity(editor.attributes.len() + 3)
        .push(widget::text::heading(fl!("editor-attributes")))
        .push(widget::text::caption(fl!("editor-attributes-note")))
        .spacing(8);
    for (index, row) in editor.attributes.iter().enumerate() {
        attributes = attributes.push(
            widget::row::with_capacity(3)
                .push(
                    on_input_maybe(
                        widget::text_input(fl!("editor-attribute-name"), row.key.as_str()),
                        field(true).then_some(move |t| Message::EditorAttributeKey(index, t)),
                    )
                    .width(Length::FillPortion(2)),
                )
                .push(
                    on_input_maybe(
                        widget::text_input(fl!("editor-attribute-value"), row.value.as_str()),
                        field(true).then_some(move |t| Message::EditorAttributeValue(index, t)),
                    )
                    .width(Length::FillPortion(3)),
                )
                .push(widget::tooltip(
                    widget::button::icon(widget::icon::from_name("list-remove-symbolic"))
                        .on_press_maybe(
                            field(true).then_some(Message::EditorRemoveAttribute(index)),
                        ),
                    widget::text(fl!("editor-remove-attribute")),
                    widget::tooltip::Position::Bottom,
                ))
                .spacing(8)
                .align_y(Alignment::Center),
        );
    }
    attributes = attributes.push(
        widget::button::text(fl!("editor-add-attribute"))
            .leading_icon(widget::icon::from_name("list-add-symbolic"))
            .on_press_maybe(field(true).then_some(Message::EditorAddAttribute)),
    );
    column = column.push(attributes);

    // Actions
    let deleted = editor.notice == Some(Notice::Deleted);
    let save_label = match editor.stage {
        Stage::Checking(_) => fl!("editor-checking"),
        Stage::Saving => fl!("editor-saving"),
        Stage::Editing if editor.is_new() => fl!("editor-create"),
        Stage::Editing => fl!("editor-save"),
    };
    column = column.push(
        widget::row::with_capacity(3)
            .push(widget::space::horizontal())
            .push(
                widget::button::standard(fl!("cancel"))
                    .on_press_maybe(app.write.is_none().then_some(Message::EditorCancel)),
            )
            .push(
                widget::button::suggested(save_label)
                    .on_press_maybe(field(!deleted).then_some(Message::EditorSave)),
            )
            .spacing(8),
    );

    column
        .max_width(720)
        .padding([12, 16])
        .apply(widget::container)
        .center_x(Length::Fill)
        .apply(widget::scrollable)
        .height(Length::Fill)
        .into()
}

/// A highlighted message with optional actions; `true` marks the main one.
fn notice_card(text: String, actions: Vec<(String, Message, bool)>) -> Element<'static, Message> {
    let mut column = widget::column::with_capacity(2)
        .push(widget::text::body(text).wrapping(Wrapping::WordOrGlyph))
        .spacing(8);
    if !actions.is_empty() {
        let mut row = widget::row::with_capacity(actions.len()).spacing(8);
        for (label, message, main) in actions {
            row = row.push(if main {
                widget::button::suggested(label).on_press(message)
            } else {
                widget::button::standard(label).on_press(message)
            });
        }
        column = column.push(row);
    }
    column
        .width(Length::Fill)
        .apply(widget::container)
        .padding(12)
        .class(cosmic::theme::Container::Card)
        .into()
}

/// Makes a text input editable only when `on_input` is given.
fn on_input_maybe<'a>(
    input: widget::TextInput<'a, Message>,
    on_input: Option<impl Fn(String) -> Message + 'a>,
) -> widget::TextInput<'a, Message> {
    match on_input {
        Some(f) => input.on_input(f),
        None => input,
    }
}
