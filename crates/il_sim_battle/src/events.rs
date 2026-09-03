//! Battle events: the sim's only output (TDD §4.2 `BattleEvent`, ADR-005).
//! Variants are added together with the systems that emit them.

use il_core::{Event, PlayerId, RegimentId, SoldierId, V2};
use serde::{Deserialize, Serialize};

use crate::command::RejectReason;
use crate::resources::BattlePhase;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BattleEvent {
    /// A command failed validation at Stage 0 (SIM-CMD-001, SIM-CMD-003).
    CommandRejected {
        command_seq: u16,
        player: PlayerId,
        reason: RejectReason,
    },
    /// SIM-FLOW-010..017.
    PhaseChanged { from: BattlePhase, to: BattlePhase },
    /// `TransferControl` succeeded: every regiment of `from` is now `to`'s.
    ControlTransferred { from: PlayerId, to: PlayerId },
    /// No route to the order target; the order was dropped (SIM-MOVE-002).
    PathNotFound { regiment: RegimentId },
    /// Placeholder shape for Phase 2 (T2-022); keeps the variant list in
    /// the TDD order.
    SoldierDied {
        id: SoldierId,
        regiment: RegimentId,
        killer: Option<SoldierId>,
        pos: V2,
    },
}

impl Event for BattleEvent {}
