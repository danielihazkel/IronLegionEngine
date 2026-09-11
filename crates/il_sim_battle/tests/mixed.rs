//! T3-040 (REQ-FORM-008; SIM-FORM-012..014, SIM-MOVE-011): mixed regiments.
//! `tests/scenarios/mixed_cohort.json5` puts 40 velites and 100 hastati in
//! one `rome:cohort` (skirmishers in the front rank, infantry behind): the
//! velites spawn in rank 0 and the hastati behind, the anchor walks at the
//! hastati's pace, a reform after deaths keeps the zones, the snapshot
//! carries the groups, and the setup check rejects the malformed forms.

mod common;

use il_core::{RegimentId, S, Scalar, SoldierId};
use il_data::UnitCategory;
use il_sim_battle::combat::{Kill, Kills};
use il_sim_battle::components::{Anchor, Health, Rank, Regiment, Soldier};
use il_sim_battle::formation::files_used;
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleWorld, RegimentSetup, Scenario, SetupError, Snapshot, UnitGroupSetup};

/// The scenario file, embedded (sim crates never read the filesystem).
const MIXED_COHORT: &str = include_str!("../../../tests/scenarios/mixed_cohort.json5");

fn scenario() -> Scenario {
    json5::from_str(MIXED_COHORT).expect("scenario parses")
}

fn world() -> BattleWorld {
    let s = scenario();
    let mut w = BattleWorld::new(&s.setup, common::regs()).unwrap();
    w.set_threads(1);
    w
}

fn entity(w: &BattleWorld, rid: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap()
}

/// `(id, category, group, rank)` of every soldier of the regiment, ascending.
fn roster(w: &BattleWorld, rid: u32) -> Vec<(SoldierId, UnitCategory, u8, u8)> {
    let regiment = w.ecs().get::<Regiment>(entity(w, rid)).unwrap();
    regiment
        .soldiers
        .iter()
        .map(|sid| {
            let e = w.ecs().resource::<Ids>().soldier_entity(*sid).unwrap();
            let s = w.ecs().get::<Soldier>(e).unwrap();
            let r = w.ecs().get::<Rank>(e).unwrap();
            (*sid, s.category, s.group, r.rank)
        })
        .collect()
}

/// Queues `n` soldiers of group `g` for death (the Stage 10 path).
fn kill_of_group(w: &mut BattleWorld, rid: u32, g: u8, n: usize) {
    let victims: Vec<SoldierId> = roster(w, rid)
        .into_iter()
        .filter(|(_, c, group, _)| *group == g && *c != UnitCategory::General)
        .map(|(id, ..)| id)
        .take(n)
        .collect();
    assert_eq!(victims.len(), n);
    for v in victims {
        let e = w.ecs().resource::<Ids>().soldier_entity(v).unwrap();
        w.ecs_mut().get_mut::<Health>(e).unwrap().hp = S::ZERO;
        w.ecs_mut().resource_mut::<Kills>().0.push(Kill {
            victim: v,
            killer: None,
            killer_regiment: None,
        });
    }
    w.recompute_hash();
}

/// Rank 0 holds skirmishers only, and every front-rank slot is filled.
fn assert_front_rank_is_velites(w: &BattleWorld, rid: u32, what: &str) {
    let roster = roster(w, rid);
    let front: Vec<_> = roster.iter().filter(|(_, _, _, rank)| *rank == 0).collect();
    let files = files_used(&w.view().formation_state(RegimentId(rid)).unwrap().slots);
    assert_eq!(
        front.len(),
        usize::from(files),
        "{what}: the front rank is full"
    );
    assert!(
        front
            .iter()
            .all(|(_, c, ..)| *c == UnitCategory::Skirmisher),
        "{what}: a non-skirmisher in the front rank: {front:?}"
    );
    assert!(
        roster
            .iter()
            .filter(|(_, c, ..)| *c == UnitCategory::Infantry)
            .all(|(_, _, _, rank)| *rank >= 1),
        "{what}: a hastatus in the front rank"
    );
}

