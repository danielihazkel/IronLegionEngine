//! T1-047: one test per movement command, checking the resulting `Order`,
//! `Path` and `FormationState`, plus the rejections.

mod common;

use il_core::{Angle, PlayerId, RegimentId, S, Scalar, Tick, V2};
use il_data::Layout;
use il_sim_battle::components::{Anchor, FormationState, Order, OrderKind, Path, Pos};
use il_sim_battle::resources::{Ids, Phase};
use il_sim_battle::{
    BattlePhase, BattleWorld, Command, CommandKind, PathRequests, RejectReason, SpeedMode,
};

fn v(x: f32, y: f32) -> V2 {
    V2::from_f32_data(x, y)
}

fn cmd(w: &BattleWorld, player: u8, kind: CommandKind) -> Command {
    Command {
        tick: w.tick().next(),
        player: PlayerId(player),
        seq: 0,
        kind,
    }
}

fn entity(w: &BattleWorld, rid: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap()
}

fn order(w: &BattleWorld, rid: u32) -> Order {
    *w.ecs().get::<Order>(entity(w, rid)).unwrap()
}

fn state(w: &BattleWorld, rid: u32) -> FormationState {
    w.ecs()
        .get::<FormationState>(entity(w, rid))
        .unwrap()
        .clone()
}

fn layout(w: &BattleWorld, rid: u32) -> Layout {
    w.registries().formations.get(state(w, rid).template).layout
}

#[test]
fn move_sets_the_order_requests_a_path_and_reforms() {
    let mut w = common::world(20);
    let target = v(350.0, 200.0);
    let out = w.step(&[cmd(
        &w,
        0,
        CommandKind::Move {
            regiments: vec![RegimentId(0)],
            target,
            facing: Some(Angle::from_degrees_data(90.0)),
            speed: SpeedMode::Run,
        },
    )]);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    let o = order(&w, 0);
    assert_eq!(o.kind, OrderKind::Move);
    assert_eq!(o.target, target);
    assert_eq!(o.speed, SpeedMode::Run);
    assert_eq!(o.since, Tick(1));
    assert_eq!(o.facing, Some(Angle::from_degrees_data(90.0)));
    // Served at Stage 3 of the same tick.
    let path = w.ecs().get::<Path>(entity(&w, 0)).unwrap();
    assert!(path.is_active() && !path.requested, "{path:?}");
    assert!(w.ecs().resource::<PathRequests>().0.is_empty());
    // Off-map targets are clamped, not rejected.
    let out = w.step(&[cmd(
        &w,
        1,
        CommandKind::AttackMove {
            regiments: vec![RegimentId(1)],
            target: v(-50.0, 9_999.0),
        },
    )]);
    assert!(out.rejected.is_empty());
    let o = order(&w, 1);
    assert_eq!(o.kind, OrderKind::AttackMove);
    assert_eq!(o.target, v(0.0, 600.0));
}

#[test]
fn halt_ends_the_order_and_drops_the_path() {
    let mut w = common::world(20);
    w.step(&[cmd(
        &w,
        0,
        CommandKind::Move {
            regiments: vec![RegimentId(0)],
            target: v(350.0, 200.0),
            facing: None,
            speed: SpeedMode::Walk,
        },
    )]);
    for _ in 0..10 {
        w.step(&[]);
    }
    let moved = w.ecs().get::<Anchor>(entity(&w, 0)).unwrap().pos;
    assert_ne!(moved, v(300.0, 150.0));
    let out = w.step(&[cmd(
        &w,
        0,
        CommandKind::Halt {
            regiments: vec![RegimentId(0)],
        },
    )]);
    assert!(out.rejected.is_empty());
    assert_eq!(order(&w, 0).kind, OrderKind::Idle);
    assert!(!w.ecs().get::<Path>(entity(&w, 0)).unwrap().is_active());
    let stopped = w.ecs().get::<Anchor>(entity(&w, 0)).unwrap().pos;
    w.step(&[]);
    assert_eq!(w.ecs().get::<Anchor>(entity(&w, 0)).unwrap().pos, stopped);
}

