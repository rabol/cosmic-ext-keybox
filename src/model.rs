// SPDX-License-Identifier: GPL-3.0-only

//! Application state that does not depend on the GUI toolkit: loaded
//! metadata, loading states, sorting and search.
//!
//! Holds metadata only. Secrets never enter this module.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::backend::{CollectionInfo, Error, ItemInfo, ObjectId};

/// The connection to the Secret Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    /// Connecting failed. The error says why (no service, activation
    /// failed, permission denied, ...).
    Failed(Error),
    /// The service went away or was replaced while connected.
    Lost,
}

/// Something being loaded. A failed or unfinished load is never shown as
/// empty.
#[derive(Debug, Clone)]
pub enum Load<T> {
    Loading,
    Loaded(T),
    Failed(Error),
}

impl<T> Load<T> {
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Self::Loaded(value) => Some(value),
            _ => None,
        }
    }
}

/// Metadata for one connection generation.
#[derive(Debug)]
pub struct Store {
    generation: u64,
    collections: Load<Vec<CollectionInfo>>,
    items: HashMap<ObjectId, Load<Vec<ItemInfo>>>,
}

impl Store {
    #[must_use]
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            collections: Load::Loading,
            items: HashMap::new(),
        }
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn collections(&self) -> &Load<Vec<CollectionInfo>> {
        &self.collections
    }

    /// Collections sorted for display: default first, then by label.
    #[must_use]
    pub fn sorted_collections(&self) -> Vec<&CollectionInfo> {
        let mut list: Vec<&CollectionInfo> = self
            .collections
            .loaded()
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        list.sort_by(|a, b| {
            b.is_default
                .unwrap_or(false)
                .cmp(&a.is_default.unwrap_or(false))
                .then_with(|| compare_labels(a.label.as_deref(), b.label.as_deref()))
                .then_with(|| a.id.path().cmp(b.id.path()))
        });
        list
    }

    #[must_use]
    pub fn collection(&self, id: &ObjectId) -> Option<&CollectionInfo> {
        self.collections.loaded()?.iter().find(|c| &c.id == id)
    }

    /// Stores a collection listing. Item lists of collections that no longer
    /// exist are dropped. Results from another generation are ignored.
    pub fn set_collections(&mut self, generation: u64, result: Result<Vec<CollectionInfo>, Error>) {
        if generation != self.generation {
            return;
        }
        match result {
            Ok(collections) => {
                self.items
                    .retain(|id, _| collections.iter().any(|c| &c.id == id));
                self.collections = Load::Loaded(collections);
            }
            Err(error) => self.collections = Load::Failed(error),
        }
    }

    #[must_use]
    pub fn items(&self, collection: &ObjectId) -> Option<&Load<Vec<ItemInfo>>> {
        self.items.get(collection)
    }

    /// Marks a collection's items as loading, keeping any previous list
    /// visible until the new one arrives.
    pub fn begin_items(&mut self, collection: &ObjectId) {
        self.items
            .entry(collection.clone())
            .or_insert(Load::Loading);
    }

    pub fn set_items(
        &mut self,
        generation: u64,
        collection: &ObjectId,
        result: Result<Vec<ItemInfo>, Error>,
    ) {
        if generation != self.generation || self.collection(collection).is_none() {
            return;
        }
        let load = match result {
            Ok(items) => Load::Loaded(items),
            Err(error) => Load::Failed(error),
        };
        self.items.insert(collection.clone(), load);
    }

    /// Replaces one entry's metadata after a change notification.
    pub fn update_item(&mut self, generation: u64, info: ItemInfo) {
        if generation != self.generation {
            return;
        }
        if let Some(Load::Loaded(items)) = self.items.get_mut(&info.collection) {
            match items.iter_mut().find(|i| i.id == info.id) {
                Some(existing) => *existing = info,
                None => items.push(info),
            }
        }
    }

    pub fn remove_item(&mut self, collection: &ObjectId, item: &ObjectId) {
        if let Some(Load::Loaded(items)) = self.items.get_mut(collection) {
            items.retain(|i| &i.id != item);
        }
    }

    #[must_use]
    pub fn item(&self, id: &ObjectId) -> Option<&ItemInfo> {
        self.items
            .values()
            .filter_map(Load::loaded)
            .flatten()
            .find(|i| &i.id == id)
    }

    /// A collection's entries whose metadata is visible, sorted by label
    /// with the object path as a stable tie-breaker. `hidden` counts
    /// entries whose label the service did not disclose (typically locked).
    #[must_use]
    pub fn visible_items(&self, collection: &ObjectId) -> Option<VisibleItems<'_>> {
        let items = self.items.get(collection)?.loaded()?;
        let mut visible: Vec<&ItemInfo> = items.iter().filter(|i| i.label.is_some()).collect();
        visible.sort_by(|a, b| compare_items(a, b));
        Some(VisibleItems {
            hidden: items.len() - visible.len(),
            items: visible,
        })
    }

    /// Case-insensitive search over loaded labels and attributes of all
    /// collections. Never retrieves secrets or unlocks anything.
    #[must_use]
    pub fn search(&self, query: &str) -> SearchResult<'_> {
        let needle = query.trim().to_lowercase();
        let mut result = SearchResult::default();
        for collection in self.sorted_collections() {
            match self.items.get(&collection.id) {
                Some(Load::Loaded(items)) => {
                    for item in items {
                        if item.label.is_none() && item.attributes.is_none() {
                            result.undisclosed += 1;
                        } else if matches(item, &needle) {
                            result.matches.push(item);
                        }
                    }
                }
                Some(Load::Failed(_)) => result.failed_collections += 1,
                Some(Load::Loading) | None => result.loading_collections += 1,
            }
        }
        result.matches.sort_by(|a, b| compare_items(a, b));
        result
    }
}

