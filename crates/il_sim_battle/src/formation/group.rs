//! Group formations (T1-046; SIM-FORM-040..042, TDD §7 `arrange_group`,
//! Phase 1 plan S13).
//!
//! `arrange_group` turns a group template, the regiments to arrange, and
//! the group anchor, facing and requested width into one anchor, facing and
//! rank count per regiment. Regiments keep their lateral order (projected
//! onto the group's right axis) so they never cross; cavalry may take the
//! flanks and skirmishers may stand ahead of the line.

use il_core::{Angle, RegimentId, S, Scalar, V2};
use il_data::{
    FormationRules, FormationTemplate, GroupFormationTemplate, GroupKind, Handle, Registries,
    UnitCategory,
};

use crate::formation::layout::{effective_ranks, files_for};
use crate::movement::regiment::formation_width;

/// What `arrange_group` needs to know about one regiment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegimentInfo {
    pub id: RegimentId,
    /// Current anchor, for the lateral ordering.
    pub pos: V2,
    pub category: UnitCategory,
    pub count: u16,
    pub template: Handle<FormationTemplate>,
    pub radius: S,
}

/// One regiment's place in the arrangement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub id: RegimentId,
    pub anchor: V2,
    pub facing: Angle<S>,
    pub ranks: u8,
}

fn is_cavalry(c: UnitCategory) -> bool {
    matches!(c, UnitCategory::Cavalry)
}

fn is_skirmisher(c: UnitCategory) -> bool {
    matches!(c, UnitCategory::Ranged | UnitCategory::Skirmisher)
}

/// SIM-FORM-040: indices of `regiments` from left to right along the
/// group's right axis (ties by id), cavalry moved to the outer positions
/// when the template asks for it (SIM-FORM-041).
pub fn lateral_order(regiments: &[RegimentInfo], right: V2, cavalry_flanks: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..regiments.len()).collect();
    order.sort_by(|&a, &b| {
        let ka = regiments[a].pos.dot(right);
        let kb = regiments[b].pos.dot(right);
        ka.partial_cmp(&kb)
            .expect("finite positions")
            .then(regiments[a].id.cmp(&regiments[b].id))
    });
    if !cavalry_flanks {
        return order;
    }
    let (cav, rest): (Vec<usize>, Vec<usize>) = order
        .iter()
        .partition(|&&i| is_cavalry(regiments[i].category));
    // Alternate cavalry onto the left and right flanks, keeping order.
    let half = cav.len().div_ceil(2);
    let mut out = Vec::with_capacity(order.len());
    out.extend_from_slice(&cav[..half]);
    out.extend_from_slice(&rest);
    out.extend_from_slice(&cav[half..]);
    out
}

/// Width in metres of a regiment laid out `ranks` deep.
fn regiment_width(r: &RegimentInfo, regs: &Registries, ranks: u8) -> S {
    let t = regs.formations.get(r.template);
    formation_width(t, files_for(r.count, ranks), r.radius)
}

/// SIM-FORM-042 (plan S13): every regiment starts at its fewest ranks and
/// the widest one gains a rank until the line fits `width` within
/// `width_tolerance` (or nobody can deepen further). Returns the ranks and
/// widths per regiment in `line` order and the total width.
fn choose_ranks(
    line: &[usize],
    regiments: &[RegimentInfo],
    regs: &Registries,
    rules: &FormationRules,
    gap: S,
    width: S,
) -> (Vec<u8>, Vec<S>, S) {
    let mut ranks: Vec<u8> = line
        .iter()
        .map(|&i| {
            effective_ranks(
                regs.formations.get(regiments[i].template),
                regiments[i].count,
                Some(1),
            )
        })
        .collect();
    let mut widths: Vec<S> = line
        .iter()
        .zip(&ranks)
        .map(|(&i, &r)| regiment_width(&regiments[i], regs, r))
        .collect();
    let gaps = gap * S::from_i32(line.len().saturating_sub(1) as i32);
    let limit = width * (S::ONE + rules.width_tolerance);
    loop {
        let total = widths.iter().fold(gaps, |a, w| a + *w);
        if total <= limit {
            return (ranks, widths, total);
        }
        // The widest regiment that can still deepen; ties by line position.
        let mut best: Option<usize> = None;
        for (k, &i) in line.iter().enumerate() {
            let t = regs.formations.get(regiments[i].template);
            if ranks[k] >= effective_ranks(t, regiments[i].count, Some(u8::MAX)) {
                continue;
            }
            if best.is_none_or(|b| widths[k] > widths[b]) {
                best = Some(k);
            }
        }
        let Some(k) = best else {
            return (ranks, widths, total);
        };
        ranks[k] += 1;
        widths[k] = regiment_width(&regiments[line[k]], regs, ranks[k]);
    }
}

/// Places `line` side by side along `right`, centred on `centre`, with
/// `gap` between neighbours and each regiment's own forward offset.
#[allow(clippy::too_many_arguments)]
fn place_line(
    line: &[usize],
    regiments: &[RegimentInfo],
    ranks: &[u8],
    widths: &[S],
    centre: V2,
    right: V2,
    forward: V2,
    facing: Angle<S>,
    gap: S,
    total: S,
    forward_offset: impl Fn(usize, &RegimentInfo) -> S,
    facing_offset: impl Fn(usize) -> S,
    out: &mut Vec<Placement>,
) {
    let mut x = -total * S::HALF;
    for (k, &i) in line.iter().enumerate() {
        let r = &regiments[i];
        let lateral = x + widths[k] * S::HALF;
        out.push(Placement {
            id: r.id,
            anchor: centre + right * lateral + forward * forward_offset(k, r),
            facing: Angle::new(facing.radians() + facing_offset(k)),
            ranks: ranks[k],
        });
        x = x + widths[k] + gap;
    }
}

