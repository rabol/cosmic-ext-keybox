// SPDX-License-Identifier: GPL-3.0-only

//! Creating and editing text entries.
//!
//! Saving is a two-step process. First a check re-reads what is stored now:
//! for an existing entry, whether it changed or disappeared since editing
//! began; for any new attribute set, which other entries already have those
//! attributes. Findings are shown and need an explicit choice. Only then are
//! the changes sent. The service offers no atomic compare-and-set, so a
//! change made by another application between the check and the save can
//! still be overwritten; the editor says so instead of promising otherwise.
//!
//! A new secret is typed into a password field and sent only over the
//! encrypted session. The text field widget keeps its own copy of what is
//! typed, which Keybox cannot wipe.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use cosmic::prelude::*;
use cosmic::widget;
use zeroize::Zeroizing;

use keybox::backend::{Client, Error, ItemInfo, ObjectId};
use keybox::secret::SecretText;

use super::{App, Message, PendingWrite, WriteDone};
use crate::fl;

/// Text typed into the secret field. Messages must be `Clone + Debug`; this
/// wrapper shares one wiping buffer and never prints it.
#[derive(Clone)]
pub struct TypedSecret(Arc<Zeroizing<String>>);

impl std::fmt::Debug for TypedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TypedSecret([REDACTED])")
    }
}

impl TypedSecret {
    pub fn new(text: String) -> Self {
        Self(Arc::new(Zeroizing::new(text)))
    }

    /// The text, moved out if this is the only reference, else copied.
    pub fn take(self) -> String {
        match Arc::try_unwrap(self.0) {
            Ok(mut text) => std::mem::take(&mut *text),
            Err(shared) => shared.as_str().to_owned(),
        }
    }
}

/// What an existing entry looked like when editing began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    label: Option<String>,
    attributes: Option<BTreeMap<String, String>>,
    modified: Option<SystemTime>,
}

impl Snapshot {
    fn of(info: &ItemInfo) -> Self {
        Self {
            label: info.label.clone(),
            attributes: info.attributes.clone(),
            modified: info.modified,
        }
    }

    /// Whether `info` differs from this snapshot in anything Keybox can see.
    fn differs(&self, info: &ItemInfo) -> bool {
        let modified = match (self.modified, info.modified) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        };
        modified || self.label != info.label || self.attributes != info.attributes
    }
}

pub enum Target {
    New {
        collection: ObjectId,
    },
    Existing {
        collection: ObjectId,
        item: ObjectId,
        snapshot: Snapshot,
    },
}

impl Target {
    pub fn collection(&self) -> &ObjectId {
        match self {
            Self::New { collection } | Self::Existing { collection, .. } => collection,
        }
    }

    pub fn item(&self) -> Option<&ObjectId> {
        match self {
            Self::New { .. } => None,
            Self::Existing { item, .. } => Some(item),
        }
    }
}

pub struct AttributeRow {
    pub key: String,
    pub value: String,
}

/// Something the user must see or decide before saving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Other entries in the keyring have all of these attributes.
    Duplicates {
        count: usize,
        single: Option<ObjectId>,
    },
    /// The stored entry changed since editing began.
    Conflict,
    /// The stored entry no longer exists.
    Deleted,
    /// The keyring is locked; nothing can be saved.
    Locked,
    /// Input that cannot be saved.
    Invalid(String),
    /// Saving failed, perhaps partly.
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Editing,
    Checking(u64),
    Saving,
}

#[allow(clippy::struct_excessive_bools)]
pub struct Editor {
    pub target: Target,
    pub label: String,
    pub attributes: Vec<AttributeRow>,
    /// For an existing entry: whether to replace its secret. Always true
    /// for a new entry.
    pub replace_secret: bool,
    pub secret: Zeroizing<String>,
    pub secret_hidden: bool,
    pub stage: Stage,
    pub notice: Option<Notice>,
    /// Another application changed the entry while it was being edited.
    pub changed_elsewhere: bool,
    pub label_id: widget::Id,
    /// Choices already confirmed for the next save attempt.
    allow_duplicates: bool,
    allow_overwrite: bool,
}

