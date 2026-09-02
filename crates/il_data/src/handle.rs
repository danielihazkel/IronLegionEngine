//! `Handle<T>`: a typed index into a [`Registry`](crate::Registry) (TDD §3.2,
//! SAD §7). The ECS stores handles, never strings. Handles are not
//! serialised; snapshots store `ContentId`s and re-resolve on restore.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use il_core::hash::{Hashable, StateHasher};

pub struct Handle<T> {
    index: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    pub(crate) const fn from_index(index: u32) -> Self {
        Self {
            index,
            _marker: PhantomData,
        }
    }

    /// Position in the registry; ascending index is the deterministic order.
    pub const fn index(self) -> u32 {
        self.index
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, o: &Self) -> bool {
        self.index == o.index
    }
}
impl<T> Eq for Handle<T> {}

impl<T> PartialOrd for Handle<T> {
    fn partial_cmp(&self, o: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl<T> Ord for Handle<T> {
    fn cmp(&self, o: &Self) -> core::cmp::Ordering {
        self.index.cmp(&o.index)
    }
}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Handle<{}>({})",
            core::any::type_name::<T>()
                .rsplit("::")
                .next()
                .unwrap_or("?"),
            self.index
        )
    }
}

impl<T> Hashable for Handle<T> {
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u32(self.index);
    }
}
