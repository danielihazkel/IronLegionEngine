//! Slot layout functions (T1-040; SIM-FORM-001..011, TDD §7).
//!
//! A layout turns `(template, n, ranks, soldier radius)` into `n` slot
//! offsets in the formation's local frame (x right, y forward, metres); the
//! anchor is the centre of the front rank (the apex for a wedge), which sits
//! at `y = 0` and every other rank behind it at negative `y`. Everything is
//! computed from integers through `S`, so the tables are bit-stable.

use il_core::{Angle, S, Scalar, V2};
use il_data::{FormationTemplate, Layout, UnitCategory};

/// One formation slot (TDD §7 `Slot`; `file` is `u16` per the Phase 1 plan
/// so a 2,000-man line fits).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// Offset from the anchor in the local frame.
    pub offset: V2,
    /// Slot facing relative to the anchor facing (non-zero only for the
    /// outward-facing sides of a square, SIM-FORM-010).
    pub facing_offset: Angle<S>,
    /// Row from the front, `0` = front rank.
    pub rank: u8,
    /// Position within the rank, `0` = leftmost.
    pub file: u16,
    /// Category the slot is reserved for in a mixed regiment (SIM-FORM-011;
    /// data only until Phase 3).
    pub category: Option<UnitCategory>,
}

/// File and rank spacing in metres: template multipliers of the soldier
/// diameter (SIM-FORM-002).
pub fn spacing(t: &FormationTemplate, radius: S) -> (S, S) {
    let diameter = radius + radius;
    (t.spacing_file * diameter, t.spacing_rank * diameter)
}

/// `ceil(n / ranks)` (SIM-FORM-002).
pub fn files_for(n: u16, ranks: u8) -> u16 {
    let ranks = u16::from(ranks.max(1));
    n.div_ceil(ranks).max(1)
}

/// The rank count a regiment of `n` uses: the request (or the template
/// default) clamped to `[min_ranks, max_ranks]`, never more ranks than
/// soldiers, and never so few that a rank exceeds `u16::MAX` files.
pub fn effective_ranks(t: &FormationTemplate, n: u16, requested: Option<u8>) -> u8 {
    let lo = t.min_ranks.max(1);
    let hi = t.max_ranks.max(lo);
    let wanted = requested.unwrap_or(t.default_ranks).clamp(lo, hi);
    let by_count = u8::try_from(n.max(1)).unwrap_or(u8::MAX);
    wanted.min(by_count).max(1)
}

/// Category reserved for `rank` (0-based) by the template's role zones.
fn category_for(t: &FormationTemplate, rank: u8) -> Option<UnitCategory> {
    let one_based = u16::from(rank) + 1;
    t.role_zones
        .iter()
        .find(|z| u16::from(z.ranks_from) <= one_based && one_based <= u16::from(z.ranks_to))
        .map(|z| z.unit_category)
}

/// `(f - (m - 1) / 2) * sf` without float literals: files are centred on
/// the axis.
fn centred(f: u16, m: u16, sf: S) -> S {
    S::from_i32(2 * i32::from(f) - (i32::from(m) - 1)) * sf * S::HALF
}

/// Fills `out` with `n` slots in `files` columns, rank by rank, each rank
/// centred on the axis (the last one may be short), starting at `rank0`
/// and offset by `origin`.
#[allow(clippy::too_many_arguments)]
fn line_slots(
    t: &FormationTemplate,
    n: u16,
    files: u16,
    sf: S,
    sr: S,
    origin: V2,
    rank0: u8,
    out: &mut Vec<Slot>,
) {
    let files = files.max(1);
    let mut placed: u16 = 0;
    let mut q: u16 = 0;
    while placed < n {
        let m = (n - placed).min(files);
        let rank = u8::try_from(u16::from(rank0) + q).unwrap_or(u8::MAX);
        for f in 0..m {
            out.push(Slot {
                offset: origin + V2::new(centred(f, m, sf), -S::from_i32(i32::from(q)) * sr),
                facing_offset: Angle::ZERO,
                rank,
                file: f,
                category: category_for(t, rank),
            });
        }
        placed += m;
        q += 1;
    }
}

