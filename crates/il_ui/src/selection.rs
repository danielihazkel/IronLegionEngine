//! The selection model (T1-061, REQ-INP-002, TDD §11): the selected
//! regiments and ten control groups. Pure data; ownership and visibility are
//! decided by the caller (see [`crate::pick`]), which only ever hands in
//! regiments the local player may command.

use std::collections::BTreeSet;

use il_core::RegimentId;

/// Number of control groups (Ctrl+0..9).
pub const GROUPS: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub regiments: BTreeSet<RegimentId>,
    pub groups: [BTreeSet<RegimentId>; GROUPS],
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.regiments.is_empty()
    }

    pub fn len(&self) -> usize {
        self.regiments.len()
    }

    /// Whether `id` is selected.
    pub fn contains(&self, id: RegimentId) -> bool {
        self.regiments.contains(&id)
    }

    /// Plain click: the hit regiment alone, or nothing when the click missed.
    /// With `add` (shift-click) the hit toggles and a miss changes nothing.
    pub fn click(&mut self, hit: Option<RegimentId>, add: bool) {
        match (hit, add) {
            (Some(id), true) => {
                if !self.regiments.remove(&id) {
                    self.regiments.insert(id);
                }
            }
            (None, true) => {}
            (Some(id), false) => {
                self.regiments.clear();
                self.regiments.insert(id);
            }
            (None, false) => self.regiments.clear(),
        }
    }

    /// Box select: the hits replace the selection, or join it with `add`.
    /// An empty box without `add` clears the selection.
    pub fn box_select(&mut self, hits: impl IntoIterator<Item = RegimentId>, add: bool) {
        if !add {
            self.regiments.clear();
        }
        self.regiments.extend(hits);
    }

    /// Replaces the selection (double-click by type, select all).
    pub fn set(&mut self, ids: impl IntoIterator<Item = RegimentId>) {
        self.regiments.clear();
        self.regiments.extend(ids);
    }

    /// Ctrl+n: stores the current selection as group `n`.
    pub fn set_group(&mut self, n: usize) {
        if let Some(g) = self.groups.get_mut(n) {
            *g = self.regiments.clone();
        }
    }

    /// n: recalls group `n` (joined to the selection with `add`); an empty
    /// group leaves the selection alone.
    pub fn recall_group(&mut self, n: usize, add: bool) {
        let Some(g) = self.groups.get(n) else {
            return;
        };
        if g.is_empty() {
            return;
        }
        if !add {
            self.regiments.clear();
        }
        self.regiments.extend(g.iter().copied());
    }

    /// Drops regiments that no longer qualify (destroyed, transferred),
    /// from the selection and every group.
    pub fn retain(&mut self, mut keep: impl FnMut(RegimentId) -> bool) {
        self.regiments.retain(|id| keep(*id));
        for g in &mut self.groups {
            g.retain(|id| keep(*id));
        }
    }

    pub fn clear(&mut self) {
        self.regiments.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: u32) -> RegimentId {
        RegimentId(n)
    }

    fn ids(s: &Selection) -> Vec<u32> {
        s.regiments.iter().map(|id| id.0).collect()
    }

    #[test]
    fn click_replaces_shift_click_toggles_miss_clears() {
        let mut s = Selection::new();
        s.click(Some(r(1)), false);
        s.click(Some(r(2)), false);
        assert_eq!(ids(&s), [2]);
        s.click(Some(r(1)), true);
        assert_eq!(ids(&s), [1, 2]);
        s.click(Some(r(2)), true);
        assert_eq!(ids(&s), [1]);
        s.click(None, true);
        assert_eq!(ids(&s), [1]);
        s.click(None, false);
        assert!(s.is_empty());
    }

    #[test]
    fn box_select_replaces_or_adds() {
        let mut s = Selection::new();
        s.click(Some(r(9)), false);
        s.box_select([r(1), r(2)], false);
        assert_eq!(ids(&s), [1, 2]);
        s.box_select([r(3)], true);
        assert_eq!(ids(&s), [1, 2, 3]);
        s.box_select([], false);
        assert!(s.is_empty());
    }

    #[test]
    fn control_groups_store_and_recall() {
        let mut s = Selection::new();
        s.set([r(1), r(2)]);
        s.set_group(3);
        s.set([r(5)]);
        s.recall_group(3, false);
        assert_eq!(ids(&s), [1, 2]);
        s.set([r(5)]);
        s.recall_group(3, true);
        assert_eq!(ids(&s), [1, 2, 5]);
        s.recall_group(7, false);
        assert_eq!(ids(&s), [1, 2, 5], "an empty group is a no-op");
        s.set_group(42);
        s.recall_group(42, false);
        assert_eq!(ids(&s), [1, 2, 5], "out-of-range groups are ignored");
    }

    #[test]
    fn retain_prunes_selection_and_groups() {
        let mut s = Selection::new();
        s.set([r(1), r(2), r(3)]);
        s.set_group(0);
        s.retain(|id| id.0 != 2);
        assert_eq!(ids(&s), [1, 3]);
        assert_eq!(s.groups[0].len(), 2);
    }
}