#[test]
fn the_cohort_spawns_the_velites_in_front_and_the_hastati_behind() {
    let w = world();
    let regs = w.registries().clone();
    let r = w.ecs().get::<Regiment>(entity(&w, 0)).unwrap().clone();
    // The composition in setup order, the general counted in group 0.
    let names: Vec<(&str, u16, u8)> = r
        .units
        .iter()
        .map(|g| (regs.units.get(g.unit).id.as_str(), g.count, g.experience))
        .collect();
    assert_eq!(
        names,
        vec![("rome:velites", 41, 0), ("rome:hastati", 100, 2)]
    );
    assert_eq!(regs.units.id_of(r.unit).as_str(), "rome:velites");
    assert_eq!(r.soldiers.len(), 141);
    // Ids ascend by group: 40 velites, 100 hastati, then the general in
    // group 0 with its own unit.
    let roster = roster(&w, 0);
    assert!(
        roster[..40]
            .iter()
            .all(|(_, c, g, _)| *c == UnitCategory::Skirmisher && *g == 0)
    );
    assert!(
        roster[40..140]
            .iter()
            .all(|(_, c, g, _)| *c == UnitCategory::Infantry && *g == 1)
    );
    assert_eq!(roster[140].1, UnitCategory::General);
    assert_eq!(roster[140].2, 0);
    // 141 in four ranks are 36 files: the front rank takes 36 velites, the
    // other four spill to the free slots behind (SIM-FORM-011).
    assert_front_rank_is_velites(&w, 0, "spawn");
    let velites_behind = roster
        .iter()
        .filter(|(_, c, _, rank)| *c == UnitCategory::Skirmisher && *rank > 0)
        .count();
    assert_eq!(velites_behind, 4);
    // The regiment's experience is the count-weighted mean (200 / 140 = 1),
    // and it fires (a velite throws).
    let view = w.view();
    let row = view.regiment(RegimentId(0)).unwrap();
    assert!(row.fire.is_some(), "Fire exists: a group has ranged");
    assert_eq!(
        w.ecs()
            .get::<il_sim_battle::components::Combat>(entity(&w, 0))
            .unwrap()
            .experience,
        1
    );
    assert_eq!(view.regiment_living_by_group(RegimentId(0)), vec![41, 100]);
    assert_eq!(view.regiment_units(RegimentId(0)).len(), 2);
    assert!(w.setup_warnings().is_empty());
}

#[test]
fn the_cohort_walks_at_the_hastati_pace() {
    let s = scenario();
    let mut w = BattleWorld::new(&s.setup, common::regs()).unwrap();
    let regs = w.registries().clone();
    let mut script = s.script();
    let velites = regs
        .units
        .get(regs.units.lookup(&common::cid("rome:velites")).unwrap());
    let hastati = regs
        .units
        .get(regs.units.lookup(&common::cid("rome:hastati")).unwrap());
    assert!(
        velites.speed_walk > hastati.speed_walk,
        "the velites are the faster unit"
    );
    for _ in 0..20 {
        let cmds = script.take_for(w.tick().next());
        w.step(&cmds);
    }
    let a = w.ecs().get::<Anchor>(entity(&w, 0)).unwrap().pos;
    for _ in 0..100 {
        let cmds = script.take_for(w.tick().next());
        w.step(&cmds);
    }
    let b = w.ecs().get::<Anchor>(entity(&w, 0)).unwrap().pos;
    let per_tick = a.distance(b) / S::from_i32(100);
    let walk = |u: &il_data::UnitType| u.speed_walk * il_sim_battle::movement::tick_dt();
    assert!(
        per_tick > walk(hastati) * S::from_f32_data(0.8),
        "the anchor crawls: {per_tick:?} per tick against the hastati's {:?}",
        walk(hastati)
    );
    assert!(
        per_tick < walk(velites) * S::from_f32_data(0.95),
        "the anchor moves at the velites' pace: {per_tick:?} per tick against {:?}",
        walk(velites)
    );
}