/// A layout function (TDD §7 `LayoutFn`).
pub trait LayoutFn: Sync {
    /// Appends exactly `n` slots to `out` (cleared first).
    fn layout(&self, t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>);
}

struct LineLayout;
struct ColumnLayout;
struct SquareLayout;
struct WedgeLayout;
struct PhalanxLayout;
struct LooseLayout;
struct CustomLayout;

/// The layout function of a template's `layout` (SIM-FORM-003..009).
pub fn layout_for(layout: Layout) -> &'static dyn LayoutFn {
    match layout {
        Layout::Line => &LineLayout,
        Layout::Column => &ColumnLayout,
        Layout::Square => &SquareLayout,
        Layout::Wedge => &WedgeLayout,
        Layout::Phalanx => &PhalanxLayout,
        Layout::Loose => &LooseLayout,
        Layout::Custom => &CustomLayout,
    }
}

/// Convenience: lays out `n` slots for `t` with the given ranks and radius.
pub fn layout_slots(t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>) {
    layout_for(t.layout).layout(t, n, ranks, radius, out);
}

/// The rank count a layout actually produces (`ranks` for line-likes, the
/// derived value for column and wedge, the side depth for a square).
pub fn ranks_used(slots: &[Slot]) -> u8 {
    slots.iter().map(|s| s.rank).max().map_or(0, |r| r + 1)
}

/// The width in files of the widest rank of `slots`.
pub fn files_used(slots: &[Slot]) -> u16 {
    slots.iter().map(|s| s.file).max().map_or(0, |f| f + 1)
}

impl LayoutFn for LineLayout {
    /// SIM-FORM-003.
    fn layout(&self, t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>) {
        out.clear();
        let (sf, sr) = spacing(t, radius);
        line_slots(t, n, files_for(n, ranks), sf, sr, V2::ZERO, 0, out);
    }
}

impl LayoutFn for PhalanxLayout {
    /// SIM-FORM-007: a line with the template's tighter spacing; `min_ranks`
    /// is enforced by `effective_ranks`.
    fn layout(&self, t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>) {
        LineLayout.layout(t, n, ranks, radius, out);
    }
}

impl LayoutFn for LooseLayout {
    /// SIM-FORM-008: a line with spacing × `loose_mult`.
    fn layout(&self, t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>) {
        out.clear();
        let (sf, sr) = spacing(t, radius);
        line_slots(
            t,
            n,
            files_for(n, ranks),
            sf * t.loose_mult,
            sr * t.loose_mult,
            V2::ZERO,
            0,
            out,
        );
    }
}

impl LayoutFn for ColumnLayout {
    /// SIM-FORM-004: `default_files_column` files, ranks derived (widened
    /// only if the column would exceed 255 ranks).
    fn layout(&self, t: &FormationTemplate, n: u16, _ranks: u8, radius: S, out: &mut Vec<Slot>) {
        out.clear();
        let (sf, sr) = spacing(t, radius);
        let min_files = n.div_ceil(u16::from(u8::MAX));
        let files = u16::from(t.default_files_column.max(1)).max(min_files);
        line_slots(t, n, files, sf, sr, V2::ZERO, 0, out);
    }
}

