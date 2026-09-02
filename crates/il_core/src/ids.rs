//! Stable entity ids (TDD §2.2 `ids.rs`).
//!
//! Ids are monotonic within a battle or campaign and never reused
//! (SAD §9.1). Ordering by id is the only sanctioned iteration order for
//! order-dependent systems (SIM-DET-003).

use core::marker::PhantomData;

use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident($inner:ty)) => {
        $(#[$meta])*
        #[derive(
            Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub $inner);

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}#{}", stringify!($name), self.0)
            }
        }
    };
}

id_newtype!(
    /// A soldier within one battle.
    SoldierId(u32)
);
id_newtype!(
    /// A regiment within one battle.
    RegimentId(u32)
);
id_newtype!(
    /// An army (campaign-level, echoed into battles).
    ArmyId(u16)
);
id_newtype!(
    /// A faction.
    FactionId(u8)
);
id_newtype!(
    /// A player. `0..=7` are humans or AIs; `255` is the engine AI
    /// (SIM-CMD-003, Networking Spec §2).
    PlayerId(u8)
);
id_newtype!(
    /// A projectile within one battle.
    ProjectileId(u32)
);
id_newtype!(
    /// A campaign province.
    ProvinceId(u16)
);

impl PlayerId {
    /// The engine-internal AI player that owns regiments handed over by
    /// `TransferControl { to: 255 }` (SIM-CMD-002, SIM-CMD-003).
    pub const ENGINE_AI: PlayerId = PlayerId(255);
}

/// Ids that an [`IdAllocator`] can hand out: the `u32`-backed per-battle ids.
pub trait AllocatableId: Copy + Ord {
    fn from_raw(raw: u32) -> Self;
    fn raw(self) -> u32;
}

macro_rules! allocatable {
    ($($name:ident),*) => {$(
        impl AllocatableId for $name {
            #[inline]
            fn from_raw(raw: u32) -> Self { $name(raw) }
            #[inline]
            fn raw(self) -> u32 { self.0 }
        }
    )*};
}
allocatable!(SoldierId, RegimentId, ProjectileId);

/// Hands out ids in ascending order and never reuses one. Its counter is part
/// of the snapshot so restored worlds continue the same sequence (TDD §4.6).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdAllocator<T> {
    next: u32,
    #[serde(skip)]
    _marker: PhantomData<T>,
}

impl<T> Default for IdAllocator<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> IdAllocator<T> {
    /// An allocator whose first id is `0`.
    pub const fn new() -> Self {
        Self {
            next: 0,
            _marker: PhantomData,
        }
    }

    /// Rebuilds an allocator from a snapshotted counter.
    pub const fn from_next(next: u32) -> Self {
        Self {
            next,
            _marker: PhantomData,
        }
    }

    /// The raw value the next call to [`alloc`](Self::alloc) will return.
    pub const fn peek_raw(&self) -> u32 {
        self.next
    }

    /// How many ids have been handed out so far.
    pub const fn allocated(&self) -> u32 {
        self.next
    }
}

impl<T: AllocatableId> IdAllocator<T> {
    /// Returns the next id. Panics if the `u32` space is exhausted, which is
    /// impossible within the 32,768 soldier cap over any realistic battle.
    pub fn alloc(&mut self) -> T {
        let id = T::from_raw(self.next);
        self.next = self.next.checked_add(1).expect("id space exhausted");
        id
    }

    /// The next id without consuming it.
    pub fn peek(&self) -> T {
        T::from_raw(self.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_is_monotonic_and_never_reuses() {
        let mut a = IdAllocator::<SoldierId>::new();
        let ids: Vec<SoldierId> = (0..1000).map(|_| a.alloc()).collect();
        for w in ids.windows(2) {
            assert!(w[0] < w[1]);
        }
        assert_eq!(ids[0], SoldierId(0));
        assert_eq!(ids[999], SoldierId(999));
        assert_eq!(a.peek(), SoldierId(1000));
        assert_eq!(a.allocated(), 1000);
    }

    #[test]
    fn allocator_serde_round_trip_continues_sequence() {
        let mut a = IdAllocator::<RegimentId>::new();
        for _ in 0..7 {
            a.alloc();
        }
        let json = serde_json::to_string(&a).unwrap();
        let mut b: IdAllocator<RegimentId> = serde_json::from_str(&json).unwrap();
        assert_eq!(b.alloc(), RegimentId(7));
        assert_eq!(a.alloc(), RegimentId(7));
        assert_eq!(
            IdAllocator::<RegimentId>::from_next(42).alloc(),
            RegimentId(42)
        );
    }

    #[test]
    fn ids_serialise_transparently_and_order() {
        assert_eq!(serde_json::to_string(&SoldierId(5)).unwrap(), "5");
        assert_eq!(
            serde_json::from_str::<PlayerId>("255").unwrap(),
            PlayerId::ENGINE_AI
        );
        assert!(ProvinceId(1) < ProvinceId(2));
        assert_eq!(format!("{}", ArmyId(3)), "ArmyId#3");
    }
}