impl Editor {
    pub fn new_entry(collection: ObjectId) -> Self {
        Self {
            target: Target::New { collection },
            label: String::new(),
            attributes: Vec::new(),
            replace_secret: true,
            secret: Zeroizing::new(String::new()),
            secret_hidden: true,
            stage: Stage::Editing,
            notice: None,
            changed_elsewhere: false,
            label_id: widget::Id::unique(),
            allow_duplicates: false,
            allow_overwrite: false,
        }
    }

    pub fn edit(info: &ItemInfo) -> Self {
        let attributes = info
            .attributes
            .iter()
            .flatten()
            .map(|(key, value)| AttributeRow {
                key: key.clone(),
                value: value.clone(),
            })
            .collect();
        Self {
            target: Target::Existing {
                collection: info.collection.clone(),
                item: info.id.clone(),
                snapshot: Snapshot::of(info),
            },
            label: info.label.clone().unwrap_or_default(),
            attributes,
            replace_secret: false,
            ..Self::new_entry(info.collection.clone())
        }
    }

    pub fn is_new(&self) -> bool {
        matches!(self.target, Target::New { .. })
    }

    /// Whether anything was typed or changed.
    pub fn is_dirty(&self) -> bool {
        match &self.target {
            Target::New { .. } => {
                !self.label.is_empty() || !self.secret.is_empty() || !self.attributes.is_empty()
            }
            Target::Existing { snapshot, .. } => {
                self.replace_secret
                    || Some(&self.label) != snapshot.label.as_ref()
                    || self.attribute_map().ok() != snapshot.attributes
            }
        }
    }

    /// The attributes as entered. Rows with an empty key and value are
    /// ignored; an empty key with a value, or a repeated key, is an error.
    pub fn attribute_map(&self) -> Result<BTreeMap<String, String>, String> {
        let mut map = BTreeMap::new();
        for row in &self.attributes {
            if row.key.is_empty() && row.value.is_empty() {
                continue;
            }
            if row.key.trim().is_empty() {
                return Err(fl!("editor-invalid-attribute-name"));
            }
            if map.insert(row.key.clone(), row.value.clone()).is_some() {
                return Err(fl!(
                    "editor-invalid-attribute-repeated",
                    name = keybox::model::display_line(Some(&row.key), 60).unwrap_or_default()
                ));
            }
        }
        Ok(map)
    }

    /// Any edit after a finding invalidates the choice made about it.
    pub fn touched(&mut self) {
        self.allow_duplicates = false;
        self.allow_overwrite = false;
        if matches!(
            self.notice,
            Some(Notice::Duplicates { .. } | Notice::Invalid(_) | Notice::Failed(_))
        ) {
            self.notice = None;
        }
    }

    /// The changes to send. `None` for unchanged parts of an existing entry.
    fn plan(&self) -> Result<Plan, String> {
        if self.label.trim().is_empty() {
            return Err(fl!("editor-invalid-label"));
        }
        let attributes = self.attribute_map()?;
        Ok(match &self.target {
            Target::New { .. } => Plan {
                label: Some(self.label.clone()),
                attributes: Some(attributes),
                secret: Some(Zeroizing::new(self.secret.as_str().to_owned())),
            },
            Target::Existing { snapshot, .. } => Plan {
                label: (snapshot.label.as_ref() != Some(&self.label)).then(|| self.label.clone()),
                attributes: (snapshot.attributes.as_ref() != Some(&attributes))
                    .then_some(attributes),
                secret: self
                    .replace_secret
                    .then(|| Zeroizing::new(self.secret.as_str().to_owned())),
            },
        })
    }
}

/// What a save will change.
struct Plan {
    label: Option<String>,
    attributes: Option<BTreeMap<String, String>>,
    secret: Option<Zeroizing<String>>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.label.is_none() && self.attributes.is_none() && self.secret.is_none()
    }
}

