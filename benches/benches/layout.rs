//! T1-080: `layout_slots` for a 500-man regiment in each Phase 1 template
//! (TDD §7). Layout runs for every regiment whose formation changes, so it
//! sits inside the Stage 2 budget together with `assign_slots`.

use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use il_core::{S, Scalar};
use il_data::{ContentId, Registries};
use il_sim_battle::{effective_ranks, layout_slots};

fn layout(c: &mut Criterion) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../game");
    let regs = Registries::load_root(&root).unwrap_or_else(|d| panic!("{d}"));
    let radius = S::from_f32_data(0.4);
    let n = 500u16;
    let mut slots = Vec::with_capacity(usize::from(n));
    for id in [
        "rome:line",
        "rome:column",
        "rome:loose",
        "rome:phalanx",
        "rome:wedge",
        "rome:square",
    ] {
        let template = regs.formations.get(
            regs.formations
                .lookup(&ContentId::new(id).unwrap())
                .unwrap(),
        );
        let ranks = effective_ranks(template, n, None);
        let name = format!("layout_slots_500_{}", id.trim_start_matches("rome:"));
        c.bench_function(&name, |b| {
            b.iter(|| {
                layout_slots(template, n, ranks, radius, &mut slots);
                slots.len()
            })
        });
    }
}

criterion_group!(benches, layout);
criterion_main!(benches);