impl LayoutFn for WedgeLayout {
    /// SIM-FORM-006: rank `q` has `2q + 1` slots centred on the axis; the
    /// anchor is the apex; the last rank is centred.
    fn layout(&self, t: &FormationTemplate, n: u16, _ranks: u8, radius: S, out: &mut Vec<Slot>) {
        out.clear();
        let (sf, sr) = spacing(t, radius);
        let mut placed: u16 = 0;
        let mut q: u16 = 0;
        while placed < n {
            let width = 2 * q + 1;
            let m = (n - placed).min(width);
            let rank = u8::try_from(q).unwrap_or(u8::MAX);
            for f in 0..m {
                out.push(Slot {
                    offset: V2::new(centred(f, m, sf), -S::from_i32(i32::from(q)) * sr),
                    facing_offset: Angle::ZERO,
                    rank,
                    file: f,
                    category: category_for(t, rank),
                });
            }
            placed += m;
            q += 1;
        }
    }
}

impl LayoutFn for SquareLayout {
    /// SIM-FORM-005 as amended (Phase 1 plan S9): four outward-facing sides
    /// of `n / 4` soldiers each (the remainder joins the rear side), each
    /// side `ranks` deep with a corner band of `ranks × sr` between sides.
    /// The front side is the front rank at `y = 0`; the square extends
    /// `side_len` behind the anchor. Facing offsets: front 0, right −90°,
    /// rear 180°, left +90°.
    fn layout(&self, t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>) {
        out.clear();
        if n == 0 {
            return;
        }
        let (sf, sr) = spacing(t, radius);
        let depth = u16::from(ranks.max(1)).min(n.div_ceil(4).max(1));
        let base = n / 4;
        let rem = n % 4;
        let counts = [base, base, base + rem, base];
        // Side length: the longest side's files at `sf` plus a corner band of
        // `depth × sr` at each end, so the inset rows of neighbouring sides
        // never meet.
        let longest = counts.iter().copied().max().unwrap_or(0);
        let files_per_row = longest.div_ceil(depth).max(1);
        let band = S::from_i32(i32::from(depth)) * sr;
        let side_len = S::from_i32(i32::from(files_per_row)) * sf + band + band;
        let half = side_len * S::HALF;
        let quarter = S::PI * S::HALF;
        // (direction along the side, inward normal, facing offset) per side:
        // front runs +x at y = 0; right runs −y at x = +half; rear runs −x
        // at y = −side_len; left runs +y at x = −half.
        let sides: [(V2, V2, V2, S); 4] = [
            (
                V2::new(S::ZERO, S::ZERO),
                V2::new(S::ONE, S::ZERO),
                V2::new(S::ZERO, -S::ONE),
                S::ZERO,
            ),
            (
                V2::new(half, -half),
                V2::new(S::ZERO, -S::ONE),
                V2::new(-S::ONE, S::ZERO),
                -quarter,
            ),
            (
                V2::new(S::ZERO, -side_len),
                V2::new(-S::ONE, S::ZERO),
                V2::new(S::ZERO, S::ONE),
                S::PI,
            ),
            (
                V2::new(-half, -half),
                V2::new(S::ZERO, S::ONE),
                V2::new(S::ONE, S::ZERO),
                quarter,
            ),
        ];
        let mut file_base: u16 = 0;
        for (side, &(centre, along, inward, facing)) in sides.iter().enumerate() {
            let k = counts[side];
            if k == 0 {
                continue;
            }
            let per_row = k.div_ceil(depth).max(1);
            let mut placed = 0;
            let mut row: u16 = 0;
            while placed < k {
                let m = (k - placed).min(per_row);
                let inset = inward * (S::from_i32(i32::from(row)) * sr);
                for f in 0..m {
                    let offset = centre + along * centred(f, m, sf) + inset;
                    out.push(Slot {
                        offset,
                        facing_offset: Angle::new(facing),
                        rank: u8::try_from(row).unwrap_or(u8::MAX),
                        file: file_base + f,
                        category: category_for(t, u8::try_from(row).unwrap_or(u8::MAX)),
                    });
                }
                placed += m;
                row += 1;
            }
            file_base += per_row;
        }
    }
}

