//! T0-034: `hash(restore(snapshot(w))) == hash(w)` and identical hash
//! sequences after restore (SIM-DET-005).

mod common;

use il_core::{RegimentId, S, Scalar, SoldierId, Tick};
use il_sim_battle::{BattleWorld, RestoreError, SNAPSHOT_VERSION, Snapshot};

#[test]
fn round_trip_preserves_hash_and_continues_identically() {
    let mut original = common::world(200);
    for _ in 0..37 {
        original.step(&[]);
    }
    let snap = original.snapshot();
    assert_eq!(snap.version, SNAPSHOT_VERSION);
    assert_eq!(snap.tick, Tick(37));
    assert_eq!(snap.soldiers.len(), 400);
    assert_eq!(snap.regiments.len(), 2);
    assert_eq!(snap.ids.soldiers_next, 400);

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
    assert_eq!(ids.last(), Some(&SoldierId(399)));
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
    for tick in 0..400 {
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
