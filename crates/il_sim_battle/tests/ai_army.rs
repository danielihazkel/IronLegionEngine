//! T2-082: the army AI (SIM-AI-010..014): stance and its hysteresis, the
//! blind target, the stepping line, skirmishers, flank groups and the
//! charge trigger, the defend ground, counter-charges, reserves and the
//! retreat with its cavalry screen.

mod common;

use il_core::{PlayerId, RegimentId, S, Scalar, Tick, V2};
use il_sim_battle::ai::army::{ArmyContext, choose_stance, highest_ground};
use il_sim_battle::ai::inputs::SideSnapshot;
use il_sim_battle::components::{Anchor, Morale, OrderKind};
use il_sim_battle::flow_battle::zone_centre;
use il_sim_battle::resources::Ids;
use il_sim_battle::{
    ArmyPlan, BattleSetup, BattleWorld, Command, CommandKind, RegimentSetup, Role, SideSetup,
    SpeedMode, Stance,
};

fn regiment(id: u32, unit: &str, count: u16, x: f32, facing_deg: f32) -> RegimentSetup {
    common::regiment(id, unit, count, x, facing_deg)
}

fn at(id: u32, unit: &str, count: u16, x: f32, y: f32, facing_deg: f32) -> RegimentSetup {
    RegimentSetup {
        position: Some([x, y]),
        ..regiment(id, unit, count, x, facing_deg)
    }
}

fn side(player: u8, zone: u8, regiments: Vec<RegimentSetup>) -> SideSetup {
    SideSetup {
        deployment_zone: zone,
        ..common::side(player, regiments)
    }
}

fn setup(sides: Vec<SideSetup>) -> BattleSetup {
    BattleSetup {
        sides,
        ..common::two_sides(1)
    }
}

/// `n` regiments of `unit` in a north-south line at `x`, 50 m apart.
fn line_of(
    n: u32,
    unit: &str,
    count: u16,
    x: f32,
    facing: f32,
    first_id: u32,
) -> Vec<RegimentSetup> {
    (0..n)
        .map(|k| {
            let y = (150 + (2 * k as i32 - (n as i32 - 1)) * 25) as f32;
            at(first_id + k, unit, count, x, y, facing)
        })
        .collect()
}

struct Run {
    ai: Vec<Command>,
    rejected: usize,
}

fn run(w: &mut BattleWorld, ticks: u32) -> Run {
    let mut r = Run {
        ai: Vec::new(),
        rejected: 0,
    };
    for _ in 0..ticks {
        let out = w.step(&[]);
        r.ai.extend(out.ai_commands.iter().cloned());
        r.rejected += out.rejected.len();
    }
    r
}

fn plan(w: &BattleWorld, side: u8) -> ArmyPlan {
    w.view().ai_plan(side).expect("the army decided").clone()
}

fn anchor(w: &BattleWorld, id: RegimentId) -> V2 {
    w.view().regiment(id).unwrap().anchor_pos
}

fn entity(w: &BattleWorld, id: RegimentId) -> bevy_ecs::entity::Entity {
    w.ecs().resource::<Ids>().regiment_entity(id).unwrap()
}

fn set_morale(w: &mut BattleWorld, id: RegimentId, m: i32) {
    let e = entity(w, id);
    w.ecs_mut().get_mut::<Morale>(e).unwrap().m = S::from_i32(m);
    w.recompute_hash();
}

