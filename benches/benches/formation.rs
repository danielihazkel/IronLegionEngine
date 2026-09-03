//! T1-041 done-when: `assign_slots` for a 500-man regiment under 0.5 ms
//! (SIM-FORM-023), in the worst case where nobody keeps a slot.

use criterion::{Criterion, criterion_group, criterion_main};
use il_core::{Angle, S, Scalar, SoldierId, V2};
use il_data::{ContentId, Registries, UnitCategory};
use il_sim_battle::components::Anchor;
use il_sim_battle::{
    AssignScratch, AssignSoldier, assign_slots, effective_ranks, layout_slots, slot_world,
};

fn assign(c: &mut Criterion) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game");
    let regs = Registries::load_root(&root).unwrap_or_else(|d| panic!("{d}"));
    let line = regs.formations.get(
        regs.formations
            .lookup(&ContentId::new("rome:line").unwrap())
            .unwrap(),
    );
    let radius = S::from_f32_data(0.4);
    let n = 500u16;
    let ranks = effective_ranks(line, n, None);
    let mut slots = Vec::new();
    layout_slots(line, n, ranks, radius, &mut slots);
    // Soldiers stand on the slots of the old facing; the new anchor is
    // turned 90 degrees, so every keep test fails and the greedy pass and
    // the swap passes do all the work.
    let old = Anchor {
        pos: V2::new(S::from_i32(400), S::from_i32(300)),
        facing: Angle::ZERO,
    };
    let new = Anchor {
        pos: old.pos,
        facing: Angle::from_degrees_data(90.0),
    };
    let soldiers: Vec<AssignSoldier> = slots
        .iter()
        .enumerate()
        .map(|(i, s)| AssignSoldier {
            id: SoldierId(i as u32),
            pos: slot_world(&old, s),
            category: UnitCategory::Infantry,
        })
        .collect();
    let prev: Vec<Option<u16>> = (0..n).map(Some).collect();
    let mut out = Vec::new();
    let mut scratch = AssignScratch::default();
    c.bench_function("assign_slots_500_turned", |b| {
        b.iter(|| {
            assign_slots(
                &soldiers,
                &slots,
                &new,
                &regs.rules.formation,
                &prev,
                &mut out,
                &mut scratch,
            );
            out.len()
        });
    });
    c.bench_function("assign_slots_500_keep", |b| {
        b.iter(|| {
            assign_slots(
                &soldiers,
                &slots,
                &old,
                &regs.rules.formation,
                &prev,
                &mut out,
                &mut scratch,
            );
            out.len()
        });
    });
}

criterion_group!(benches, assign);
criterion_main!(benches);