/// The result of re-reading stored state before saving.
#[derive(Debug, Clone)]
pub enum Check {
    Ok,
    Duplicates(Vec<ObjectId>),
    Changed,
    Gone,
    Locked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Label,
    Attributes,
    Secret,
}

/// What a save did. Changes are separate calls, so some may have been
/// applied before one failed.
#[derive(Debug, Clone)]
pub struct SaveOutcome {
    pub item: Option<ObjectId>,
    pub applied: Vec<Part>,
    pub failed: Option<(Part, Error)>,
    pub created: bool,
}

impl App {
    pub(super) fn open_new_entry(&mut self) -> Task<cosmic::Action<Message>> {
        if self.editor.is_some() || self.write.is_some() {
            return Task::none();
        }
        let Some(collection) = self.active_collection() else {
            return Task::none();
        };
        if collection.locked != Some(false) {
            return self.toast(fl!("editor-unlock-first"));
        }
        let editor = Editor::new_entry(collection.id.clone());
        let focus = widget::text_input::focus(editor.label_id.clone());
        self.hide_secret();
        self.editor = Some(editor);
        focus
    }

    pub(super) fn open_edit_entry(&mut self) -> Task<cosmic::Action<Message>> {
        if self.editor.is_some() || self.write.is_some() {
            return Task::none();
        }
        let Some(info) = self.selected_item_info() else {
            return Task::none();
        };
        if info.locked != Some(false) || info.attributes.is_none() {
            return self.toast(fl!("editor-unlock-first"));
        }
        let editor = Editor::edit(info);
        let focus = widget::text_input::focus(editor.label_id.clone());
        self.hide_secret();
        self.editor = Some(editor);
        focus
    }

    /// Closes the editor and discards what was typed.
    pub(super) fn close_editor(&mut self) {
        if self.write.as_ref().is_some_and(|w| w.editor) {
            self.cancel_write();
        }
        self.editor = None;
    }

    /// First step of saving: re-read stored state.
    pub(super) fn editor_save(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        if self.write.is_some() {
            return Task::none();
        }
        self.next_op += 1;
        let op = self.next_op;
        let Some(editor) = self.editor.as_mut() else {
            return Task::none();
        };
        if editor.stage != Stage::Editing {
            return Task::none();
        }
        let plan = match editor.plan() {
            Ok(plan) => plan,
            Err(reason) => {
                editor.notice = Some(Notice::Invalid(reason));
                return Task::none();
            }
        };
        if plan.is_empty() {
            self.editor = None;
            return Task::none();
        }
        if plan.secret.is_some() && !client.is_encrypted() {
            editor.notice = Some(Notice::Failed(fl!(
                "editor-failed",
                reason = super::error_reason(&Error::EncryptionUnavailable)
            )));
            return Task::none();
        }
        editor.stage = Stage::Checking(op);
        let collection = editor.target.collection().clone();
        let existing = match &editor.target {
            Target::New { .. } => None,
            Target::Existing { item, snapshot, .. } => Some((item.clone(), snapshot.clone())),
        };
        let attributes = plan.attributes.filter(|a| !a.is_empty());
        cosmic::task::future(async move {
            let result = check(&client, &collection, existing, attributes).await;
            Message::EditorChecked(op, result)
        })
    }

