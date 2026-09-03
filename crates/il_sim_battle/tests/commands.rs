//! T0-031: Stage 0 command validation and application
//! (SIM-CMD-001, SIM-CMD-003, SIM-DET-008).

mod common;

use il_core::{PlayerId, RegimentId, Tick};
use il_sim_battle::{BattleEvent, Command, CommandKind, RejectReason};

fn cmd(tick: Tick, player: u8, seq: u16, kind: CommandKind) -> Command {
    Command {
        tick,
        player: PlayerId(player),
        seq,
        kind,
    }
}

fn halt(regiment: u32) -> CommandKind {
    CommandKind::Halt {
        regiments: vec![RegimentId(regiment)],
    }
}

#[test]
fn commands_apply_in_player_then_seq_order_regardless_of_arrival() {
    let mut w = common::world(10);
    let t = w.tick().next();
    // Player 0 hands its side to player 5 at seq 1, then (seq 2) halts its
    // regiment. Applied in seq order the halt is refused; applied in arrival
    // order it would succeed.
    let out = w.step(&[
        cmd(t, 0, 2, halt(0)),
        cmd(
            t,
            0,
            1,
            CommandKind::TransferControl {
                from: PlayerId(0),
                to: PlayerId(5),
            },
        ),
    ]);
    assert_eq!(out.rejected.len(), 1, "{:?}", out.rejected);
    assert_eq!(out.rejected[0].0.seq, 2);
    assert_eq!(out.rejected[0].1, RejectReason::NotOwner(RegimentId(0)));
    assert!(out.events.iter().any(|e| matches!(
        e,
        BattleEvent::ControlTransferred {
            from: PlayerId(0),
            to: PlayerId(5)
        }
    )));

    // Across players: player 0 sorts before player 1 whatever the seq numbers.
    let t = w.tick().next();
    let out = w.step(&[
        cmd(t, 1, 0, CommandKind::Surrender),
        cmd(t, 0, 9, CommandKind::Surrender),
    ]);
    let players: Vec<u8> = out.rejected.iter().map(|(c, _)| c.player.0).collect();
    assert_eq!(players, vec![0, 1]);
}

#[test]
fn stale_commands_are_rejected_with_an_event() {
    let mut w = common::world(10);
    w.step(&[]);
    w.step(&[]);
    let current = w.tick().next(); // 3
    let out = w.step(&[
        cmd(Tick(1), 0, 0, halt(0)),
        cmd(Tick(99), 0, 1, halt(0)),
        cmd(current, 0, 2, halt(0)),
    ]);
    assert_eq!(out.rejected.len(), 2);
    assert_eq!(
        out.rejected[0].1,
        RejectReason::StaleTick {
            command_tick: Tick(1),
            current
        }
    );
    assert_eq!(
        out.rejected[1].1,
        RejectReason::StaleTick {
            command_tick: Tick(99),
            current
        }
    );
    let rejected_events = out
        .events
        .iter()
        .filter(|e| matches!(e, BattleEvent::CommandRejected { .. }))
        .count();
    assert_eq!(rejected_events, 2);
}

#[test]
fn ownership_is_enforced_and_transfer_changes_it() {
    let mut w = common::world(10);
    let t = w.tick().next();
    let out = w.step(&[
        cmd(t, 1, 0, halt(0)), // side 0 belongs to player 0
        cmd(t, 1, 1, halt(1)), // own regiment
        cmd(t, 0, 0, halt(7)), // no such regiment
    ]);
    assert_eq!(out.rejected.len(), 2);
    assert_eq!(
        out.rejected[0].1,
        RejectReason::UnknownRegiment(RegimentId(7))
    );
    assert_eq!(out.rejected[1].1, RejectReason::NotOwner(RegimentId(0)));

    // Drop-to-AI: player 1 hands over to the engine, which can then command.
    let t = w.tick().next();
    let out = w.step(&[
        cmd(
            t,
            1,
            2,
            CommandKind::TransferControl {
                from: PlayerId(1),
                to: PlayerId::ENGINE_AI,
            },
        ),
        cmd(t, 255, 0, halt(1)),
    ]);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    let t = w.tick().next();
    let out = w.step(&[cmd(t, 1, 3, halt(1))]);
    assert_eq!(out.rejected[0].1, RejectReason::NotOwner(RegimentId(1)));

    // Only the owner (or the engine) may transfer.
    let t = w.tick().next();
    let out = w.step(&[cmd(
        t,
        1,
        4,
        CommandKind::TransferControl {
            from: PlayerId(0),
            to: PlayerId(1),
        },
    )]);
    assert_eq!(out.rejected.len(), 1);
}

#[test]
fn unimplemented_variants_are_rejected_not_dropped() {
    let mut w = common::world(10);
    let t = w.tick().next();
    let out = w.step(&[
        cmd(
            t,
            0,
            0,
            CommandKind::FireMode {
                regiments: vec![RegimentId(0)],
                mode: il_sim_battle::FireMode::Hold,
            },
        ),
        cmd(t, 0, 1, CommandKind::ConfirmDeployment),
        cmd(t, 0, 2, CommandKind::Surrender),
    ]);
    assert_eq!(out.rejected.len(), 3);
    assert!(
        out.rejected
            .iter()
            .all(|(_, r)| *r == RejectReason::NotImplemented)
    );
}

#[test]
fn pause_and_set_speed_are_accepted_no_ops() {
    let mut a = common::world(10);
    let mut b = common::world(10);
    let t = a.tick().next();
    let out = a.step(&[
        cmd(t, 0, 0, CommandKind::Pause),
        cmd(t, 0, 1, CommandKind::SetSpeed { mult_x100: 200 }),
        cmd(t, 0, 2, halt(0)),
    ]);
    assert!(out.rejected.is_empty());
    assert!(out.events.is_empty());
    // SIM-DET-008: no effect on the state hash.
    assert_eq!(out.hash, b.step(&[]).hash);
}
