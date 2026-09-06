//! T2-071: `BattleWorld::result` (SIM-FLOW-018). Winner on annihilation and
//! on the timer, experience points, ammo, loot, the general fates through
//! the result, pending reinforcements, and `winner` before the end.

mod common;

use common::cid;
use il_core::{PlayerId, RegimentId, S, Scalar, Tick};
use il_data::MapEdge;
use il_sim_battle::components::{Health, Morale, MoraleState};
use il_sim_battle::resources::{BattleFlow, Ids, Phase, Sides};
use il_sim_battle::{
    BattlePhase, BattleSetup, BattleWorld, Command, CommandKind, FireMode, GeneralFate,
    RegimentSetup, ReinforcementGroup, SideSetup,
};

fn reg(id: u32, unit: &str, count: u16, x: f32, y: f32, deg: f32) -> RegimentSetup {
    RegimentSetup {
        id,
        unit_type: cid(unit),
        count,
        experience: 0,
        fatigue: 0.0,
        formation: Some(cid("rome:line")),
        position: Some([x, y]),
        facing_deg: Some(deg),
    }
}

fn side(player: u8, regiments: Vec<RegimentSetup>) -> SideSetup {
    SideSetup {
        deployment_zone: player,
        ..common::side(player, regiments)
    }
}

fn setup(sides: Vec<SideSetup>) -> BattleSetup {
    BattleSetup {
        map_id: cid("rome:test_field"),
        seed: 5,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides,
        victory: Default::default(),
    }
}

fn command(tick: u32, player: u8, seq: u16, kind: CommandKind) -> Command {
    Command {
        tick: Tick(tick),
        player: PlayerId(player),
        seq,
        kind,
    }
}

fn run(w: &mut BattleWorld, commands: &[Command], until: u32) {
    while w.tick().0 < until {
        let next = w.tick().next();
        let due: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let out = w.step(&due);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    }
}

#[test]
fn a_fresh_battle_has_no_winner_and_full_ammo() {
    let s = setup(vec![
        side(0, vec![reg(1, "rome:velites", 20, 300.0, 150.0, 0.0)]),
        side(1, vec![reg(2, "rome:hastati", 20, 335.0, 150.0, 180.0)]),
    ]);
    let w = BattleWorld::new(&s, common::regs()).unwrap();
    let r = w.result();
    assert_eq!(r.winner, None);
    assert_eq!(r.duration_ticks, 0);
    assert_eq!(r.sides.len(), 2);
    let v = &r.sides[0].regiments[0];
    assert_eq!(
        (v.id, v.initial, v.survivors, v.killed, v.fled),
        (1, 21, 21, 0, 0)
    );
    assert_eq!(
        v.ammo_left,
        20 * 8,
        "eight javelins each; the general throws nothing"
    );
    assert_eq!(r.sides[1].regiments[0].ammo_left, 20 * 2, "two pila each");
    assert!(v.arrived);
    assert_eq!(v.experience_gain, 1, "surviving so far is worth a point");
    assert_eq!(r.sides[0].loot, 0);
    assert_eq!(r.sides[0].general_fate, GeneralFate::Alive);
}

#[test]
fn annihilation_names_the_winner_and_reconciles_every_count() {
    let s = setup(vec![
        side(0, vec![reg(1, "rome:hastati", 120, 300.0, 150.0, 0.0)]),
        side(1, vec![reg(2, "rome:velites", 12, 330.0, 150.0, 180.0)]),
    ]);
    let regs = common::regs();
    let mut w = BattleWorld::new(&s, regs.clone()).unwrap();
    let commands = [
        command(
            1,
            0,
            0,
            CommandKind::FireMode {
                regiments: vec![RegimentId(0)],
                mode: FireMode::Hold,
            },
        ),
        command(
            1,
            0,
            1,
            CommandKind::AttackRegiment {
                regiments: vec![RegimentId(0)],
                target: RegimentId(1),
            },
        ),
    ];
    let mut t = 0;
    while w.phase() != BattlePhase::Ended && t < 8_000 {
        t += 20;
        run(&mut w, &commands, t);
    }
    assert_eq!(w.phase(), BattlePhase::Ended, "the velites never went");
    let r = w.result();
    assert_eq!(r.winner, Some(0));
    // Frozen at the end; the loop above overshoots by up to twenty ticks.
    assert!(r.duration_ticks > 0 && r.duration_ticks <= w.tick().0);
    assert_eq!(r.duration_ticks, w.view().flow().ended_at.0);
    for side in &r.sides {
        for x in &side.regiments {
            assert_eq!(x.initial, x.survivors + x.fled + x.killed, "{x:?}");
        }
    }
    let loser = &r.sides[1].regiments[0];
    assert_eq!(loser.survivors, 0);
    assert_eq!(loser.fled + loser.killed, 13);
    assert_eq!(
        loser.experience_gain, 0,
        "nobody survived to bank the point"
    );
    let winner = &r.sides[0].regiments[0];
    assert!(winner.survivors >= 100, "{winner:?}");
    assert_eq!(
        r.sides[0].loot,
        i64::from(loser.killed) * 10,
        "loot_per_enemy_killed 10 for the winner"
    );
    assert_eq!(r.sides[1].loot, 0);
    assert_eq!(
        r.summary.total_killed,
        u32::from(loser.killed + winner.killed)
    );
    assert_eq!(r.summary.total_fled, u32::from(loser.fled + winner.fled));
    let kills = w
        .ecs()
        .get::<il_sim_battle::components::Combat>(
            w.ecs()
                .resource::<Ids>()
                .regiment_entity(RegimentId(0))
                .unwrap(),
        )
        .unwrap()
        .kills;
    let expected = (0.01 * f64::from(kills) + 1.0).floor() as u16;
    assert_eq!(winner.experience_gain, expected);
}