    pub(super) fn editor_checked(
        &mut self,
        op: u64,
        result: Result<Check, Error>,
    ) -> Task<cosmic::Action<Message>> {
        let Some(editor) = self.editor.as_mut() else {
            return Task::none();
        };
        if editor.stage != Stage::Checking(op) {
            return Task::none();
        }
        editor.stage = Stage::Editing;
        match result {
            Ok(Check::Duplicates(found)) if !editor.allow_duplicates => {
                editor.notice = Some(Notice::Duplicates {
                    count: found.len(),
                    single: (found.len() == 1).then(|| found[0].clone()),
                });
                return Task::none();
            }
            Ok(Check::Changed) if !editor.allow_overwrite => {
                editor.notice = Some(Notice::Conflict);
                return Task::none();
            }
            Ok(Check::Ok | Check::Duplicates(_) | Check::Changed) => {}
            Ok(Check::Gone) => {
                editor.notice = Some(Notice::Deleted);
                return Task::none();
            }
            Ok(Check::Locked) | Err(Error::Locked) => {
                editor.notice = Some(Notice::Locked);
                return Task::none();
            }
            Err(error) => {
                editor.notice = Some(Notice::Failed(fl!(
                    "editor-failed",
                    reason = super::error_reason(&error)
                )));
                return Task::none();
            }
        }
        self.editor_send()
    }

    /// Second step of saving: send the changes.
    fn editor_send(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let Some(editor) = self.editor.as_mut() else {
            return Task::none();
        };
        let Ok(plan) = editor.plan() else {
            return Task::none();
        };
        editor.stage = Stage::Saving;
        editor.notice = None;
        let target = match &editor.target {
            Target::New { collection } => Err(collection.clone()),
            Target::Existing { item, .. } => Ok(item.clone()),
        };
        self.start_write(true, async move {
            WriteDone::Entry(save(&client, target, plan).await)
        })
    }

    /// The user chose to go ahead despite the shown finding.
    pub(super) fn editor_confirm(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(editor) = self.editor.as_mut() else {
            return Task::none();
        };
        match editor.notice {
            Some(Notice::Duplicates { .. }) => editor.allow_duplicates = true,
            Some(Notice::Conflict) => {
                editor.allow_overwrite = true;
                editor.changed_elsewhere = false;
            }
            _ => return Task::none(),
        }
        editor.notice = None;
        self.editor_save()
    }

    /// Discards the edits and starts again from what is stored now.
    pub(super) fn editor_reload(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(item) = self.editor.as_ref().and_then(|e| e.target.item().cloned()) else {
            return Task::none();
        };
        self.editor = None;
        self.selected_item = Some(item);
        self.open_edit_entry()
    }

    /// Opens the one existing entry with the same attributes instead.
    pub(super) fn editor_open_existing(&mut self, item: ObjectId) -> Task<cosmic::Action<Message>> {
        self.editor = None;
        self.selected_item = Some(item);
        self.open_edit_entry()
    }

    pub(super) fn editor_saved(&mut self, outcome: SaveOutcome) -> Task<cosmic::Action<Message>> {
        let mut tasks = Vec::new();
        if let Some(collection) = self.editor.as_ref().map(|e| e.target.collection().clone()) {
            tasks.push(self.load_items(&collection));
        }
        match outcome.failed {
            None => {
                let created = outcome.created;
                self.editor = None;
                if let Some(item) = outcome.item {
                    self.selected_item = Some(item);
                }
                tasks.push(self.toast(if created {
                    fl!("editor-created")
                } else {
                    fl!("editor-saved")
                }));
            }
            Some((part, error)) => {
                let Some(editor) = self.editor.as_mut() else {
                    return Task::batch(tasks);
                };
                editor.stage = Stage::Editing;
                let reason = super::error_reason(&error);
                let text = if outcome.applied.is_empty() {
                    fl!("editor-failed", reason = reason)
                } else {
                    let applied: Vec<String> =
                        outcome.applied.iter().map(|p| part_name(*p)).collect();
                    fl!(
                        "editor-failed-partly",
                        applied = applied.join(", "),
                        part = part_name(part),
                        reason = reason
                    )
                };
                // What was applied is now the stored state; a further save
                // must not warn about our own changes.
                let entered = editor.attribute_map().ok();
                if let (Target::Existing { snapshot, .. }, false) =
                    (&mut editor.target, outcome.applied.is_empty())
                {
                    snapshot.modified = None;
                    if outcome.applied.contains(&Part::Label) {
                        snapshot.label = Some(editor.label.clone());
                    }
                    if outcome.applied.contains(&Part::Attributes) {
                        snapshot.attributes = entered;
                    }
                    if outcome.applied.contains(&Part::Secret) {
                        editor.replace_secret = false;
                    }
                }
                editor.notice = Some(match error {
                    Error::Locked => Notice::Locked,
                    Error::NoSuchObject => Notice::Deleted,
                    _ => Notice::Failed(text),
                });
            }
        }
        Task::batch(tasks)
    }

