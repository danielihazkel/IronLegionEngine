//! T0-034: `hash(restore(snapshot(w))) == hash(w)` and identical hash
//! sequences after restore (SIM-DET-005).

mod common;

use il_core::{RegimentId, S, Scalar, SoldierId, Tick, V2};
use il_sim_battle::{
    AiState, ArmyPlan, Assignment, BattleWorld, Command, CommandKind, RestoreError, Role,
    SNAPSHOT_VERSION, Snapshot, Stance,
};

/// T2-080: the AI outbox and plans are stored verbatim, and a restored
/// outbox applies at the next tick exactly as the original one.
#[test]
fn ai_outbox_and_plans_survive_a_round_trip() {
    let mut original = common::world(50);
    for _ in 0..5 {
        original.step(&[]);
    }
    let next = original.tick().next();
    let regiment = original.regiment_ids().next().unwrap();
    {
        let mut ai = original.ecs_mut().resource_mut::<AiState>();
        ai.outbox.push(Command {
            tick: next,
            player: il_core::PlayerId::ENGINE_AI,
            seq: 0,
            kind: CommandKind::Move {
                regiments: vec![regiment],
                target: V2::from_f32_data(320.0, 150.0),
                facing: None,
                speed: il_sim_battle::SpeedMode::Walk,
            },
        });
        let plan = ArmyPlan {
            stance: Stance::Attack,
            stance_score: S::from_f32_data(0.7),
            decided_at: Tick(5),
            target: V2::from_f32_data(500.0, 150.0),
            line_anchor: V2::from_f32_data(300.0, 150.0),
            line_facing: il_core::Angle::default(),
            line_width: S::from_i32(60),
            formed: true,
            assignments: vec![Assignment {
                regiment,
                role: Role::Flank {
                    slot: V2::from_f32_data(1.0, 2.0),
                    charge: Some(RegimentId(1)),
                },
            }],
            charging: false,
            standoff_since: Some(Tick(3)),
            run_in: true,
        };
        ai.plans = vec![Some(plan), None];
    }
    original.recompute_hash();
    let snap = original.snapshot();
    assert_eq!(snap.ai.outbox.len(), 1);
    assert_eq!(snap.ai.plans[0].as_ref().unwrap().stance, Stance::Attack);
    let decoded = Snapshot::from_bytes(&snap.to_bytes()).unwrap();
    let mut restored = BattleWorld::restore(&decoded, common::regs()).unwrap();
    assert_eq!(restored.hash(), original.hash());
    assert_eq!(
        restored.ecs().resource::<AiState>(),
        original.ecs().resource::<AiState>()
    );
    // The outbox is fed to Stage 0 of the next tick on both sides: the move
    // is NotOwner (player 0 owns the regiment), rejected identically, and
    // the outbox is empty afterwards (no side belongs to the engine).
    let a = original.step(&[]);
    let b = restored.step(&[]);
    assert_eq!(a.hash, b.hash);
    assert_eq!(a.rejected.len(), 1);
    assert_eq!(a.rejected, b.rejected);
    assert!(a.ai_commands.is_empty() && b.ai_commands.is_empty());
    assert!(original.ecs().resource::<AiState>().outbox.is_empty());
}

