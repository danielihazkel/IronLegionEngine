//! `Registry<T>`, `ContentKind`, the two-pass `Lookup` (TDD §3.2, §3.3 step 4).

use std::collections::{BTreeMap, BTreeSet};

use il_core::StateHasher;
use serde::de::DeserializeOwned;

use crate::content_id::ContentId;
use crate::handle::Handle;
use crate::schema::KindTag;

/// A reference that did not resolve, reported by `ContentKind::resolve`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveError {
    /// Field path inside the object, e.g. `formations[1]`.
    pub field: String,
    pub id: ContentId,
    /// The kind the reference should have named.
    pub kind: KindTag,
    /// Overrides the default "unknown reference" wording.
    pub message: Option<String>,
}

impl ResolveError {
    pub fn new(field: impl Into<String>, id: ContentId, kind: KindTag) -> Self {
        Self {
            field: field.into(),
            id,
            kind,
            message: None,
        }
    }

    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// A kind of content file: which folder it lives in, which schema validates
/// it, how to find its id, how to turn its references into handles and which
/// fields the content hash covers.
pub trait ContentKind: DeserializeOwned + Clone + Send + Sync + 'static {
    /// Folder under the mod's `content_root`, e.g. `"units"`.
    const DIR: &'static str;
    /// The embedded schema every merged object of this kind must satisfy.
    const TAG: KindTag;
    fn id(&self) -> &ContentId;
    /// Turns ContentId references into handles. Called once per item after
    /// every kind's ids are known, so file order never matters.
    fn resolve(&mut self, lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        let _ = (lookup, errors);
    }
    /// Writes the sim-relevant fields in a fixed order (content registry
    /// hash). Render-only kinds write nothing.
    fn hash_content(&self, h: &mut StateHasher) {
        let _ = h;
    }
}

/// ContentId → final registry index for every kind, built before any typed
/// item exists (pass 1), so pass 2 can resolve references in any order.
#[derive(Debug, Default)]
pub struct Lookup {
    tables: BTreeMap<KindTag, BTreeMap<ContentId, u32>>,
    /// Ids that exist but failed validation: a reference to one is not an
    /// error of the referencing item.
    invalid: BTreeMap<KindTag, std::collections::BTreeSet<ContentId>>,
}

impl Lookup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the ids of one kind with the indices they will occupy.
    pub fn register<'a>(
        &mut self,
        kind: KindTag,
        ids: impl IntoIterator<Item = (&'a ContentId, u32)>,
    ) {
        let table = self.tables.entry(kind).or_default();
        for (id, i) in ids {
            table.insert(id.clone(), i);
        }
    }

    pub fn handle<T: ContentKind>(&self, id: &ContentId) -> Option<Handle<T>> {
        self.tables
            .get(&T::TAG)?
            .get(id)
            .map(|&i| Handle::from_index(i))
    }

    /// Ids of a kind, for "nearest" suggestions.
    pub fn ids(&self, kind: KindTag) -> impl Iterator<Item = &ContentId> {
        self.tables.get(&kind).into_iter().flat_map(|t| t.keys())
    }

    /// Marks ids of `kind` that were defined but failed validation.
    pub fn register_invalid<'a>(
        &mut self,
        kind: KindTag,
        ids: impl IntoIterator<Item = &'a ContentId>,
    ) {
        let set = self.invalid.entry(kind).or_default();
        set.extend(ids.into_iter().cloned());
    }

    /// Whether `id` of `kind` exists but was rejected (its own diagnostics
    /// already explain why).
    pub fn is_invalid(&self, kind: KindTag, id: &ContentId) -> bool {
        self.invalid.get(&kind).is_some_and(|s| s.contains(id))
    }
}

/// Error for inserting an id twice.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("duplicate content id {0}")]
pub struct DuplicateId(pub ContentId);

/// Immutable-after-load store of one content kind, indexed by [`Handle`].
/// Iteration is in ascending index (insertion) order, so it is deterministic
/// given a deterministic load order.
pub struct Registry<T> {
    items: Vec<T>,
    ids: Vec<ContentId>,
    // Lookup only, never iterated (SIM-DET-003 allows this use).
    #[allow(clippy::disallowed_types)]
    by_id: std::collections::HashMap<ContentId, u32>,
    /// Slots whose id was deleted by a hot reload: the old item stays so held
    /// handles keep reading, but `lookup` and `iter` skip them.
    removed: BTreeSet<u32>,
}