    /// Notes changes made by other applications to the entry being edited.
    pub(super) fn editor_on_item_event(&mut self, item: &ObjectId, deleted: bool) {
        let saving = matches!(&self.write, Some(PendingWrite { editor: true, .. }));
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if saving || editor.target.item() != Some(item) {
            return;
        }
        if deleted {
            editor.notice = Some(Notice::Deleted);
        } else {
            editor.changed_elsewhere = true;
        }
    }
}

async fn check(
    client: &Client,
    collection: &ObjectId,
    existing: Option<(ObjectId, Snapshot)>,
    attributes: Option<BTreeMap<String, String>>,
) -> Result<Check, Error> {
    let mut changed = false;
    if let Some((item, snapshot)) = &existing {
        match client.item(collection, item).await {
            Ok(info) if info.locked != Some(false) => return Ok(Check::Locked),
            Ok(info) => changed = snapshot.differs(&info),
            Err(Error::NoSuchObject) => return Ok(Check::Gone),
            Err(error) => return Err(error),
        }
    } else {
        let collections = client.collections().await?;
        match collections.iter().find(|c| &c.id == collection) {
            None => return Err(Error::NoSuchObject),
            Some(c) if c.locked != Some(false) => return Ok(Check::Locked),
            Some(_) => {}
        }
    }
    if let Some(attributes) = attributes {
        let own = existing.as_ref().map(|(item, _)| item);
        let found: Vec<ObjectId> = client
            .find_items(collection, &attributes)
            .await?
            .into_iter()
            .filter(|id| Some(id) != own)
            .collect();
        if !found.is_empty() {
            return Ok(Check::Duplicates(found));
        }
    }
    Ok(if changed { Check::Changed } else { Check::Ok })
}

async fn save(client: &Client, target: Result<ObjectId, ObjectId>, plan: Plan) -> SaveOutcome {
    let Plan {
        label,
        attributes,
        secret,
    } = plan;
    let secret = secret.map(SecretText::new);
    match target {
        Err(collection) => {
            let result = client
                .create_item(
                    &collection,
                    label.as_deref().unwrap_or_default(),
                    &attributes.unwrap_or_default(),
                    secret.as_ref().expect("new entries always have a secret"),
                    None,
                )
                .await;
            match result {
                Ok(item) => SaveOutcome {
                    item: Some(item),
                    applied: vec![Part::Label, Part::Attributes, Part::Secret],
                    failed: None,
                    created: true,
                },
                Err(error) => SaveOutcome {
                    item: None,
                    applied: Vec::new(),
                    failed: Some((Part::Secret, error)),
                    created: false,
                },
            }
        }
        Ok(item) => {
            let mut outcome = SaveOutcome {
                item: Some(item.clone()),
                applied: Vec::new(),
                failed: None,
                created: false,
            };
            let steps = [
                (Part::Secret, secret.is_some()),
                (Part::Attributes, attributes.is_some()),
                (Part::Label, label.is_some()),
            ];
            for (part, wanted) in steps {
                if !wanted {
                    continue;
                }
                let result = match part {
                    Part::Secret => {
                        client
                            .set_item_secret(&item, secret.as_ref().expect("checked"))
                            .await
                    }
                    Part::Attributes => {
                        client
                            .set_item_attributes(&item, attributes.as_ref().expect("checked"))
                            .await
                    }
                    Part::Label => {
                        client
                            .set_item_label(&item, label.as_deref().expect("checked"))
                            .await
                    }
                };
                match result {
                    Ok(()) => outcome.applied.push(part),
                    Err(error) => {
                        outcome.failed = Some((part, error));
                        break;
                    }
                }
            }
            outcome
        }
    }
}

