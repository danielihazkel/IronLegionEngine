//! T2-022: death, kill credit, casualty rings, reform trigger
//! (SIM-CORE-008, SIM-CMBT-018, SIM-FORM-021).
//!
//! Regiment counts, the id lists, the grid and the hash stay consistent
//! after 5,000 scripted deaths at 1 and 8 threads and across a restore;
//! kill credit reconciles with the enemy's losses in a real fight; an
//! emptied regiment stays inert; front-rank gaps close after deaths.

mod common;

use common::*;
use il_core::{PlayerId, RegimentId, S, Scalar, SoldierId, StateHash, Tick};
use il_sim_battle::combat::{Kill, Kills};
use il_sim_battle::components::{Combat, FormationState, Health, MeleeState, Morale, Regiment};
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleEvent, BattleWorld, Command, CommandKind};

fn regiment_entity(w: &BattleWorld, id: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(id))
        .expect("regiment exists")
}

/// Queues the first `n` living soldiers of `regiment` as killed (hp to
/// zero, no killer), the way Stage 10 would.
fn kill_first(w: &mut BattleWorld, regiment: u32, n: usize) -> Vec<SoldierId> {
    let victims: Vec<SoldierId> = w
        .ecs()
        .get::<Regiment>(regiment_entity(w, regiment))
        .unwrap()
        .soldiers
        .iter()
        .copied()
        .take(n)
        .collect();
    for v in &victims {
        let e = w.ecs().resource::<Ids>().soldier_entity(*v).unwrap();
        w.ecs_mut().get_mut::<Health>(e).unwrap().hp = S::ZERO;
        w.ecs_mut().resource_mut::<Kills>().0.push(Kill {
            victim: *v,
            killer: None,
            killer_regiment: None,
        });
    }
    victims
}

/// Invariants hold once the next Stage 2 has laid the formations out again.
fn check_invariants(w: &mut BattleWorld) {
    w.step(&[]);
    let ids = w.ecs().resource::<Ids>();
    assert!(
        ids.soldier_entities.windows(2).all(|p| p[0].0 < p[1].0),
        "Ids not ascending"
    );
    let view = w.view();
    assert_eq!(view.spatial_grid().len(), ids.soldier_entities.len());
    let mut total = 0;
    for r in view.regiments() {
        let e = regiment_entity(w, r.id.0);
        let regiment = w.ecs().get::<Regiment>(e).unwrap();
        let f = w.ecs().get::<FormationState>(e).unwrap();
        assert_eq!(regiment.soldiers.len(), f.assignment.len(), "{:?}", r.id);
        assert_eq!(regiment.soldiers.len(), f.slots.len(), "{:?}", r.id);
        for sid in &regiment.soldiers {
            assert!(
                ids.soldier_entity(*sid).is_some(),
                "{sid:?} listed but gone"
            );
        }
        total += regiment.soldiers.len();
    }
    assert_eq!(total, ids.soldier_entities.len());
    for (_, e) in &ids.soldier_entities {
        if let Some(t) = w.ecs().get::<MeleeState>(*e).unwrap().target {
            assert!(ids.soldier_entity(t).is_some(), "target {t:?} is dead");
        }
        assert!(w.ecs().get::<Health>(*e).unwrap().hp > S::ZERO);
    }
}