impl LayoutFn for CustomLayout {
    /// SIM-FORM-009: `custom_slots` in soldier diameters (x right, y
    /// forward); soldiers beyond the list form a line one rank behind the
    /// lowest custom slot.
    fn layout(&self, t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>) {
        out.clear();
        let (sf, sr) = spacing(t, radius);
        let diameter = radius + radius;
        let custom = usize::from(n).min(t.custom_slots.len());
        let mut y_min = S::ZERO;
        for (i, o) in t.custom_slots.iter().take(custom).enumerate() {
            let offset = *o * diameter;
            y_min = y_min.min(offset.y);
            out.push(Slot {
                offset,
                facing_offset: Angle::ZERO,
                rank: 0,
                file: i as u16,
                category: category_for(t, 0),
            });
        }
        let extra = n - custom as u16;
        if extra > 0 {
            let origin = V2::new(S::ZERO, y_min - sr);
            line_slots(t, extra, files_for(extra, ranks), sf, sr, origin, 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_data::ContentId;

    fn template(layout: Layout) -> FormationTemplate {
        let json = format!(
            r#"{{ "id": "t:x", "name_key": "t.x.name", "layout": "{}", "default_ranks": 4, "min_ranks": 1,
                 "max_ranks": 16, "spacing_file": 1.0, "spacing_rank": 1.25, "custom_slots": [
                 {{ "x": 0, "y": 0 }}, {{ "x": -2, "y": -1 }}, {{ "x": 2, "y": -1 }} ] }}"#,
            match layout {
                Layout::Line => "line",
                Layout::Column => "column",
                Layout::Square => "square",
                Layout::Wedge => "wedge",
                Layout::Phalanx => "phalanx",
                Layout::Loose => "loose",
                Layout::Custom => "custom",
            }
        );
        let t: FormationTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(t.id, ContentId::new("t:x").unwrap());
        t
    }

    const ALL: [Layout; 7] = [
        Layout::Line,
        Layout::Column,
        Layout::Square,
        Layout::Wedge,
        Layout::Phalanx,
        Layout::Loose,
        Layout::Custom,
    ];

    #[test]
    fn every_layout_places_n_distinct_slots_with_a_centred_front_rank() {
        let radius = S::from_f32_data(0.4);
        for layout in ALL {
            let t = template(layout);
            for n in [1u16, 2, 7, 60, 160, 500] {
                let ranks = effective_ranks(&t, n, None);
                let mut slots = Vec::new();
                layout_slots(&t, n, ranks, radius, &mut slots);
                assert_eq!(slots.len(), usize::from(n), "{layout:?} n={n}");
                for i in 0..slots.len() {
                    for j in i + 1..slots.len() {
                        assert!(
                            slots[i].offset.distance_sq(slots[j].offset) > S::from_f32_data(1e-6),
                            "{layout:?} n={n}: slots {i} and {j} coincide"
                        );
                    }
                }
                // Front rank centred on the axis and at y = 0 (a square's
                // front side, the wedge apex and the first custom slot too).
                let front: Vec<&Slot> = slots.iter().filter(|s| s.rank == 0).collect();
                assert!(!front.is_empty());
                let mut sum = S::ZERO;
                let mut n_front = 0;
                for s in &front {
                    if s.facing_offset == Angle::ZERO {
                        sum = sum + s.offset.x;
                        n_front += 1;
                    }
                }
                // A square below four soldiers has only a rear side.
                if layout != Layout::Custom && n_front > 0 {
                    assert!(
                        (sum / S::from_i32(n_front)).abs() < S::from_f32_data(1e-4),
                        "{layout:?} n={n}: front rank off centre by {sum:?}"
                    );
                    assert!(front.iter().all(|s| s.offset.y <= S::ZERO));
                    assert!(slots.iter().all(|s| s.offset.y <= S::from_f32_data(1e-6)));
                }
                // Ranks and files are consistent.
                let mut ids: Vec<(u8, u16, u8)> = slots
                    .iter()
                    .map(|s| (s.rank, s.file, s.facing_offset.to_facing8()))
                    .collect();
                ids.sort_unstable();
                ids.dedup();
                assert_eq!(
                    ids.len(),
                    slots.len(),
                    "{layout:?} n={n}: duplicate (rank, file)"
                );
            }
        }
    }

    #[test]
    fn line_geometry_matches_sim_form_003() {
        let t = template(Layout::Line);
        let radius = S::from_f32_data(0.5); // diameter 1 → sf 1.0, sr 1.25
        let mut slots = Vec::new();
        layout_slots(&t, 7, 3, radius, &mut slots);
        // files = ceil(7/3) = 3: ranks of 3, 3, 1 (last centred).
        assert_eq!(files_used(&slots), 3);
        assert_eq!(ranks_used(&slots), 3);
        assert_eq!(slots[0].offset, V2::new(-S::ONE, S::ZERO));
        assert_eq!(slots[1].offset, V2::new(S::ZERO, S::ZERO));
        assert_eq!(slots[2].offset, V2::new(S::ONE, S::ZERO));
        assert_eq!(slots[3].offset, V2::new(-S::ONE, -S::from_f32_data(1.25)));
        assert_eq!(slots[6].offset, V2::new(S::ZERO, -S::from_f32_data(2.5)));
        assert_eq!((slots[6].rank, slots[6].file), (2, 0));
        // Column: 4 files, ranks derived.
        let mut col = Vec::new();
        layout_slots(&template(Layout::Column), 10, 4, radius, &mut col);
        assert_eq!(files_used(&col), 4);
        assert_eq!(ranks_used(&col), 3);
        // Loose doubles the spacing.
        let mut loose = Vec::new();
        layout_slots(&template(Layout::Loose), 3, 1, radius, &mut loose);
        assert_eq!(loose[2].offset.x, S::from_i32(2));
        // Wedge: 1, 3, 5, ... slots per rank from the apex.
        let mut wedge = Vec::new();
        layout_slots(&template(Layout::Wedge), 9, 1, radius, &mut wedge);
        assert_eq!(wedge[0].offset, V2::ZERO);
        assert_eq!(wedge.iter().filter(|s| s.rank == 1).count(), 3);
        assert_eq!(wedge.iter().filter(|s| s.rank == 2).count(), 5);
        // Square: 8 soldiers, one deep: two per side, outward facings.
        let mut sq = Vec::new();
        layout_slots(&template(Layout::Square), 8, 1, radius, &mut sq);
        let facings: Vec<u8> = sq.iter().map(|s| s.facing_offset.to_facing8()).collect();
        assert_eq!(facings, vec![0, 0, 6, 6, 4, 4, 2, 2]);
        // Side length = 2 files × 1.0 + 2 × 1.25 = 4.5.
        assert!(
            sq.iter().all(|s| s.offset.x.abs() <= S::from_f32_data(2.25)
                && s.offset.y >= -S::from_f32_data(4.5))
        );
        assert_eq!(sq[0].offset, V2::new(-S::HALF, S::ZERO));
        assert_eq!(
            sq[2].offset,
            V2::new(S::from_f32_data(2.25), -S::from_f32_data(1.75))
        );
        // Custom: three listed slots then a line behind.
        let mut custom = Vec::new();
        layout_slots(&template(Layout::Custom), 5, 2, radius, &mut custom);
        assert_eq!(custom[1].offset, V2::new(-S::from_i32(2), -S::ONE));
        assert_eq!(custom[3].rank, 1);
        assert!(custom[3].offset.y < -S::ONE);
        assert_eq!(effective_ranks(&t, 2, Some(40)), 2);
        assert_eq!(effective_ranks(&t, 500, Some(40)), 16);
        assert_eq!(effective_ranks(&t, 500, None), 4);
        assert_eq!(files_for(7, 3), 3);
        assert_eq!(files_for(1, 4), 1);
    }
}
