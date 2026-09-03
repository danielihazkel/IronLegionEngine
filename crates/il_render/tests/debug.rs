//! T1-054: each overlay draws from an immutable `BattleView` and adds
//! segments only when its flag is on.

use std::path::Path;
use std::sync::Arc;

use glam::Vec2;
use il_core::{Angle, PlayerId, RegimentId, Tick, V2};
use il_data::Registries;
use il_render::{Camera, DebugFlags, LineScene, build_debug_lines};
use il_sim_battle::{BattleWorld, Command, CommandKind, SpeedMode};

const SCREEN: Vec2 = Vec2::new(1280.0, 800.0);

fn world() -> BattleWorld {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
    let regs = Arc::new(Registries::load_root(&root).expect("game content loads"));
    let scenario: il_sim_battle::Scenario = json5::from_str(
        r#"{
          map_id: "rome:test_field", seed: 7,
          sides: [
            { faction: "rome:rome", player: 0, deployment_zone: 0,
              general: { unit_type: "rome:hastati", name_key: "g0" },
              regiments: [ { id: 1, unit_type: "rome:hastati", count: 40, position: [300, 150], facing_deg: 90 } ] },
          ],
        }"#,
    )
    .expect("scenario parses");
    let mut world = BattleWorld::new(&scenario.setup, regs).expect("world builds");
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
    assert!(world.step(&[order]).rejected.is_empty());
    world
}

fn segments(world: &BattleWorld, flags: DebugFlags, camera: &Camera) -> usize {
    let mut lines = LineScene::default();
    build_debug_lines(&world.view(), flags, camera, SCREEN, &mut lines);
    lines.segment_count()
}

#[test]
fn every_toggle_adds_segments_from_the_view() {
    let world = world();
    // Over the regiment (slots, anchors, cells) and over the river (the
    // nav grid draws only impassable and costly cells).
    let near = Camera::new(Vec2::new(300.0, 170.0));
    let river = Camera::new(Vec2::new(400.0, 300.0));
    let none = DebugFlags::default();
    assert_eq!(segments(&world, none, &near), 0);
    assert_eq!(segments(&world, none, &river), 0);
    let flags = [
        (
            DebugFlags {
                nav_grid: true,
                ..none
            },
            &river,
        ),
        (
            DebugFlags {
                slots: true,
                ..none
            },
            &near,
        ),
        (
            DebugFlags {
                paths: true,
                ..none
            },
            &near,
        ),
        (
            DebugFlags {
                anchors: true,
                ..none
            },
            &near,
        ),
        (
            DebugFlags {
                spatial_cells: true,
                ..none
            },
            &near,
        ),
    ];
    let mut sum = 0;
    for (f, camera) in flags {
        let n = segments(&world, f, camera);
        assert!(n > 0, "{f:?} drew nothing");
        if std::ptr::eq(camera, &near) {
            sum += n;
        }
    }
    let all_near = DebugFlags {
        slots: true,
        paths: true,
        anchors: true,
        spatial_cells: true,
        ..none
    };
    assert_eq!(segments(&world, all_near, &near), sum);
    // The path crosses the bridge, whose corridor is narrower than the
    // 10-file line: the narrow marker (a 4-chord circle) is drawn.
    let only_paths = DebugFlags {
        paths: true,
        ..none
    };
    assert!(segments(&world, only_paths, &near) >= 4 + 2);
    // Open ground draws no nav cells; a regiment off screen draws no
    // slots or anchors.
    assert_eq!(
        segments(
            &world,
            DebugFlags {
                nav_grid: true,
                ..none
            },
            &near
        ),
        0
    );
    let far = Camera::new(Vec2::new(750.0, 550.0));
    assert_eq!(
        segments(
            &world,
            DebugFlags {
                slots: true,
                anchors: true,
                ..none
            },
            &far
        ),
        0
    );
}
