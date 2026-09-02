//! Event base types (TDD §2.2 `events.rs`, SAD §3 principle 2).
//!
//! Events are the sim's only output. They are derived from state during a
//! tick, pushed in a fixed system order, and drained at Stage 17. They are
//! never an input and never snapshotted.

use serde::Serialize;

use crate::time::Tick;

/// Marker for event payload types.
pub trait Event: Serialize + Clone + core::fmt::Debug {}

/// An ordered per-tick event buffer.
#[derive(Clone, Debug)]
pub struct EventQueue<E: Event> {
    items: Vec<(Tick, E)>,
}

impl<E: Event> Default for EventQueue<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Event> EventQueue<E> {
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Appends an event; order of insertion is preserved.
    #[inline]
    pub fn push(&mut self, tick: Tick, event: E) {
        self.items.push((tick, event));
    }

    /// Removes and returns every queued event in insertion order.
    pub fn drain(&mut self) -> Vec<(Tick, E)> {
        core::mem::take(&mut self.items)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &(Tick, E)> {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize)]
    enum Ev {
        A(u32),
        B,
    }
    impl Event for Ev {}

    #[test]
    fn preserves_insertion_order_and_drains() {
        let mut q = EventQueue::new();
        assert!(q.is_empty());
        q.push(Tick(1), Ev::B);
        q.push(Tick(1), Ev::A(2));
        q.push(Tick(2), Ev::A(1));
        assert_eq!(q.len(), 3);
        let drained = q.drain();
        assert_eq!(
            drained,
            vec![(Tick(1), Ev::B), (Tick(1), Ev::A(2)), (Tick(2), Ev::A(1))]
        );
        assert!(q.is_empty());
        assert!(q.drain().is_empty());
    }
}
