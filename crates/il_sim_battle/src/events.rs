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
    /// A soldier of the regiment entered `Fighting` while none was
    /// (SIM-CMBT-003, T2-020).
    Engaged { regiment: RegimentId },
    /// A running regiment made contact and opened its charge window
    /// (SIM-CMBT-015, T2-021); `target` is the regiment its first fighter
    /// struck.
    Charge {
        regiment: RegimentId,
        target: RegimentId,
    },
    /// A soldier died at Stage 15 (T2-022); `pos` feeds the render-only
    /// corpse (SIM-CORE-008).
    SoldierDied {
        id: SoldierId,
        regiment: RegimentId,
        killer: Option<SoldierId>,
        pos: V2,
    },
}

impl Event for BattleEvent {}