#[test]
fn a_reform_after_deaths_keeps_the_zones() {
    let mut w = world();
    w.step(&[]);
    kill_of_group(&mut w, 0, 1, 30);
    kill_of_group(&mut w, 0, 0, 10);
    // Stage 15 resolves the deaths this tick; Stage 2 reforms on the next.
    w.step(&[]);
    w.step(&[]);
    let r = w.ecs().get::<Regiment>(entity(&w, 0)).unwrap().clone();
    assert_eq!(r.soldiers.len(), 101);
    assert_eq!(
        w.view().regiment_living_by_group(RegimentId(0)),
        vec![31, 70]
    );
    // 101 in four ranks are 26 files: 26 velites in front, 4 behind.
    assert_front_rank_is_velites(&w, 0, "after deaths");
    // And again after a second round of losses.
    kill_of_group(&mut w, 0, 1, 20);
    w.step(&[]);
    w.step(&[]);
    assert_front_rank_is_velites(&w, 0, "after more deaths");
    assert_eq!(
        w.view().regiment_living_by_group(RegimentId(0)),
        vec![31, 50]
    );
}

#[test]
fn a_snapshot_carries_the_groups() {
    let mut w = world();
    for _ in 0..50 {
        w.step(&[]);
    }
    let snap = w.snapshot();
    let bytes = snap.to_bytes();
    let back = Snapshot::from_bytes(&bytes).unwrap();
    let mut restored = BattleWorld::restore(&back, common::regs()).unwrap();
    assert_eq!(restored.hash(), w.hash());
    let a = w
        .ecs()
        .get::<Regiment>(entity(&w, 0))
        .unwrap()
        .units
        .clone();
    let b = restored
        .ecs()
        .get::<Regiment>(entity(&restored, 0))
        .unwrap()
        .units
        .clone();
    assert_eq!(a, b);
    assert_eq!(roster(&w, 0), roster(&restored, 0));
    assert_front_rank_is_velites(&restored, 0, "restored");
    for _ in 0..100 {
        assert_eq!(w.step(&[]).hash, restored.step(&[]).hash);
    }
}

#[test]
fn the_setup_check_rejects_the_malformed_forms() {
    let regs = common::regs();
    let mixed = |units: Vec<(&str, u16)>| RegimentSetup {
        position: Some([300.0, 150.0]),
        facing_deg: Some(0.0),
        ..RegimentSetup::mixed(
            1,
            units
                .into_iter()
                .map(|(u, n)| UnitGroupSetup {
                    unit_type: common::cid(u),
                    count: n,
                    experience: 0,
                })
                .collect(),
        )
    };
    let with = |r: RegimentSetup| {
        let mut s = common::two_sides(20);
        s.sides[0].regiments = vec![r];
        s
    };
    // Both forms at once.
    let mut both = mixed(vec![("rome:velites", 40), ("rome:hastati", 100)]);
    both.count = Some(140);
    assert_eq!(
        BattleWorld::new(&with(both), regs.clone()).unwrap_err(),
        SetupError::BothUnitForms {
            side: 0,
            regiment: 1
        }
    );
    // Neither form.
    let none = mixed(vec![]);
    assert_eq!(
        BattleWorld::new(&with(none), regs.clone()).unwrap_err(),
        SetupError::EmptyComposition {
            side: 0,
            regiment: 1
        }
    );
    // An unknown unit in a group, a group of nobody.
    assert_eq!(
        BattleWorld::new(
            &with(mixed(vec![("rome:velites", 40), ("rome:nope", 1)])),
            regs.clone()
        )
        .unwrap_err(),
        SetupError::UnknownUnit {
            side: 0,
            regiment: 1,
            unit_type: common::cid("rome:nope")
        }
    );
    assert_eq!(
        BattleWorld::new(
            &with(mixed(vec![("rome:velites", 40), ("rome:hastati", 0)])),
            regs.clone()
        )
        .unwrap_err(),
        SetupError::GroupCountZero {
            side: 0,
            regiment: 1
        }
    );
    // A unit type may repeat: two hastati groups with their own experience.
    let mut twice = mixed(vec![("rome:hastati", 60), ("rome:hastati", 60)]);
    twice.units[1].experience = 5;
    let w = BattleWorld::new(&with(twice), regs.clone()).unwrap();
    let r = w.ecs().get::<Regiment>(entity(&w, 0)).unwrap();
    assert_eq!(r.units.len(), 2);
    assert_eq!((r.units[0].count, r.units[1].count), (61, 60));
    assert_eq!(
        w.view().regiment_living_by_group(RegimentId(0)),
        vec![61, 60]
    );
    // The shorthand still reads as one group.
    let w = common::world(20);
    let r = w.ecs().get::<Regiment>(entity(&w, 0)).unwrap();
    assert_eq!(r.units.len(), 1);
    assert_eq!(r.units[0].count, 21, "the general counted in group 0");
}