/// SIM-FORM-042 for a single regiment (the drag-formation gesture on one
/// regiment, TDD §11): the fewest ranks in `[min_ranks, max_ranks]` whose
/// width fits `width × (1 + tolerance)`, deepening one rank at a time; the
/// deepest allowed when nothing fits.
pub fn ranks_for_width(t: &FormationTemplate, count: u16, radius: S, width: S, tolerance: S) -> u8 {
    let deepest = effective_ranks(t, count, Some(u8::MAX));
    let limit = width * (S::ONE + tolerance);
    let mut ranks = effective_ranks(t, count, Some(1));
    while ranks < deepest && formation_width(t, files_for(count, ranks), radius) > limit {
        ranks += 1;
    }
    ranks
}

/// SIM-FORM-040..042: anchor, facing and ranks per regiment for `t`.
pub fn arrange_group(
    t: &GroupFormationTemplate,
    regiments: &[RegimentInfo],
    anchor: V2,
    facing: Angle<S>,
    width: S,
    rules: &FormationRules,
    regs: &Registries,
) -> Vec<Placement> {
    let mut out = Vec::with_capacity(regiments.len());
    if regiments.is_empty() {
        return out;
    }
    let forward = facing.direction();
    let right = V2::new(forward.y, -forward.x);
    let gap = if t.gap > S::ZERO {
        t.gap
    } else {
        rules.group_gap
    };
    let order = lateral_order(regiments, right, t.cavalry_flanks);
    let skirmish = |_: usize, r: &RegimentInfo| {
        if t.skirmishers_forward && is_skirmisher(r.category) {
            rules.skirmish_offset
        } else {
            S::ZERO
        }
    };
    let two_gaps = gap + gap;

    match t.kind {
        GroupKind::DoubleLine if regiments.len() > 1 => {
            let lines = usize::from(t.lines.max(2));
            for l in 0..lines {
                let line: Vec<usize> = order.iter().copied().skip(l).step_by(lines).collect();
                if line.is_empty() {
                    continue;
                }
                let (ranks, widths, total) =
                    choose_ranks(&line, regiments, regs, rules, gap, width);
                let back = forward * (two_gaps * S::from_i32(l as i32));
                place_line(
                    &line,
                    regiments,
                    &ranks,
                    &widths,
                    anchor - back,
                    right,
                    forward,
                    facing,
                    gap,
                    total,
                    skirmish,
                    |_| S::ZERO,
                    &mut out,
                );
            }
        }
        GroupKind::EchelonLeft | GroupKind::EchelonRight => {
            let (ranks, widths, total) = choose_ranks(&order, regiments, regs, rules, gap, width);
            let n = order.len();
            let step_back = |k: usize| {
                // Successive regiments toward the named flank fall back.
                let steps = if t.kind == GroupKind::EchelonLeft {
                    n - 1 - k
                } else {
                    k
                };
                -two_gaps * S::from_i32(steps as i32)
            };
            place_line(
                &order,
                regiments,
                &ranks,
                &widths,
                anchor,
                right,
                forward,
                facing,
                gap,
                total,
                |k, r| skirmish(k, r) + step_back(k),
                |_| S::ZERO,
                &mut out,
            );
        }
        GroupKind::RefusedLeft | GroupKind::RefusedRight => {
            let (ranks, widths, total) = choose_ranks(&order, regiments, regs, rules, gap, width);
            let n = order.len();
            let refused = if t.kind == GroupKind::RefusedLeft {
                0
            } else {
                n - 1
            };
            let three_gaps = gap + gap + gap;
            let quarter_eighth = S::PI / S::from_i32(4);
            // The left flank turns clockwise (inward), the right flank
            // counter-clockwise.
            let turn = if t.kind == GroupKind::RefusedLeft {
                -quarter_eighth
            } else {
                quarter_eighth
            };
            place_line(
                &order,
                regiments,
                &ranks,
                &widths,
                anchor,
                right,
                forward,
                facing,
                gap,
                total,
                |k, r| {
                    skirmish(k, r)
                        + if k == refused && n > 1 {
                            -three_gaps
                        } else {
                            S::ZERO
                        }
                },
                |k| if k == refused && n > 1 { turn } else { S::ZERO },
                &mut out,
            );
        }
        // Battle line, single-regiment double lines and `custom` (no
        // geometry of its own in Phase 1).
        _ => {
            let (ranks, widths, total) = choose_ranks(&order, regiments, regs, rules, gap, width);
            place_line(
                &order,
                regiments,
                &ranks,
                &widths,
                anchor,
                right,
                forward,
                facing,
                gap,
                total,
                skirmish,
                |_| S::ZERO,
                &mut out,
            );
        }
    }
    // Ascending id, so callers apply placements deterministically.
    out.sort_by_key(|p| p.id);
    out
}

/// The lateral extent of a set of placements along `right` plus their
/// widths at the chosen ranks: the line's total width.
pub fn arranged_width(
    placements: &[Placement],
    regiments: &[RegimentInfo],
    regs: &Registries,
    right: V2,
) -> S {
    let mut lo = None;
    let mut hi = None;
    for p in placements {
        let Some(r) = regiments.iter().find(|r| r.id == p.id) else {
            continue;
        };
        let half = regiment_width(r, regs, p.ranks) * S::HALF;
        let x = p.anchor.dot(right);
        lo = Some(lo.map_or(x - half, |l: S| l.min(x - half)));
        hi = Some(hi.map_or(x + half, |h: S| h.max(x + half)));
    }
    match (lo, hi) {
        (Some(l), Some(h)) => h - l,
        _ => S::ZERO,
    }
}