#[test]
fn set_formation_morphs_and_validates_the_template() {
    let mut w = common::world(20);
    let out = w.step(&[cmd(
        &w,
        0,
        CommandKind::SetFormation {
            regiments: vec![RegimentId(0)],
            template: common::cid("rome:column"),
            ranks: None,
        },
    )]);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    let s = state(&w, 0);
    assert_eq!(layout(&w, 0), Layout::Column);
    assert_eq!(s.morph_until, Tick(1 + 60), "rome:column morph_ticks");
    assert!(!s.needs_reform, "laid out at Stage 2 of the same tick");
    assert_eq!(s.files, 4);
    // Explicit ranks are clamped to the template.
    w.step(&[cmd(
        &w,
        0,
        CommandKind::SetFormation {
            regiments: vec![RegimentId(0)],
            template: common::cid("rome:line"),
            ranks: Some(40),
        },
    )]);
    assert_eq!(state(&w, 0).ranks, 16, "rome:line max_ranks");
    // Hastati may not form a phalanx; unknown templates are unknown content.
    let out = w.step(&[
        cmd(
            &w,
            0,
            CommandKind::SetFormation {
                regiments: vec![RegimentId(0)],
                template: common::cid("rome:phalanx"),
                ranks: None,
            },
        ),
        Command {
            seq: 1,
            ..cmd(
                &w,
                0,
                CommandKind::SetFormation {
                    regiments: vec![RegimentId(0)],
                    template: common::cid("rome:testudo"),
                    ranks: None,
                },
            )
        },
    ]);
    assert_eq!(out.rejected.len(), 2);
    assert_eq!(
        out.rejected[0].1,
        RejectReason::FormationNotAllowed {
            regiment: RegimentId(0),
            template: common::cid("rome:phalanx")
        }
    );
    assert_eq!(
        out.rejected[1].1,
        RejectReason::UnknownContent(common::cid("rome:testudo"))
    );
    assert_eq!(layout(&w, 0), Layout::Line);
}

#[test]
fn set_facing_wheels_or_about_faces_and_speed_mode_sticks() {
    let mut w = common::world(20);
    let out = w.step(&[
        cmd(
            &w,
            0,
            CommandKind::SetFacing {
                regiments: vec![RegimentId(0)],
                facing: Angle::from_degrees_data(60.0),
            },
        ),
        Command {
            seq: 1,
            ..cmd(
                &w,
                0,
                CommandKind::SetSpeedMode {
                    regiments: vec![RegimentId(0)],
                    mode: SpeedMode::March,
                },
            )
        },
    ]);
    assert!(out.rejected.is_empty());
    let o = order(&w, 0);
    assert_eq!(o.facing, Some(Angle::from_degrees_data(60.0)));
    assert_eq!(o.speed, SpeedMode::March);
    assert_eq!(o.kind, OrderKind::Idle);
    let f = w.ecs().get::<Anchor>(entity(&w, 0)).unwrap().facing;
    assert!(
        f.radians() > S::ZERO && f.radians() < S::from_f32_data(0.1),
        "wheeling: {f:?}"
    );
    // 180 degrees: about-face (regiment 1 faces west; order east).
    let before = w.ecs().get::<Anchor>(entity(&w, 1)).unwrap().pos;
    w.step(&[cmd(
        &w,
        1,
        CommandKind::SetFacing {
            regiments: vec![RegimentId(1)],
            facing: Angle::ZERO,
        },
    )]);
    let a = *w.ecs().get::<Anchor>(entity(&w, 1)).unwrap();
    assert_eq!(a.facing, Angle::ZERO);
    assert!(a.pos.x > before.x + S::from_i32(2), "{a:?} vs {before:?}");
}