#[test]
fn a_zoneless_template_lays_the_groups_out_in_list_order() {
    // Hastati first, velites behind, in a plain line: the hastati take the
    // front ranks and the velites the rear, front to back in list order.
    let regs = common::regs();
    let mut s = common::two_sides(20);
    s.sides[0].regiments = vec![RegimentSetup {
        formation: Some(common::cid("rome:line")),
        position: Some([300.0, 150.0]),
        facing_deg: Some(0.0),
        ..RegimentSetup::mixed(
            1,
            vec![
                UnitGroupSetup {
                    unit_type: common::cid("rome:hastati"),
                    count: 60,
                    experience: 0,
                },
                UnitGroupSetup {
                    unit_type: common::cid("rome:velites"),
                    count: 60,
                    experience: 0,
                },
            ],
        )
    }];
    let w = BattleWorld::new(&s, regs).unwrap();
    let roster = roster(&w, 0);
    let max_hastati_rank = roster
        .iter()
        .filter(|(_, c, ..)| *c == UnitCategory::Infantry)
        .map(|(.., rank)| *rank)
        .max()
        .unwrap();
    let min_velites_rank = roster
        .iter()
        .filter(|(_, c, ..)| *c == UnitCategory::Skirmisher)
        .map(|(.., rank)| *rank)
        .min()
        .unwrap();
    assert!(
        max_hastati_rank <= min_velites_rank,
        "hastati up to rank {max_hastati_rank}, velites from rank {min_velites_rank}"
    );
    assert_eq!(roster[0].3, 0);
}

#[test]
fn groups_sharing_no_formation_take_the_first_list_with_a_warning() {
    // `tests/mods/velites_wedge_only` leaves the velites only the wedge: a
    // cohort of hastati and velites has no formation in common, so the
    // hastati's list stands and the world carries one warning naming the
    // regiment (SIM-FORM-014).
    let regs = common::regs_with_mod("velites_wedge_only");
    let mut s = common::two_sides(20);
    s.sides[0].regiments = vec![RegimentSetup {
        position: Some([300.0, 150.0]),
        facing_deg: Some(0.0),
        ..RegimentSetup::mixed(
            7,
            vec![
                UnitGroupSetup {
                    unit_type: common::cid("rome:hastati"),
                    count: 60,
                    experience: 0,
                },
                UnitGroupSetup {
                    unit_type: common::cid("rome:velites"),
                    count: 20,
                    experience: 0,
                },
            ],
        )
    }];
    let w = BattleWorld::new(&s, regs.clone()).unwrap();
    let warnings = w.setup_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!((warnings[0].side, warnings[0].regiment), (0, 7));
    assert!(warnings[0].text.contains("SIM-FORM-014"), "{}", warnings[0]);
    let hastati = regs
        .units
        .get(regs.units.lookup(&common::cid("rome:hastati")).unwrap());
    assert_eq!(
        w.view().regiment_formations(RegimentId(0)),
        hastati.formations
    );
    // The unmodded game shares three formations: no warning there.
    let plain = BattleWorld::new(&s, common::regs()).unwrap();
    assert!(plain.setup_warnings().is_empty());
}
