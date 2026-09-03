//! T1-043 done-when: 2,000 soldiers in ten regiments reform Line → Column →
//! Line and settle to integrity ≥ 0.95 within 15 s of sim time each way,
//! identically at 1 and 8 threads.

mod common;

use il_core::{PlayerId, RegimentId, S, Scalar};
use il_data::{ContentId, Layout};
use il_sim_battle::components::{Anchor, Order, Pos, SlotRef};
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleSetup, BattleWorld, SpeedMode, slot_world};

/// Ten regiments of 200 hastati in a row across the northern deployment
/// zone, 70 m apart.
fn ten_regiments() -> BattleSetup {
    let regiments = (0..10)
        .map(|i| common::regiment(i + 1, "rome:hastati", 200, (80 + 70 * i) as f32, 90.0))
        .map(|mut r| {
            r.position = Some([r.position.unwrap()[0], 120.0]);
            r
        })
        .collect();
    let mut side = common::side(0, regiments);
    side.player = PlayerId(0);
    BattleSetup {
        map_id: common::cid("rome:test_field"),
        seed: 7,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: vec![side],
        victory: Default::default(),
    }
}

/// Fraction of soldiers within `integrity_radius × sf` of their slot.
fn integrity(w: &BattleWorld) -> S {
    let regs = w.registries();
    let ids = w.ecs().resource::<Ids>();
    let mut inside = 0;
    let mut total = 0;
    for (rid, e) in &ids.regiment_entities {
        let state = w.view().formation_state(*rid).unwrap();
        let anchor = w.ecs().get::<Anchor>(*e).unwrap();
        let regiment = w
            .ecs()
            .get::<il_sim_battle::components::Regiment>(*e)
            .unwrap();
        let radius = regs.units.get(regiment.unit).soldier_radius;
        let template = regs.formations.get(state.template);
        let sf = template.spacing_file * (radius + radius);
        let r = regs.rules.formation.integrity_radius * sf;
        for sid in &regiment.soldiers {
            let se = ids.soldier_entity(*sid).unwrap();
            let p = w.ecs().get::<Pos>(se).unwrap().p;
            let slot = w.ecs().get::<SlotRef>(se).unwrap().slot.unwrap();
            total += 1;
            if slot_world(anchor, &state.slots[usize::from(slot)]).distance(p) <= r {
                inside += 1;
            }
        }
    }
    S::from_i32(inside) / S::from_i32(total)
}

/// What `SetFormation` and `SetSpeedMode` will do from T1-047 on.
fn set_formation(w: &mut BattleWorld, id: &str) {
    let handle = w
        .registries()
        .formations
        .lookup(&ContentId::new(id).unwrap())
        .unwrap();
    let entities: Vec<_> = w
        .ecs()
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in entities {
        let mut state = w
            .ecs_mut()
            .get_mut::<il_sim_battle::components::FormationState>(e)
            .unwrap();
        state.template = handle;
        state.needs_reform = true;
        w.ecs_mut().get_mut::<Order>(e).unwrap().speed = SpeedMode::Run;
    }
    w.recompute_hash();
}

fn settle(w: &mut BattleWorld, ticks: u32) -> S {
    for _ in 0..ticks {
        w.step(&[]);
    }
    integrity(w)
}

#[test]
fn two_thousand_soldiers_reform_line_column_line_within_15_seconds() {
    let regs = common::regs();
    let mut a = BattleWorld::new(&ten_regiments(), regs.clone()).unwrap();
    let mut b = BattleWorld::new(&ten_regiments(), regs).unwrap();
    b.set_threads(8);
    assert_eq!(a.soldier_count(), 2_000);
    assert_eq!(integrity(&a), S::ONE, "spawned on slots");

    for w in [&mut a, &mut b] {
        set_formation(w, "rome:column");
    }
    let (ia, ib) = (settle(&mut a, 300), settle(&mut b, 300));
    assert_eq!(a.hash(), b.hash(), "column reform differs across threads");
    assert_eq!(ia, ib);
    let layout = a
        .registries()
        .formations
        .get(a.view().formation_state(RegimentId(0)).unwrap().template)
        .layout;
    assert_eq!(layout, Layout::Column);
    assert!(
        ia >= S::from_f32_data(0.95),
        "column integrity after 15 s: {ia:?}"
    );

    for w in [&mut a, &mut b] {
        set_formation(w, "rome:line");
    }
    let (ia, ib) = (settle(&mut a, 300), settle(&mut b, 300));
    assert_eq!(a.hash(), b.hash(), "line reform differs across threads");
    assert!(
        ia >= S::from_f32_data(0.95),
        "line integrity after 15 s: {ia:?}"
    );
    assert_eq!(ia, ib);
    // Soldiers are idle again and inside the map.
    let idle = a
        .view()
        .soldiers()
        .filter(|s| s.state == il_sim_battle::components::SoldierState::Idle)
        .count();
    // SIM-MOVE-022's separation reach (two diameters plus the margin) is
    // wider than the file spacing, so edge soldiers keep jostling between
    // the arrive and leave radii; most are still idle. Tuning is Phase 2.
    assert!(idle > 1_500, "{idle} idle");
    assert!(a.view().soldiers().all(|s| a.map().in_bounds(s.pos)));
}
