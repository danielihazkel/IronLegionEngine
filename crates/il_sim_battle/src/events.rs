//! Battle events: the sim's only output (TDD §4.2 `BattleEvent`, ADR-005).
//! Variants are added together with the systems that emit them.

use il_core::{Event, PlayerId, RegimentId, SoldierId, V2};
use il_data::ContentId;
use serde::{Deserialize, Serialize};

use crate::command::RejectReason;
use crate::components::MoraleState;
use crate::interface::BattleResult;
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
    /// `count` soldiers of the regiment threw this tick (SIM-PROJ-003,
    /// T2-030); the statistical path emits it too (REQ-CMBT-015).
    VolleyFired { regiment: RegimentId, count: u16 },
    /// A projectile reached its landing point (SIM-PROJ-006, T2-031);
    /// `victim` is the soldier it struck, if any. Never emitted by the
    /// statistical path, which has no visible projectile.
    ProjectileLanded {
        pos: V2,
        hit: bool,
        victim: Option<SoldierId>,
    },
    /// Direct fire was refused this tick because `blocker`, a friendly
    /// regiment, lay in the line of fire (SIM-PROJ-009); the mode is
    /// unchanged and fire resumes when the line clears.
    FireBlocked {
        regiment: RegimentId,
        blocker: RegimentId,
    },
    /// The regiment's morale state changed at Stage 14 (SIM-MOR-003,
    /// T2-041); `to == Routing` is the rout (SIM-MOR-030).
    MoraleChanged {
        regiment: RegimentId,
        from: MoraleState,
        to: MoraleState,
    },
    /// A Routing regiment rallied into Shaken (SIM-MOR-031, T2-042).
    Rallied { regiment: RegimentId },
    /// The regiment shattered and leaves the field (SIM-MOR-032, T2-042).
    Shattered { regiment: RegimentId },
    /// A routing or withdrawing soldier reached its side's escape edge and
    /// left the battle (SIM-FLOW-002, T2-042); no corpse.
    SoldierFled {
        id: SoldierId,
        regiment: RegimentId,
        pos: V2,
    },
    /// The side's general died (SIM-GEN-003, T2-043): the aura is gone and
    /// every regiment of the side takes the death shock next tick.
    GeneralDied { side: u8, soldier: SoldierId },
    /// `UseAbility` succeeded at Stage 0 (SIM-ABIL-003, T2-050); `targets`
    /// counts the regiments that received the status.
    AbilityUsed {
        regiment: RegimentId,
        ability: ContentId,
        targets: u8,
    },
    /// A status effect ran out at Stage 12 (SIM-ABIL-004, T2-050).
    StatusExpired {
        regiment: RegimentId,
        ability: ContentId,
    },
    /// A side confirmed its deployment (SIM-FLOW-011, T2-070).
    DeploymentConfirmed { side: u8 },
    /// A side surrendered (SIM-FLOW-017, T2-070).
    Surrendered { side: u8 },
    /// The regiment was ordered to withdraw (SIM-FLOW-014, T2-070).
    Withdrawing { regiment: RegimentId },
    /// A withdrawing soldier reached its side's escape edge and left the
    /// battle as a survivor (SIM-FLOW-014, T2-070).
    SoldierWithdrew {
        id: SoldierId,
        regiment: RegimentId,
        pos: V2,
    },
    /// A reinforcement group entered at its edge (SIM-FLOW-016, T2-070).
    ReinforcementsArrived { side: u8 },
    /// A reinforcement group was dropped at the soldier cap (SIM-CORE-006).
    ReinforcementsDropped { side: u8, group: u8 },
    /// The battle ended (SIM-FLOW-013/015/018, T2-070); the result as of
    /// that tick.
    Ended { result: Box<BattleResult> },
}

impl Event for BattleEvent {}