/// (l) An even army attacks, aiming at the enemy zones while nothing is
/// visible and at the enemy centroid once it is.
#[test]
fn army_attacks_an_even_enemy_toward_the_zone_then_the_centroid() {
    // 250 m apart: nothing visible from low ground (about 184 m).
    let s = setup(vec![
        side(0, 0, line_of(3, "rome:hastati", 120, 250.0, 0.0, 1)),
        side(255, 1, line_of(3, "rome:hastati", 120, 500.0, 180.0, 4)),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let r = run(&mut w, 2);
    assert_eq!(r.rejected, 0);
    let p = plan(&w, 1);
    assert_eq!(p.stance, Stance::Attack);
    assert!(
        p.target.distance(zone_centre(w.map(), 0)) < S::ONE,
        "{:?}",
        p.target
    );
    assert_eq!(p.decided_at, Tick(1));
    // Visible at 150 m: the target is the enemy centroid.
    let s = setup(vec![
        side(0, 0, line_of(3, "rome:hastati", 120, 350.0, 0.0, 1)),
        side(255, 1, line_of(3, "rome:hastati", 120, 500.0, 180.0, 4)),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    run(&mut w, 2);
    let p = plan(&w, 1);
    assert_eq!(p.stance, Stance::Attack);
    assert!(
        p.target.distance(V2::from_f32_data(350.0, 150.0)) < S::ONE,
        "{:?}",
        p.target
    );
    assert!(p.line_facing.direction().x < S::ZERO, "faces west");
}

/// (l) A beaten remnant retreats: the infantry withdraws, the cavalry
/// screens where the retreat began and withdraws once the infantry is
/// `screen_gap` away.
#[test]
fn beaten_army_retreats_behind_a_cavalry_screen() {
    let mut s = setup(vec![
        side(0, 0, line_of(3, "rome:hastati", 120, 350.0, 0.0, 1)),
        side(
            255,
            1,
            vec![
                regiment(4, "rome:hastati", 40, 500.0, 180.0),
                regiment(5, "persia:cavalry", 20, 520.0, 180.0),
            ],
        ),
    ]);
    s.sides[1].general.bodyguard = Some(4);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let (foot, horse) = (RegimentId(3), RegimentId(4));
    // Half the regiment already gone: casualty fraction 0.5.
    {
        let e = entity(&w, foot);
        w.ecs_mut().get_mut::<Morale>(e).unwrap().initial = 80;
        w.recompute_hash();
    }
    let r = run(&mut w, 2);
    assert_eq!(r.rejected, 0);
    let p = plan(&w, 1);
    assert_eq!(p.stance, Stance::Retreat, "{:?}", p.stance_score);
    assert!(r.ai.iter().any(|c| matches!(
        &c.kind,
        CommandKind::Withdraw { regiments } if regiments == &vec![foot]
    )));
    let screen = match p.role_of(horse) {
        Some(Role::Screen { slot }) => slot,
        other => panic!("{other:?}"),
    };
    assert_eq!(w.view().regiment(foot).unwrap().order, OrderKind::Withdraw);
    let mut withdrew = None;
    for _ in 0..3000 {
        let next = w.tick().next();
        let out = w.step(&[]);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        if out.ai_commands.iter().any(|c| {
            matches!(
                &c.kind,
                CommandKind::Withdraw { regiments } if regiments == &vec![horse]
            )
        }) {
            withdrew = Some(next.0);
            break;
        }
        if let Some(Role::Screen { slot }) = plan(&w, 1).role_of(horse) {
            assert!(slot.distance(screen) < S::ONE, "the screen slot moved");
        }
    }
    let t = withdrew.expect("the cavalry withdrew after screening");
    assert!(
        anchor(&w, foot).distance(anchor(&w, horse)) > S::from_i32(70),
        "tick {t}"
    );
}

/// (m) Hysteresis (plan decision 10): a challenger within `stance_margin`
/// of the current stance loses; beyond it wins; retreat always wins.
#[test]
fn stance_hysteresis_holds_the_current_stance_within_the_margin() {
    let s = setup(vec![
        side(0, 0, line_of(1, "rome:hastati", 120, 350.0, 0.0, 1)),
        side(255, 1, line_of(1, "rome:hastati", 120, 500.0, 180.0, 2)),
    ]);
    let w = BattleWorld::new(&s, common::regs()).unwrap();
    let regs = w.registries();
    let snap = SideSnapshot::build(w.ecs(), 1);
    let profile = regs.ai_profiles.iter().next().unwrap().1;
    let ctx = ArmyContext {
        snap: &snap,
        profile,
        map: w.map(),
        height_ref: regs.rules.combat.height_ref,
        tick: Tick(1),
        battle_start: Tick(0),
        time_limit: 48_000,
    };
    let set = |attack: f32, retreat: f32| -> il_data::AiActionSet {
        let src = format!(
            r#"{{ id: "t:a", scope: "army", actions: [
                {{ name: "attack", kind: "attack", base: {attack} }},
                {{ name: "defend", kind: "defend", base: 0.5 }},
                {{ name: "retreat", kind: "retreat", base: {retreat} }} ] }}"#
        );
        serde_json::from_value(
            il_data::json5::parse_json5(&src, il_data::json5::FileId(0))
                .unwrap()
                .to_json(),
        )
        .unwrap()
    };
    let mut rng = il_core::RngStream::from_seed(1, il_core::StreamId::AiArmy);
    let margin = S::from_f32_data(0.1);
    let current = Some((Stance::Defend, S::from_f32_data(0.5)));
    let (st, _) = choose_stance(&set(0.55, 0.0), &ctx, current, margin, &mut rng);
    assert_eq!(st, Stance::Defend, "0.55 is within the margin of 0.5");
    let (st, sc) = choose_stance(&set(0.65, 0.0), &ctx, current, margin, &mut rng);
    assert_eq!((st, sc), (Stance::Attack, S::from_f32_data(0.65)));
    let (st, _) = choose_stance(&set(0.0, 0.51), &ctx, current, margin, &mut rng);
    assert_eq!(st, Stance::Retreat, "retreat needs no margin");
    let (st, _) = choose_stance(&set(0.55, 0.0), &ctx, None, margin, &mut rng);
    assert_eq!(st, Stance::Attack, "no current stance: the winner");
}

/// (n) The attacking line forms where it stands and steps toward the enemy
/// by at most `advance_step` per period, only once formed; the line
/// regiments' slots lie on the line.
#[test]
fn attacking_line_steps_toward_the_enemy_once_formed() {
    let s = setup(vec![
        side(0, 0, line_of(3, "rome:hastati", 120, 250.0, 0.0, 1)),
        side(255, 1, line_of(3, "rome:hastati", 120, 500.0, 180.0, 4)),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let advance_step = w
        .registries()
        .ai_profiles
        .iter()
        .next()
        .unwrap()
        .1
        .advance_step;
    let mut last: Option<ArmyPlan> = None;
    let mut moved = 0;
    for _ in 0..2000 {
        let out = w.step(&[]);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        let p = plan(&w, 1);
        if last.as_ref().is_none_or(|l| l.decided_at != p.decided_at) {
            if let Some(l) = &last {
                let step = l.line_anchor.distance(p.line_anchor);
                if step > S::from_f32_data(0.01) {
                    assert!(step <= advance_step + S::from_f32_data(0.01), "{step:?}");
                    let forward = l.line_facing.direction();
                    assert!(
                        (p.line_anchor - l.line_anchor).dot(forward) > -S::ONE,
                        "stepped away"
                    );
                    moved += 1;
                }
            }
            let forward = p.line_facing.direction();
            for a in &p.assignments {
                if let Role::Line { slot } = a.role {
                    let off = (slot - p.line_anchor).dot(forward).abs();
                    assert!(off < S::ONE, "slot off the line by {off:?}");
                }
            }
            last = Some(p);
        }
    }
    assert!(moved >= 5, "the line advanced {moved} times");
    assert!(
        anchor(&w, RegimentId(4)).x < S::from_i32(470),
        "the centre regiment walked west"
    );
}

/// (o) Skirmishers stand off at `skirmish_range_frac` of their range from
/// the nearest enemy: far ahead of a distant line, level with a close one;
/// never closer than that.
#[test]
fn skirmishers_stand_off_at_their_range() {
    let army = || {
        let mut v = line_of(2, "rome:hastati", 120, 500.0, 180.0, 2);
        v.push(regiment(4, "rome:velites", 60, 490.0, 180.0));
        v
    };
    let s = setup(vec![
        side(0, 0, line_of(2, "rome:hastati", 120, 350.0, 0.0, 0)),
        side(255, 1, army()),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    run(&mut w, 2);
    let p = plan(&w, 1);
    let forward = p.line_facing.direction();
    match p.role_of(RegimentId(4)) {
        Some(Role::Skirmish { slot }) => {
            assert!((slot - p.line_anchor).dot(forward) > S::from_i32(5))
        }
        other => panic!("{other:?}"),
    }
    let s = setup(vec![
        side(0, 0, line_of(2, "rome:hastati", 120, 465.0, 0.0, 0)),
        side(255, 1, army()),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    run(&mut w, 2);
    let p = plan(&w, 1);
    let forward = p.line_facing.direction();
    match p.role_of(RegimentId(4)) {
        Some(Role::Skirmish { slot }) => {
            // 36 m (0.9 x 40) from the nearest enemy, no closer, and no
            // longer ahead of a line that is itself 35 m from the enemy.
            let nearest = anchor(&w, RegimentId(0))
                .distance(slot)
                .min(anchor(&w, RegimentId(1)).distance(slot));
            assert!(
                (nearest - S::from_i32(36)).abs() < S::from_i32(2),
                "{nearest:?}"
            );
            let ahead = (slot - p.line_anchor).dot(forward);
            assert!(
                ahead < S::from_i32(40),
                "slot {slot:?} anchor {:?} ahead {ahead:?}",
                p.line_anchor
            );
        }
        other => panic!("{other:?}"),
    }
}

/// (p) Cavalry heads for the enemy's flank at run and charges the
/// rear-most visible regiment once the lines are within
/// `charge_trigger_dist`.
#[test]
fn cavalry_takes_the_flank_and_charges_when_the_lines_close() {
    let army = || {
        let mut v = line_of(2, "rome:hastati", 120, 500.0, 180.0, 2);
        v.push(regiment(4, "persia:cavalry", 30, 520.0, 180.0));
        v
    };
    let horse = RegimentId(4);
    let s = setup(vec![
        side(0, 0, line_of(2, "rome:hastati", 120, 380.0, 0.0, 0)),
        side(255, 1, army()),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let r = run(&mut w, 45);
    assert_eq!(r.rejected, 0);
    let p = plan(&w, 1);
    let f = p.line_facing.direction();
    let right = V2::new(f.y, -f.x);
    match p.role_of(horse) {
        Some(Role::Flank { slot, charge: None }) => {
            // Beyond the enemy's lateral extent by `flank_offset` (40 m).
            assert!(
                (slot - p.target).dot(right).abs() > S::from_i32(40),
                "{slot:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(w.view().regiment(horse).unwrap().order, OrderKind::Move);
    assert!(r.ai.iter().any(|c| matches!(
        &c.kind,
        CommandKind::Move { regiments, speed: SpeedMode::Run, .. } if regiments == &vec![horse]
    )));
    // Lines 30 m apart: the charge is on.
    let s = setup(vec![
        side(0, 0, line_of(2, "rome:hastati", 120, 470.0, 0.0, 0)),
        side(255, 1, army()),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let r = run(&mut w, 45);
    let p = plan(&w, 1);
    assert!(p.charging);
    assert!(matches!(
        p.role_of(horse),
        Some(Role::Flank {
            charge: Some(_),
            ..
        })
    ));
    assert!(r.ai.iter().any(|c| matches!(
        &c.kind,
        CommandKind::AttackRegiment { regiments, .. } if regiments == &vec![horse]
    )));
    assert_eq!(
        w.view().regiment(horse).unwrap().order,
        OrderKind::AttackRegiment
    );
}

/// (q) On defend the line forms on the highest ground within
/// `defend_search_radius` of the centroid and stays there.
#[test]
fn defending_army_takes_the_high_ground() {
    let mut s = setup(vec![
        side(0, 0, vec![at(1, "rome:hastati", 180, 500.0, 560.0, 270.0)]),
        side(
            255,
            1,
            vec![
                at(2, "rome:hastati", 100, 500.0, 420.0, 90.0),
                at(3, "rome:hastati", 20, 700.0, 420.0, 90.0),
            ],
        ),
    ]);
    s.sides[1].general.bodyguard = Some(3);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let r = run(&mut w, 2);
    assert_eq!(r.rejected, 0);
    let p = plan(&w, 1);
    assert_eq!(p.stance, Stance::Defend, "{:?}", p.stance_score);
    let map = w.map().clone();
    let centroid = V2::from_f32_data(500.0, 420.0);
    let expected = highest_ground(&map, centroid, S::from_i32(200));
    assert!(
        p.line_anchor.distance(expected) < S::ONE,
        "{:?} vs {expected:?}",
        p.line_anchor
    );
    assert!(
        map.height_at(p.line_anchor) > map.height_at(centroid) + S::ONE,
        "took a hill"
    );
    run(&mut w, 200);
    let later = plan(&w, 1);
    assert_eq!(later.stance, Stance::Defend);
    assert!(
        later.line_anchor.distance(p.line_anchor) < S::ONE,
        "the defend line does not wander"
    );
}

/// (r) On defend the cavalry counter-charges an enemy that comes within
/// `counter_charge_dist` of a line end.
#[test]
fn defending_cavalry_counter_charges_a_threatened_flank() {
    let mut s = setup(vec![
        side(
            0,
            0,
            vec![
                at(1, "rome:hastati", 180, 500.0, 560.0, 270.0),
                at(2, "rome:velites", 20, 560.0, 560.0, 270.0),
            ],
        ),
        side(
            255,
            1,
            vec![
                at(3, "rome:hastati", 100, 500.0, 420.0, 90.0),
                at(4, "persia:cavalry", 20, 560.0, 400.0, 90.0),
                at(5, "rome:hastati", 20, 700.0, 420.0, 90.0),
            ],
        ),
    ]);
    s.sides[1].general.bodyguard = Some(5);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let (velites, horse) = (RegimentId(1), RegimentId(3));
    run(&mut w, 2);
    let p = plan(&w, 1);
    assert_eq!(p.stance, Stance::Defend);
    let slot = match p.role_of(horse) {
        Some(Role::Counter { slot }) => slot,
        other => panic!("{other:?}"),
    };
    // The counter slot sits `reserve_offset` (60 m) behind the line end it
    // guards; the velites step onto that end before the next army period.
    let end = slot + p.line_facing.direction() * S::from_i32(60);
    let e = entity(&w, velites);
    w.ecs_mut().get_mut::<Anchor>(e).unwrap().pos = end;
    w.recompute_hash();
    // The next army period (tick 41) commits the cavalry; its next
    // regiment period (tick 43) issues the attack.
    let r = run(&mut w, 45);
    assert_eq!(r.rejected, 0);
    assert_eq!(
        plan(&w, 1).role_of(horse),
        Some(Role::Committed { target: velites })
    );
    assert!(r.ai.iter().any(|c| matches!(
        &c.kind,
        CommandKind::AttackRegiment { regiments, target } if regiments == &vec![horse] && *target == velites
    )));
}

/// (s) Reserves: the smallest infantry waits behind the line and is
/// committed to the enemy of the line regiment whose morale fell under
/// `commit_morale`.
#[test]
fn reserves_wait_behind_and_commit_to_the_weakest_segment() {
    let mut own = line_of(3, "rome:hastati", 120, 500.0, 180.0, 4);
    own.push(regiment(7, "rome:hastati", 40, 540.0, 180.0));
    own.push(regiment(8, "rome:hastati", 20, 700.0, 180.0));
    let mut s = setup(vec![
        side(0, 0, line_of(3, "rome:hastati", 120, 350.0, 0.0, 1)),
        side(255, 1, own),
    ]);
    s.sides[1].general.bodyguard = Some(8);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    let (weak, reserve) = (RegimentId(4), RegimentId(6));
    run(&mut w, 2);
    let p = plan(&w, 1);
    let forward = p.line_facing.direction();
    match p.role_of(reserve) {
        Some(Role::Reserve { slot }) => assert!(
            (slot - p.line_anchor).dot(forward) < -S::from_i32(50),
            "behind the line"
        ),
        other => panic!("{other:?}"),
    }
    assert!(matches!(p.role_of(weak), Some(Role::Line { .. })));
    set_morale(&mut w, weak, 30);
    let mut committed = None;
    for _ in 0..45 {
        let out = w.step(&[]);
        assert!(out.rejected.is_empty());
        set_morale(&mut w, weak, 30);
        if let Some(Role::Committed { target }) = plan(&w, 1).role_of(reserve) {
            committed = Some(target);
            break;
        }
    }
    let target = committed.expect("the reserve was committed");
    let weak_at = anchor(&w, weak);
    let nearest = w
        .view()
        .regiments()
        .filter(|r| r.side == 0)
        .min_by(|a, b| {
            a.anchor_pos
                .distance_sq(weak_at)
                .partial_cmp(&b.anchor_pos.distance_sq(weak_at))
                .unwrap()
        })
        .unwrap()
        .id;
    assert_eq!(target, nearest);
    let _ = PlayerId::ENGINE_AI;
}