#[test]
fn the_timer_verdict_flows_into_the_result_with_a_pending_group() {
    let mut s = setup(vec![
        side(0, vec![reg(1, "rome:hastati", 30, 300.0, 150.0, 0.0)]),
        side(1, vec![reg(2, "rome:hastati", 10, 500.0, 150.0, 180.0)]),
    ]);
    s.time_limit_ticks = 40;
    s.victory.timeout_winner = Some(1);
    s.sides[0].reinforcements = vec![ReinforcementGroup {
        arrival_tick: 10_000,
        edge: MapEdge::South,
        regiments: vec![reg(9, "rome:velites", 25, 0.0, 0.0, 0.0)],
    }];
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    run(&mut w, &[], 45);
    assert_eq!(w.phase(), BattlePhase::Ended);
    let r = w.result();
    assert_eq!(r.winner, Some(1));
    assert_eq!(r.duration_ticks, 40);
    let pending = r.sides[0]
        .regiments
        .iter()
        .find(|x| x.id == 9)
        .expect("the pending group is listed");
    assert!(!pending.arrived);
    assert_eq!(
        (
            pending.initial,
            pending.survivors,
            pending.killed,
            pending.fled
        ),
        (25, 25, 0, 0)
    );
    assert_eq!(
        r.sides[0].general_fate,
        GeneralFate::Alive,
        "the loser's general, unshattered"
    );
    assert_eq!(r.sides[1].loot, 0, "nobody died");
}

#[test]
fn general_fates_reach_the_result() {
    let s = setup(vec![
        side(0, vec![reg(1, "rome:hastati", 20, 300.0, 150.0, 0.0)]),
        side(1, vec![reg(2, "rome:hastati", 20, 500.0, 150.0, 180.0)]),
    ]);
    let mut w = BattleWorld::new(&s, common::regs()).unwrap();
    // Side 1's general is dead, side 0's is wounded and its bodyguard
    // shattered; side 1 wins, so side 0's general is captured.
    {
        let general0 = w.ecs().resource::<Sides>().0[0].general.unwrap();
        let ge = w.ecs().resource::<Ids>().soldier_entity(general0).unwrap();
        let ecs = w.ecs_mut();
        ecs.get_mut::<Health>(ge).unwrap().hp = S::from_i32(10);
        ecs.resource_mut::<Sides>().0[1].general_dead = true;
        let re = ecs
            .resource::<Ids>()
            .regiment_entity(RegimentId(0))
            .unwrap();
        ecs.get_mut::<Morale>(re).unwrap().state = MoraleState::Shattered;
    }
    w.recompute_hash();
    let r = w.result();
    assert_eq!(r.winner, None);
    assert_eq!(
        r.sides[0].general_fate,
        GeneralFate::Wounded,
        "no loser yet"
    );
    assert_eq!(r.sides[1].general_fate, GeneralFate::Dead);
    {
        let ecs = w.ecs_mut();
        ecs.resource_mut::<Phase>().0 = BattlePhase::Ended;
        ecs.resource_mut::<BattleFlow>().winner = Some(1);
    }
    w.recompute_hash();
    let r = w.result();
    assert_eq!(r.winner, Some(1));
    assert_eq!(r.sides[0].general_fate, GeneralFate::Captured);
    assert_eq!(r.sides[1].general_fate, GeneralFate::Dead);
}