fn part_name(part: Part) -> String {
    match part {
        Part::Label => fl!("editor-part-label"),
        Part::Attributes => fl!("editor-part-attributes"),
        Part::Secret => fl!("editor-part-secret"),
    }
}

/// How long before suggesting that a write may be waiting for a prompt.
pub const WRITE_HINT_DELAY: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    fn id(path: &str) -> ObjectId {
        ObjectId::detached(1, path).unwrap()
    }

    fn info() -> ItemInfo {
        ItemInfo {
            id: id("/c/1"),
            collection: id("/c"),
            label: Some("KBX Unit".into()),
            attributes: Some(BTreeMap::from([("user".into(), "kbx".into())])),
            locked: Some(false),
            created: None,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100)),
            error: None,
        }
    }

    fn row(key: &str, value: &str) -> AttributeRow {
        AttributeRow {
            key: key.into(),
            value: value.into(),
        }
    }

    #[test]
    fn attribute_rows_are_validated() {
        let mut editor = Editor::new_entry(id("/c"));
        editor.attributes = vec![row("a", "1"), row("", ""), row("b", "")];
        let map = editor.attribute_map().unwrap();
        assert_eq!(map.len(), 2, "blank rows are ignored, empty values kept");

        editor.attributes = vec![row("", "orphan")];
        assert!(editor.attribute_map().is_err());
        editor.attributes = vec![row("a", "1"), row("a", "2")];
        assert!(editor.attribute_map().is_err());
    }

    #[test]
    fn new_entry_needs_a_label_and_sends_everything() {
        let mut editor = Editor::new_entry(id("/c"));
        assert!(!editor.is_dirty());
        assert!(editor.plan().is_err());
        editor.label = "KBX New".into();
        editor.secret = Zeroizing::new("KBX-UNIT-SECRET".into());
        assert!(editor.is_dirty());
        let plan = editor.plan().unwrap();
        assert_eq!(plan.label.as_deref(), Some("KBX New"));
        assert_eq!(plan.attributes, Some(BTreeMap::new()));
        assert_eq!(
            plan.secret.as_deref().map(String::as_str),
            Some("KBX-UNIT-SECRET")
        );
    }

    #[test]
    fn editing_sends_only_changed_parts() {
        let mut editor = Editor::edit(&info());
        assert!(!editor.is_dirty());
        assert!(editor.plan().unwrap().is_empty());

        editor.label = "KBX Renamed".into();
        let plan = editor.plan().unwrap();
        assert_eq!(plan.label.as_deref(), Some("KBX Renamed"));
        assert!(plan.attributes.is_none() && plan.secret.is_none());

        editor.attributes.push(row("extra", "x"));
        editor.replace_secret = true;
        let plan = editor.plan().unwrap();
        assert_eq!(plan.attributes.map(|a| a.len()), Some(2));
        assert_eq!(plan.secret.as_deref().map(String::as_str), Some(""));
    }

    #[test]
    fn snapshot_detects_external_changes() {
        let original = info();
        let snapshot = Snapshot::of(&original);
        assert!(!snapshot.differs(&original));
        let mut relabelled = original.clone();
        relabelled.label = Some("other".into());
        assert!(snapshot.differs(&relabelled));
        let mut touched = original.clone();
        touched.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(200));
        assert!(snapshot.differs(&touched));
        let mut unknown_time = original;
        unknown_time.modified = None;
        assert!(!snapshot.differs(&unknown_time));
    }

    #[test]
    fn typed_secret_debug_is_redacted() {
        let typed = TypedSecret::new("KBX-UNIT-TYPED".into());
        assert!(!format!("{typed:?}").contains("KBX"));
        assert_eq!(typed.take(), "KBX-UNIT-TYPED");
    }
}