#[derive(Debug)]
pub struct VisibleItems<'a> {
    pub items: Vec<&'a ItemInfo>,
    pub hidden: usize,
}

/// Search matches plus what limited the search, so the UI can say so.
#[derive(Debug, Default)]
pub struct SearchResult<'a> {
    pub matches: Vec<&'a ItemInfo>,
    /// Entries whose label and attributes were not disclosed (locked).
    pub undisclosed: usize,
    /// Keyrings whose entries could not be listed.
    pub failed_collections: usize,
    /// Keyrings still loading.
    pub loading_collections: usize,
}

impl SearchResult<'_> {
    /// Whether some content could not be searched.
    #[must_use]
    pub fn is_limited(&self) -> bool {
        self.undisclosed > 0 || self.failed_collections > 0 || self.loading_collections > 0
    }
}

fn matches(item: &ItemInfo, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hit = |text: &str| text.to_lowercase().contains(needle);
    item.label.as_deref().is_some_and(hit)
        || item
            .attributes
            .iter()
            .flatten()
            .any(|(key, value)| hit(key) || hit(value))
}

fn compare_labels(a: Option<&str>, b: Option<&str>) -> Ordering {
    // Missing or empty labels sort last.
    let key = |label: Option<&str>| {
        label
            .filter(|l| !l.trim().is_empty())
            .map(str::to_lowercase)
    };
    match (key(a), key(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_items(a: &ItemInfo, b: &ItemInfo) -> Ordering {
    compare_labels(a.label.as_deref(), b.label.as_deref())
        .then_with(|| a.id.path().cmp(b.id.path()))
}

/// Makes untrusted metadata safe to show on one line: control characters
/// become escapes and long text is shortened. Returns `None` for missing or
/// blank text so the UI can show its fallback.
#[must_use]
pub fn display_line(text: Option<&str>, max_chars: usize) -> Option<String> {
    let text = text?;
    if text.trim().is_empty() {
        return None;
    }
    let mut out = String::new();
    for (count, c) in text.chars().enumerate() {
        if count == max_chars {
            out.push('…');
            break;
        }
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use zbus::zvariant::OwnedObjectPath;

    fn id(generation: u64, path: &str) -> ObjectId {
        ObjectId {
            generation,
            path: OwnedObjectPath::try_from(path).unwrap(),
        }
    }

    fn collection(path: &str, label: &str, default: bool) -> CollectionInfo {
        CollectionInfo {
            id: id(1, path),
            label: Some(label.into()),
            locked: Some(false),
            created: None,
            modified: None,
            is_default: Some(default),
            error: None,
        }
    }

    fn item(col: &str, path: &str, label: Option<&str>, attrs: &[(&str, &str)]) -> ItemInfo {
        ItemInfo {
            id: id(1, path),
            collection: id(1, col),
            label: label.map(Into::into),
            attributes: label.map(|_| {
                attrs
                    .iter()
                    .map(|(k, v)| ((*k).into(), (*v).into()))
                    .collect::<BTreeMap<_, _>>()
            }),
            locked: Some(label.is_none()),
            created: None,
            modified: None,
            error: None,
        }
    }

    fn store() -> Store {
        let mut store = Store::new(1);
        store.set_collections(
            1,
            Ok(vec![
                collection("/c/work", "Work", false),
                collection("/c/login", "Login", true),
                collection("/c/locked", "Locked", false),
            ]),
        );
        store.set_items(
            1,
            &id(1, "/c/login"),
            Ok(vec![
                item("/c/login", "/c/login/3", Some("beta"), &[("user", "Alice")]),
                item(
                    "/c/login",
                    "/c/login/1",
                    Some("Alpha"),
                    &[("service", "github")],
                ),
                item("/c/login", "/c/login/2", Some("alpha"), &[]),
                item("/c/login", "/c/login/4", Some(""), &[]),
            ]),
        );
        store.set_items(
            1,
            &id(1, "/c/locked"),
            Ok(vec![item("/c/locked", "/c/locked/1", None, &[])]),
        );
        store
    }

    #[test]
    fn default_collection_first() {
        let store = store();
        let labels: Vec<_> = store
            .sorted_collections()
            .iter()
            .map(|c| c.label.clone().unwrap())
            .collect();
        assert_eq!(labels, ["Login", "Locked", "Work"]);
    }

    #[test]
    fn items_sorted_case_insensitively_with_stable_tiebreak() {
        let store = store();
        let visible = store.visible_items(&id(1, "/c/login")).unwrap();
        let paths: Vec<_> = visible.items.iter().map(|i| i.id.path()).collect();
        // "Alpha" and "alpha" tie on label; path decides. Empty label last.
        assert_eq!(
            paths,
            ["/c/login/1", "/c/login/2", "/c/login/3", "/c/login/4"]
        );
    }

    #[test]
    fn undisclosed_items_are_counted_not_shown() {
        let store = store();
        let visible = store.visible_items(&id(1, "/c/locked")).unwrap();
        assert!(visible.items.is_empty());
        assert_eq!(visible.hidden, 1);
    }

    #[test]
    fn unloaded_collection_is_not_empty() {
        let store = store();
        assert!(store.visible_items(&id(1, "/c/work")).is_none());
    }

    #[test]
    fn search_matches_labels_and_attributes_and_reports_limits() {
        let store = store();
        let result = store.search("ALICE");
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].id.path(), "/c/login/3");

        let result = store.search("github");
        assert_eq!(result.matches[0].id.path(), "/c/login/1");

        let result = store.search("service");
        assert_eq!(result.matches.len(), 1, "attribute keys are searched");

        assert_eq!(result.undisclosed, 1);
        assert_eq!(result.loading_collections, 1, "Work was never loaded");
        assert!(result.is_limited());
    }

    #[test]
    fn stale_generation_results_are_ignored() {
        let mut store = store();
        store.set_collections(2, Ok(vec![]));
        assert_eq!(store.sorted_collections().len(), 3);
        store.set_items(2, &id(1, "/c/login"), Ok(vec![]));
        assert_eq!(
            store.visible_items(&id(1, "/c/login")).unwrap().items.len(),
            4
        );
    }

    #[test]
    fn removed_collection_drops_its_items() {
        let mut store = store();
        store.set_collections(1, Ok(vec![collection("/c/work", "Work", false)]));
        assert!(store.items(&id(1, "/c/login")).is_none());
    }

    #[test]
    fn item_updates_and_removal() {
        let mut store = store();
        let mut changed = item("/c/login", "/c/login/1", Some("Renamed"), &[]);
        changed.locked = Some(false);
        store.update_item(1, changed);
        assert_eq!(
            store.item(&id(1, "/c/login/1")).unwrap().label.as_deref(),
            Some("Renamed")
        );
        store.remove_item(&id(1, "/c/login"), &id(1, "/c/login/1"));
        assert!(store.item(&id(1, "/c/login/1")).is_none());
    }

    #[test]
    fn display_line_escapes_and_truncates() {
        assert_eq!(display_line(Some("a\nb\0"), 10).unwrap(), "a\\nb\\u{0}");
        assert_eq!(display_line(Some("abcdef"), 3).unwrap(), "abc…");
        assert_eq!(display_line(Some("  "), 3), None);
        assert_eq!(display_line(None, 3), None);
    }
}
