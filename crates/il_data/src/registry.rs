//! `Registry<T>` and `ContentKind` (TDD §3.2).

use serde::de::DeserializeOwned;

use crate::content_id::ContentId;
use crate::handle::Handle;

/// A kind of content file: which folder it lives in and how to find its id.
/// `SCHEMA` and `resolve` arrive with validation in T1-021 and T1-023.
pub trait ContentKind: DeserializeOwned + Send + Sync + 'static {
    /// Folder under the mod's `content_root`, e.g. `"units"`.
    const DIR: &'static str;
    fn id(&self) -> &ContentId;
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
        }
    }

    /// Infallible: handles are only ever produced by this registry.
    #[inline]
    pub fn get(&self, h: Handle<T>) -> &T {
        &self.items[h.index() as usize]
    }

    pub fn lookup(&self, id: &ContentId) -> Option<Handle<T>> {
        self.by_id.get(id).map(|&i| Handle::from_index(i))
    }

    pub fn id_of(&self, h: Handle<T>) -> &ContentId {
        &self.ids[h.index() as usize]
    }

    /// Ascending index order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, item)| (Handle::from_index(i as u32), item))
    }

    pub fn ids(&self) -> impl Iterator<Item = &ContentId> {
        self.ids.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, id: &ContentId) -> bool {
        self.by_id.contains_key(id)
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
}
