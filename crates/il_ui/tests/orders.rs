//! Intent → Command conversion and the T1-062 done-when, headless: the
//! ten regiments of `move_reform_2000` are dragged into a battle line and
//! end up side by side, facing the drag direction. Regiment ids are the
//! spawn order (0..10).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec2;
use il_core::{Angle, PlayerId, RegimentId, S, Scalar, Tick};
use il_sim_battle::{BattleWorld, Command, CommandKind, Scenario, SpeedMode};
use il_ui::{OrderContext, UiIntent, commands_for, drag_formation, selection_centroid};

fn world() -> BattleWorld {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let regs =
        Arc::new(il_data::load_roots(&[root.join("game")]).unwrap_or_else(|e| panic!("{e}")));
    let text = std::fs::read_to_string(root.join("tests/scenarios/move_reform_2000.json5"))
        .expect("scenario readable");
    let scenario: Scenario = json5::from_str(&text).expect("scenario parses");
    let mut world = BattleWorld::new(&scenario.setup, regs).expect("world builds");
    world.set_threads(8);
    world
}

fn all(view: &il_sim_battle::BattleView) -> BTreeSet<RegimentId> {
    view.regiments().map(|r| r.id).collect()
}

fn stamp(world: &BattleWorld, kinds: Vec<CommandKind>) -> Vec<Command> {
    kinds
        .into_iter()
        .enumerate()
        .map(|(seq, kind)| Command {
            tick: world.tick().next(),
            player: PlayerId(0),
            seq: seq as u16,
            kind,
        })
        .collect()
}

fn f(v: S) -> f32 {
    v.to_f32_render()
}

#[test]
fn click_move_halt_run_and_formation_hotkeys_become_commands() {
    let world = world();
    let view = world.view();
    let two: BTreeSet<RegimentId> = [RegimentId(0), RegimentId(6)].into_iter().collect();
    let ctx = OrderContext {
        view: &view,
        regiments: &two,
        speed: SpeedMode::Run,
    };
    let mv = commands_for(
        &UiIntent::Move {
            target: Vec2::new(300.0, 400.0),
        },
        &ctx,
    );
    assert!(matches!(
        &mv[..],
        [CommandKind::Move { regiments, facing: None, speed: SpeedMode::Run, target }]
            if regiments == &[RegimentId(0), RegimentId(6)] && (f(target.x) - 300.0).abs() < 1e-3
    ));
    assert!(matches!(
        &commands_for(&UiIntent::Halt, &ctx)[..],
        [CommandKind::Halt { regiments }] if regiments.len() == 2
    ));
    assert!(matches!(
        &commands_for(&UiIntent::SpeedMode(SpeedMode::Walk), &ctx)[..],
        [CommandKind::SetSpeedMode {
            mode: SpeedMode::Walk,
            ..
        }]
    ));
    // Hastati (regiment 0) and hoplites (regiment 6) have different template
    // lists, so the hotkey yields one SetFormation per template, ranks unset.
    let f2 = commands_for(&UiIntent::Formation(2), &ctx);
    assert_eq!(f2.len(), 2, "{f2:?}");
    for c in &f2 {
        assert!(
            matches!(c, CommandKind::SetFormation { ranks: None, regiments, .. } if regiments.len() == 1)
        );
    }
    // A hotkey past a type's list skips those regiments.
    assert!(commands_for(&UiIntent::Formation(9), &ctx).is_empty());
    // Nothing selected, nothing emitted.
    let none = BTreeSet::new();
    let empty = OrderContext {
        view: &view,
        regiments: &none,
        speed: SpeedMode::Walk,
    };
    assert!(commands_for(&UiIntent::Halt, &empty).is_empty());
}