#[test]
fn group_formation_places_every_regiment_and_moves_them() {
    let mut w = common::world(40);
    // Both regiments belong to different players; transfer so player 0
    // owns them all, then arrange a battle line 60 m wide facing north.
    w.step(&[cmd(
        &w,
        1,
        CommandKind::TransferControl {
            from: PlayerId(1),
            to: PlayerId(0),
        },
    )]);
    let out = w.step(&[cmd(
        &w,
        0,
        CommandKind::GroupFormation {
            regiments: vec![RegimentId(0), RegimentId(1)],
            template: common::cid("rome:battle_line"),
            anchor: v(400.0, 300.0),
            facing: Angle::from_degrees_data(90.0),
            width: S::from_i32(60),
        },
    )]);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    for rid in 0..2 {
        let o = order(&w, rid);
        assert_eq!(o.kind, OrderKind::Move);
        assert_eq!(o.facing, Some(Angle::from_degrees_data(90.0)));
        assert!((o.target.y - S::from_i32(300)).abs() < S::from_f32_data(1e-3));
    }
    // Regiment 0 started west of regiment 1 and stays west.
    assert!(order(&w, 0).target.x < order(&w, 1).target.x);
    // 40 men at 60 m for two regiments: two ranks each (20 files x 0.8 m).
    assert_eq!(state(&w, 0).ranks, 2);
    let out = w.step(&[cmd(
        &w,
        0,
        CommandKind::GroupFormation {
            regiments: vec![RegimentId(0)],
            template: common::cid("rome:phalanx_wall"),
            anchor: v(400.0, 300.0),
            facing: Angle::ZERO,
            width: S::from_i32(60),
        },
    )]);
    assert_eq!(
        out.rejected[0].1,
        RejectReason::UnknownContent(common::cid("rome:phalanx_wall"))
    );
}

#[test]
fn deploy_needs_the_deployment_phase_and_then_teleports() {
    let mut w = common::world(20);
    let deploy = |w: &BattleWorld| {
        cmd(
            w,
            0,
            CommandKind::Deploy {
                regiment: RegimentId(0),
                position: v(200.0, 100.0),
                facing: Angle::from_degrees_data(90.0),
                template: None,
            },
        )
    };
    let out = w.step(&[deploy(&w)]);
    assert_eq!(out.rejected[0].1, RejectReason::WrongPhase);
    w.ecs_mut().resource_mut::<Phase>().0 = BattlePhase::Deployment;
    w.recompute_hash();
    let out = w.step(&[deploy(&w)]);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    let a = *w.ecs().get::<Anchor>(entity(&w, 0)).unwrap();
    assert_eq!(a.pos, v(200.0, 100.0));
    assert_eq!(a.facing, Angle::from_degrees_data(90.0));
    // Every soldier stands within a slot's reach of the new anchor.
    let ids = w.ecs().resource::<Ids>().soldier_entities.clone();
    let regiment = w
        .ecs()
        .get::<il_sim_battle::components::Regiment>(entity(&w, 0))
        .unwrap()
        .soldiers
        .clone();
    for (sid, e) in ids {
        if regiment.contains(&sid) {
            let p = w.ecs().get::<Pos>(e).unwrap().p;
            assert!(p.distance(a.pos) < S::from_i32(15), "{p:?}");
        }
    }
}

#[test]
fn routing_regiments_refuse_orders() {
    let mut w = common::world(20);
    let e = entity(&w, 0);
    w.ecs_mut()
        .get_mut::<il_sim_battle::components::Morale>(e)
        .unwrap()
        .state = il_sim_battle::components::MoraleState::Routing;
    w.recompute_hash();
    let out = w.step(&[cmd(
        &w,
        0,
        CommandKind::Halt {
            regiments: vec![RegimentId(0)],
        },
    )]);
    assert_eq!(out.rejected[0].1, RejectReason::Routing(RegimentId(0)));
}