impl<T> core::fmt::Debug for Registry<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Registry").field("ids", &self.ids).finish()
    }
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Registry<T> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            ids: Vec::new(),
            #[allow(clippy::disallowed_types)]
            by_id: std::collections::HashMap::new(),
            removed: BTreeSet::new(),
        }
    }

    /// Infallible: handles are only ever produced by this registry.
    #[inline]
    pub fn get(&self, h: Handle<T>) -> &T {
        &self.items[h.index() as usize]
    }

    pub fn lookup(&self, id: &ContentId) -> Option<Handle<T>> {
        self.by_id
            .get(id)
            .filter(|&&i| !self.removed.contains(&i))
            .map(|&i| Handle::from_index(i))
    }

    /// Every slot's id in index order, removed slots included (hot reload
    /// layout).
    pub fn all_ids(&self) -> impl Iterator<Item = &ContentId> {
        self.ids.iter()
    }

    /// `lookup` that also finds removed slots.
    pub fn lookup_any(&self, id: &ContentId) -> Option<Handle<T>> {
        self.by_id.get(id).map(|&i| Handle::from_index(i))
    }

    /// Whether a hot reload deleted this slot's id.
    pub fn is_removed(&self, h: Handle<T>) -> bool {
        self.removed.contains(&h.index())
    }

    /// Ids of removed slots.
    pub fn removed_ids(&self) -> impl Iterator<Item = &ContentId> {
        self.removed.iter().map(|&i| &self.ids[i as usize])
    }

    /// Every slot, live or removed (the layout length).
    pub fn slots(&self) -> usize {
        self.items.len()
    }

    /// Live ids at indices `>= from`, in index order.
    pub fn ids_added_after(&self, from: usize) -> impl Iterator<Item = &ContentId> {
        self.ids
            .iter()
            .enumerate()
            .skip(from)
            .filter(|(i, _)| !self.removed.contains(&(*i as u32)))
            .map(|(_, id)| id)
    }

    pub fn id_of(&self, h: Handle<T>) -> &ContentId {
        &self.ids[h.index() as usize]
    }

    /// Live items in ascending index order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.items
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.removed.contains(&(*i as u32)))
            .map(|(i, item)| (Handle::from_index(i as u32), item))
    }

    /// Live ids in ascending index order.
    pub fn ids(&self) -> impl Iterator<Item = &ContentId> {
        self.ids
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.removed.contains(&(*i as u32)))
            .map(|(_, id)| id)
    }

    /// Live items.
    pub fn len(&self) -> usize {
        self.items.len() - self.removed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, id: &ContentId) -> bool {
        self.lookup(id).is_some()
    }
}

impl<T: ContentKind> Registry<T> {
    /// Appends an item; its handle is the next index.
    pub fn insert(&mut self, item: T) -> Result<Handle<T>, DuplicateId> {
        let id = item.id().clone();
        if self.by_id.contains_key(&id) {
            return Err(DuplicateId(id));
        }
        let index = self.items.len() as u32;
        self.by_id.insert(id.clone(), index);
        self.ids.push(id);
        self.items.push(item);
        Ok(Handle::from_index(index))
    }

    /// Appends an item into a slot that is already marked removed (hot
    /// reload keeps deleted ids in place).
    pub fn insert_removed(&mut self, item: T) -> Handle<T> {
        let index = self.items.len() as u32;
        self.by_id.insert(item.id().clone(), index);
        self.ids.push(item.id().clone());
        self.items.push(item);
        self.removed.insert(index);
        Handle::from_index(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Thing {
        id: ContentId,
        value: u32,
    }
    impl ContentKind for Thing {
        const DIR: &'static str = "things";
        const TAG: KindTag = KindTag::Unit;
        fn id(&self) -> &ContentId {
            &self.id
        }
    }

    fn thing(id: &str, value: u32) -> Thing {
        Thing {
            id: ContentId::new(id).unwrap(),
            value,
        }
    }

    #[test]
    fn insert_lookup_get_iterate() {
        let mut r = Registry::new();
        let a = r.insert(thing("m:a", 1)).unwrap();
        let b = r.insert(thing("m:b", 2)).unwrap();
        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);
        assert_eq!(r.get(b).value, 2);
        assert_eq!(r.lookup(&ContentId::new("m:a").unwrap()), Some(a));
        assert_eq!(r.lookup(&ContentId::new("m:zz").unwrap()), None);
        assert_eq!(r.id_of(a).as_str(), "m:a");
        let order: Vec<u32> = r.iter().map(|(_, t)| t.value).collect();
        assert_eq!(order, vec![1, 2]);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r.insert(thing("m:a", 3)).unwrap_err(),
            DuplicateId(ContentId::new("m:a").unwrap())
        );
    }

    #[test]
    fn lookup_maps_ids_to_indices_per_kind() {
        let mut l = Lookup::new();
        let ids = [
            ContentId::new("m:b").unwrap(),
            ContentId::new("m:a").unwrap(),
        ];
        l.register(KindTag::Unit, ids.iter().zip(0u32..));
        assert_eq!(l.handle::<Thing>(&ids[1]).map(|h| h.index()), Some(1));
        assert_eq!(l.handle::<Thing>(&ContentId::new("m:zz").unwrap()), None);
        assert_eq!(l.ids(KindTag::Unit).count(), 2);
        assert_eq!(l.ids(KindTag::Faction).count(), 0);
    }
}