#[test]
fn round_trip_preserves_hash_and_continues_identically() {
    let mut original = common::world(200);
    for _ in 0..37 {
        original.step(&[]);
    }
    let snap = original.snapshot();
    assert_eq!(snap.version, SNAPSHOT_VERSION);
    assert_eq!(snap.tick, Tick(37));
    assert_eq!(snap.soldiers.len(), 402, "400 plus two generals");
    assert_eq!(snap.regiments.len(), 2);
    assert_eq!(snap.ids.soldiers_next, 402);

    let bytes = snap.to_bytes();
    let decoded = Snapshot::from_bytes(&bytes).unwrap();
    let mut restored = BattleWorld::restore(&decoded, common::regs()).unwrap();

    assert_eq!(restored.tick(), original.tick());
    assert_eq!(restored.phase(), original.phase());
    assert_eq!(restored.soldier_count(), original.soldier_count());
    assert_eq!(
        restored.hash(),
        original.hash(),
        "hash(restore(snapshot(w))) != hash(w)"
    );
    assert_eq!(restored.setup(), original.setup());

    for tick in 0..1000 {
        let a = original.step(&[]).hash;
        let b = restored.step(&[]).hash;
        assert_eq!(a, b, "diverged at tick {}", tick + 38);
    }
    // Ids keep ascending from the snapshotted counters.
    let ids: Vec<SoldierId> = restored.soldier_ids().collect();
    assert_eq!(
        ids.last(),
        Some(&SoldierId(401)),
        "400 soldiers and two generals"
    );
    assert_eq!(restored.regiment_ids().last(), Some(RegimentId(1)));
}

/// T1-048: a regiment restored mid-path keeps following the stored path
/// and every derived structure is rebuilt, so the runs stay identical.
#[test]
fn restore_mid_march_continues_identically() {
    use il_core::{Angle, PlayerId, V2};
    use il_sim_battle::{Command, CommandKind, SpeedMode};
    let mut original = common::world(120);
    let order = Command {
        tick: Tick(1),
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::Move {
            regiments: vec![RegimentId(0)],
            target: V2::from_f32_data(300.0, 450.0),
            facing: Some(Angle::from_degrees_data(90.0)),
            speed: SpeedMode::Run,
        },
    };
    assert!(original.step(&[order]).rejected.is_empty());
    for _ in 0..600 {
        original.step(&[]);
    }
    let snap = original.snapshot();
    let mid = &snap.regiments[0];
    assert!(mid.path.len() > 1 && mid.path_next > 0, "{mid:?}");
    let mut restored = BattleWorld::restore(&snap, common::regs()).unwrap();
    assert_eq!(restored.hash(), original.hash());
    restored.set_threads(8);
    // 900 ticks: a running regiment tires (T2-040) and crosses later.
    for tick in 0..900 {
        assert_eq!(
            original.step(&[]).hash,
            restored.step(&[]).hash,
            "diverged {tick} ticks after the restore"
        );
    }
    assert!(
        original.view().regiments().next().unwrap().anchor_pos.y > S::from_i32(300),
        "crossed the river"
    );
}

#[test]
fn snapshot_of_snapshot_is_identical() {
    let mut w = common::world(50);
    w.step(&[]);
    let a = w.snapshot();
    let restored = BattleWorld::restore(&a, common::regs()).unwrap();
    let b = restored.snapshot();
    assert_eq!(a.to_bytes(), b.to_bytes());
}

#[test]
fn bad_snapshots_are_rejected() {
    let w = common::world(5);
    let mut snap = w.snapshot();
    snap.version = 99;
    assert_eq!(
        BattleWorld::restore(&snap, common::regs()).unwrap_err(),
        RestoreError::VersionMismatch {
            found: 99,
            expected: SNAPSHOT_VERSION
        }
    );
    assert!(matches!(
        Snapshot::from_bytes(&snap.to_bytes()).unwrap_err(),
        RestoreError::VersionMismatch { found: 99, .. }
    ));
    assert!(matches!(
        Snapshot::from_bytes(b"not a snapshot").unwrap_err(),
        RestoreError::Decode(_)
    ));

    let mut snap = w.snapshot();
    snap.regiments[0].unit_type = common::cid("rome:ghost");
    assert!(matches!(
        BattleWorld::restore(&snap, common::regs()).unwrap_err(),
        RestoreError::UnknownUnitType(_)
    ));

    let mut snap = w.snapshot();
    snap.soldiers[0].regiment = RegimentId(77);
    assert!(matches!(
        BattleWorld::restore(&snap, common::regs()).unwrap_err(),
        RestoreError::OrphanSoldier { .. }
    ));
}