/// Runs the 50-deaths-per-tick script for `ticks` ticks, returning hashes.
fn scripted_deaths(w: &mut BattleWorld, ticks: u32, per_tick: usize) -> (Vec<StateHash>, usize) {
    let mut hashes = Vec::new();
    let mut deaths = 0;
    for _ in 0..ticks {
        // Alternate sides so both shrink.
        let regiment = w.tick().0 % 2;
        deaths += kill_first(w, regiment, per_tick).len();
        let out = w.step(&[]);
        let died: Vec<SoldierId> = out
            .events
            .iter()
            .filter_map(|e| match e {
                BattleEvent::SoldierDied { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(died.len(), per_tick.min(deaths));
        assert!(died.windows(2).all(|p| p[0] < p[1]), "events not ascending");
        hashes.push(out.hash);
    }
    (hashes, deaths)
}

#[test]
fn five_thousand_deaths_keep_counts_ids_grid_and_hash_consistent() {
    let setup = two_sides(2_500);
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    let (hashes, deaths) = scripted_deaths(&mut w, 100, 50);
    assert_eq!(deaths, 5_000);
    assert_eq!(w.soldier_count(), 0);
    check_invariants(&mut w);
    // The casualty ring saw fifty per tick on the side that died that tick.
    let m = w.ecs().get::<Morale>(regiment_entity(&w, 1)).unwrap();
    assert_eq!(
        m.deaths_5s.iter().map(|d| u32::from(*d)).sum::<u32>(),
        50 * 50
    );
    assert_eq!(m.initial, 2_500);

    // Eight threads reproduce the hashes.
    let mut w8 = BattleWorld::new(&setup, regs()).unwrap();
    w8.set_threads(8);
    let (hashes8, _) = scripted_deaths(&mut w8, 100, 50);
    assert_eq!(hashes, hashes8, "1 vs 8 threads");

    // A restore half-way continues identically.
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    let (head, _) = scripted_deaths(&mut w, 50, 50);
    let snap = w.snapshot();
    let mut restored = BattleWorld::restore(&snap, regs()).unwrap();
    assert_eq!(restored.hash(), w.hash());
    let (a, _) = scripted_deaths(&mut w, 50, 50);
    let (b, _) = scripted_deaths(&mut restored, 50, 50);
    assert_eq!(head.len(), 50);
    assert_eq!(a, b, "restored run diverges");
    check_invariants(&mut restored);
}

#[test]
fn kill_credit_reconciles_with_the_enemy_losses() {
    let mut setup = two_sides(120);
    setup.sides[1].regiments[0].position = Some([340.0, 150.0]);
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    let attack = Command {
        tick: Tick(1),
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::AttackRegiment {
            regiments: vec![RegimentId(0)],
            target: RegimentId(1),
        },
    };
    let out = w.step(&[attack]);
    assert!(out.rejected.is_empty());
    let mut died_events = 0;
    while w.tick().0 < 2_000 {
        let out = w.step(&[]);
        died_events += out
            .events
            .iter()
            .filter(|e| matches!(e, BattleEvent::SoldierDied { .. }))
            .count();
    }
    let count = |id: u32| {
        w.ecs()
            .get::<Regiment>(regiment_entity(&w, id))
            .unwrap()
            .soldiers
            .len() as u32
    };
    let kills = |id: u32| {
        w.ecs()
            .get::<Combat>(regiment_entity(&w, id))
            .unwrap()
            .kills
    };
    assert!(count(0) < 120 && count(1) < 120, "no deaths in 2,000 ticks");
    assert_eq!(kills(0), 120 - count(1));
    assert_eq!(kills(1), 120 - count(0));
    assert_eq!(died_events as u32, 240 - count(0) - count(1));
    check_invariants(&mut w);
}

#[test]
fn an_emptied_regiment_stays_inert() {
    let setup = two_sides(40);
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    let victims = kill_first(&mut w, 1, 40);
    assert_eq!(victims.len(), 40);
    for _ in 0..200 {
        w.step(&[]);
    }
    assert_eq!(w.regiment_count(), 2);
    assert_eq!(w.soldier_count(), 40);
    let e = regiment_entity(&w, 1);
    assert!(w.ecs().get::<Regiment>(e).unwrap().soldiers.is_empty());
    let f = w.ecs().get::<FormationState>(e).unwrap();
    assert!(!f.needs_reform && f.slots.is_empty());
    check_invariants(&mut w);
}

#[test]
fn front_rank_gaps_close_after_deaths() {
    let setup = two_sides(60);
    let mut w = BattleWorld::new(&setup, regs()).unwrap();
    w.step(&[]);
    // The first ten soldiers hold the first ten slots, all in rank 0.
    let e = regiment_entity(&w, 0);
    {
        let f = w.ecs().get::<FormationState>(e).unwrap();
        assert!(f.slots[..10].iter().all(|s| s.rank == 0));
        assert!(f.assignment[..10].iter().all(|a| a.is_some()));
    }
    let victims = kill_first(&mut w, 0, 10);
    assert_eq!(victims.len(), 10);
    for _ in 0..30 {
        w.step(&[]);
    }
    let f = w.ecs().get::<FormationState>(e).unwrap();
    assert_eq!(f.slots.len(), 50);
    assert!(f.assignment.iter().all(|a| a.is_some()), "unfilled slots");
    let mut taken: Vec<u16> = f.assignment.iter().map(|a| a.unwrap()).collect();
    taken.sort_unstable();
    taken.dedup();
    assert_eq!(taken.len(), 50, "two soldiers share a slot");
    let front = f.slots.iter().filter(|s| s.rank == 0).count();
    let filled_front = f
        .assignment
        .iter()
        .filter(|a| a.is_some_and(|s| f.slots[usize::from(s)].rank == 0))
        .count();
    assert_eq!(filled_front, front, "front rank has gaps");
    check_invariants(&mut w);
}