#[test]
fn a_single_regiment_drag_sets_ranks_for_the_width_then_moves() {
    let world = world();
    let view = world.view();
    let one: BTreeSet<RegimentId> = [RegimentId(0)].into_iter().collect();
    let ctx = OrderContext {
        view: &view,
        regiments: &one,
        speed: SpeedMode::Walk,
    };
    let centroid = selection_centroid(&view, &one).unwrap();
    let wide = drag_formation(
        centroid + Vec2::new(-60.0, 100.0),
        centroid + Vec2::new(60.0, 100.0),
        centroid,
        false,
    )
    .unwrap();
    let narrow = drag_formation(
        centroid + Vec2::new(-10.0, 100.0),
        centroid + Vec2::new(10.0, 100.0),
        centroid,
        false,
    )
    .unwrap();
    let wide_cmds = commands_for(&UiIntent::DragFormation(wide), &ctx);
    let narrow_cmds = commands_for(&UiIntent::DragFormation(narrow), &ctx);
    let ranks = |cmds: &[CommandKind]| match &cmds[0] {
        CommandKind::SetFormation {
            ranks: Some(r),
            regiments,
            ..
        } if regiments == &[RegimentId(0)] => *r,
        other => panic!("expected SetFormation first, got {other:?}"),
    };
    let (wide_ranks, narrow_ranks) = (ranks(&wide_cmds), ranks(&narrow_cmds));
    assert!(
        narrow_ranks > wide_ranks,
        "narrow {narrow_ranks} vs wide {wide_ranks}"
    );
    assert!(matches!(
        &wide_cmds[1],
        CommandKind::Move { facing: Some(facing), speed: SpeedMode::Walk, .. }
            if (facing.radians().to_f32_render() - core::f32::consts::FRAC_PI_2).abs() < 1e-3
    ));
    assert_eq!(wide_cmds.len(), 2);
}

/// T1-062 done-when, headless: ten regiments dragged into a line 100 m
/// north of them face north and stand shoulder to shoulder along the drag.
#[test]
fn ten_regiments_dragged_into_a_battle_line_face_the_drag_direction() {
    let mut world = world();
    let (selection, kinds) = {
        let view = world.view();
        let selection = all(&view);
        assert_eq!(selection.len(), 10);
        let ctx = OrderContext {
            view: &view,
            regiments: &selection,
            speed: SpeedMode::Run,
        };
        let centroid = selection_centroid(&view, &selection).unwrap();
        // The regiments stand on y = 120; the river is around y = 300, so the
        // line goes just north of it, dragged right to left this time.
        let y = centroid.y + 100.0;
        let drag =
            drag_formation(Vec2::new(650.0, y), Vec2::new(150.0, y), centroid, false).unwrap();
        assert!((drag.forward - Vec2::new(0.0, 1.0)).length() < 1e-4);
        let kinds = commands_for(&UiIntent::DragFormation(drag), &ctx);
        (selection.clone(), kinds)
    };
    assert!(matches!(
        &kinds[..],
        [CommandKind::SetSpeedMode { mode: SpeedMode::Run, .. }, CommandKind::GroupFormation { width, .. }]
            if (f(*width) - 500.0).abs() < 1e-2
    ));
    let commands = stamp(&world, kinds);
    let out = world.step(&commands);
    assert!(out.rejected.is_empty(), "{:?}", out.rejected);

    // 100 m at a run is well under a minute; give it 90 s.
    for _ in 0..1800 {
        world.step(&[]);
    }
    let view = world.view();
    let north = Angle::new(S::from_f32_data(core::f32::consts::FRAC_PI_2));
    let mut xs: Vec<(f32, RegimentId)> = Vec::new();
    for id in &selection {
        let r = view.regiment(*id).unwrap();
        let off = r.anchor_facing.delta(north).to_f32_render().abs();
        assert!(
            off < 0.05,
            "regiment {} faces {:?}, expected north",
            id.0,
            r.anchor_facing
        );
        assert!(
            r.order == il_sim_battle::components::OrderKind::Idle,
            "regiment {} still moving",
            id.0
        );
        xs.push((f(r.anchor_pos.x), *id));
    }
    // Side by side: sorted by x, every neighbour gap is at least a regiment
    // width apart and the line spans most of the 500 m drag.
    xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let span = xs.last().unwrap().0 - xs[0].0;
    assert!(span > 350.0 && span < 520.0, "span {span}");
    for w in xs.windows(2) {
        assert!(
            w[1].0 - w[0].0 > 15.0,
            "regiments {} and {} overlap",
            w[0].1.0,
            w[1].1.0
        );
    }
    let ys: Vec<f32> = selection
        .iter()
        .map(|id| f(view.regiment(*id).unwrap().anchor_pos.y))
        .collect();
    let (ymin, ymax) = ys
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(*y), hi.max(*y)));
    // Skirmishers stand `skirmish_offset` ahead; everyone else shares the line.
    assert!(ymax - ymin < 30.0, "line depth {} m", ymax - ymin);
    assert_eq!(world.tick(), Tick(1801));
}
