//! The map editor's end-to-end checks (TDD §16, §17): the registry's test
//! field saved into `tests/mods/editor_out/` is byte for byte the committed
//! map and sidecar, and the two mods validate clean together (T3-060); a
//! hill and a forest painted on a blank map reload with the same samples
//! (T3-061); a map made only with the vector tools plays (T3-062); the
//! handcrafted `rome:valley_crossing` is the builder's map, clean, and a
//! battle on it ends (T3-064).

// The editor's brush positions and radii are render-side f32 (TDD §16,
// il_editor's own lint table); nothing here feeds the sim.
#![allow(clippy::float_arithmetic)]

use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec2;
use il_cli::validate::{ValidateOptions, validate};
use il_core::Scalar;
use il_core::{PlayerId, RegimentId, Tick};
use il_data::Layout;
use il_data::{ContentId, Registries};
use il_editor::brush::HeightOp;
use il_editor::{BlankMap, EditorSession, MapDocument, Tool};
use il_sim_battle::components::{Anchor, FormationState, Order, OrderKind, Path};
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleWorld, Command, CommandKind, LoadedMap, Pathfinder, SpeedMode};

/// A fresh mod folder under `target/` declaring the `edt` namespace.
fn scratch_mod(name: &str) -> PathBuf {
    let dir = il_tests::workspace_root()
        .join("target/il_editor_scratch")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mod.json5"),
        "{\n  id: \"edt\",\n  name_key: \"edt.mod.name\",\n  version: \"0.1.0\",\n  engine_version: \"*\",\n  namespaces: [\"edt\"],\n  dependencies: [{ id: \"rome\", version: \">=0.1.0\" }],\n}\n",
    )
    .unwrap();
    dir
}

fn blank_session(regs: &Arc<Registries>, id: &str, size: [f32; 2]) -> EditorSession {
    let blank = BlankMap {
        id: ContentId::new(id).unwrap(),
        size,
        height_cell: 4.0,
        base_zone: ContentId::new("rome:open").unwrap(),
    };
    let doc = MapDocument::blank(&blank, regs).unwrap();
    EditorSession::open(doc, regs.clone(), vec![il_tests::game_root()], None).unwrap()
}

/// A hill, a forest ring with an unpainted middle (a hole), a marsh
/// patch over the ring and a rock outcrop on the hill, by the session API.
fn paint(s: &mut EditorSession) {
    s.tool = Tool::HeightBrush;
    s.brush.op = HeightOp::Raise;
    s.brush.radius = 120.0;
    s.brush.strength = 6.0;
    let hill = Vec2::new(500.0, 400.0);
    s.begin_stroke(hill);
    for k in 0..40 {
        s.dab(hill + Vec2::new(k as f32, 0.0), 0.1);
    }
    s.end_stroke();
    s.tool = Tool::ZoneBrush;
    s.brush.zone_radius = 30.0;
    let paint_at = |s: &mut EditorSession, zone: &str, points: &[Vec2]| {
        s.brush.zone = ContentId::new(zone).unwrap();
        s.begin_stroke(points[0]);
        for p in points {
            s.dab(*p, 0.05);
        }
        s.end_stroke();
    };
    let centre = Vec2::new(1000.0, 700.0);
    let ring: Vec<Vec2> = (0..36)
        .map(|k| {
            let a = k as f32 / 36.0 * std::f32::consts::TAU;
            centre + Vec2::new(a.cos(), a.sin()) * 120.0
        })
        .collect();
    paint_at(s, "rome:forest", &ring);
    paint_at(
        s,
        "rome:marsh",
        &[
            centre + Vec2::new(120.0, 0.0),
            centre + Vec2::new(150.0, 10.0),
        ],
    );
    paint_at(s, "rome:rock", &[hill]);
}

fn zone_id(regs: &Registries, map: &LoadedMap, p: il_core::V2) -> Option<ContentId> {
    map.zone_at(p).map(|h| regs.zones.get(h).id.clone())
}

/// T3-061 done-when: the painted map saved and loaded back through the
/// pipeline gives the same `height_at` and `zone_at` at every zone cell
/// centre as the editor's live view, and validates clean.
#[test]
fn a_painted_hill_and_forest_reload_with_the_same_samples() {
    let game = il_tests::game_root();
    let regs = Arc::new(
        il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}")),
    );
    let mut s = blank_session(&regs, "edt:painted", [1600.0, 1200.0]);
    paint(&mut s);
    assert_eq!(s.doc.raster_zones.len(), 3);
    let live = s.loaded.clone();
    assert!(live.mean_height > il_core::S::ZERO);

    let out = scratch_mod("painted");
    s.target = out.display().to_string();
    assert!(s.save().is_some(), "{:?}", s.note);
    assert!(s.doc.zone_raster.is_empty(), "the paint is baked");
    assert!(s.doc.def.zones.len() >= 3);

    let regs2 = il_data::load_roots(&[game.clone(), out.clone()]).unwrap_or_else(|d| panic!("{d}"));
    let def = regs2.maps.get(
        regs2
            .maps
            .lookup(&ContentId::new("edt:painted").unwrap())
            .unwrap(),
    );
    let back = LoadedMap::from_def(def, MapDocument::zone_cell(&regs2)).unwrap();
    assert_eq!(
        (back.zone_cols, back.zone_rows),
        (live.zone_cols, live.zone_rows)
    );
    assert_eq!(back.mean_height, live.mean_height);
    let cell = live.zone_cell.to_f32_render();
    let mut forest = 0;
    for j in 0..live.zone_rows {
        for i in 0..live.zone_cols {
            let p = il_core::V2::from_f32_data((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
            assert_eq!(
                back.height_at(p),
                live.height_at(p),
                "height at cell {i},{j}"
            );
            let (a, b) = (zone_id(&regs, &live, p), zone_id(&regs2, &back, p));
            assert_eq!(a, b, "zone at cell {i},{j}");
            forest += usize::from(a.as_ref().is_some_and(|z| z.as_str() == "rome:forest"));
        }
    }
    assert!(forest > 1000, "the ring was painted ({forest} cells)");
    // The ring's middle stayed open ground (the hole survived the bake).
    let mid = il_core::V2::from_f32_data(1000.0, 700.0);
    assert_eq!(zone_id(&regs2, &back, mid).unwrap().as_str(), "rome:open");
    // The session's view after the bake is the same map.
    assert_eq!(s.loaded.zones.len(), live.zones.len());
    for (slot, (x, y)) in s.loaded.zones.iter().zip(&live.zones).enumerate() {
        assert_eq!(
            regs.zones.get(s.loaded.zone_handles[usize::from(*x)]).id,
            regs.zones.get(live.zone_handles[usize::from(*y)]).id,
            "slot {slot}"
        );
    }

    let mut text = Vec::new();
    let report = validate(
        &ValidateOptions {
            roots: vec![game, out],
            deny_warnings: false,
            verbose: true,
        },
        &mut text,
    )
    .expect("validate runs");
    assert_eq!(report.errors, 0, "{}", String::from_utf8_lossy(&text));
}

/// T3-062 done-when: a map with a river, an 8 m bridge, a ford, a road,
/// two deployment zones and one reinforcement edge, made only through the
/// editor's tools, validates, and a regiment on it crosses the bridge in
/// Column with its path touching the river only on the bridge.
#[test]
fn a_map_made_in_the_editor_plays() {
    let game = il_tests::game_root();
    let regs = Arc::new(
        il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}")),
    );
    let mut s = blank_session(&regs, "edt:made_here", [800.0, 600.0]);
    let id = |z: &str| ContentId::new(z).unwrap();
    let click_all = |s: &mut EditorSession, pts: &[(f32, f32)]| {
        for (x, y) in pts {
            s.tool_click(Vec2::new(*x, *y));
        }
        s.tool_finish();
    };
    s.tool = Tool::River;
    s.vector.river_width = 12.0;
    click_all(&mut s, &[(0.0, 300.0), (800.0, 300.0)]);
    s.tool = Tool::Road;
    s.vector.road_zone = id("rome:road");
    s.vector.road_width = 8.0;
    click_all(&mut s, &[(400.0, 0.0), (400.0, 600.0)]);
    s.tool = Tool::Polygon;
    s.vector.polygon_zone = id("rome:bridge");
    click_all(
        &mut s,
        &[
            (396.0, 288.0),
            (404.0, 288.0),
            (404.0, 312.0),
            (396.0, 312.0),
        ],
    );
    s.vector.polygon_zone = id("rome:ford");
    click_all(
        &mut s,
        &[
            (620.0, 285.0),
            (650.0, 285.0),
            (650.0, 315.0),
            (620.0, 315.0),
        ],
    );
    s.tool = Tool::Deployment;
    s.vector.side = 0;
    click_all(
        &mut s,
        &[(40.0, 380.0), (760.0, 380.0), (760.0, 560.0), (40.0, 560.0)],
    );
    s.vector.side = 1;
    click_all(
        &mut s,
        &[(40.0, 40.0), (760.0, 40.0), (760.0, 220.0), (40.0, 220.0)],
    );
    // The blank map's edges were south for side 0 and north for side 1;
    // leave one: side 0 at the north edge behind its zone.
    s.toggle_edge(0, il_data::MapEdge::South);
    s.toggle_edge(1, il_data::MapEdge::North);
    s.toggle_edge(0, il_data::MapEdge::North);
    let d = &s.doc.def;
    assert_eq!(d.rivers.len(), 1);
    let kinds: Vec<&str> = d.zones.iter().map(|z| z.type_id.as_str()).collect();
    assert_eq!(kinds, ["rome:road", "rome:bridge", "rome:ford"]);
    assert_eq!(d.deployment.len(), 2);
    assert_eq!(d.reinforcement_edges.len(), 1);

    let out = scratch_mod("made_here");
    s.target = out.display().to_string();
    assert!(s.save().is_some(), "{:?}", s.note);
    let mut text = Vec::new();
    let report = validate(
        &ValidateOptions {
            roots: vec![game.clone(), out.clone()],
            deny_warnings: false,
            verbose: true,
        },
        &mut text,
    )
    .expect("validate runs");
    assert_eq!(report.errors, 0, "{}", String::from_utf8_lossy(&text));

    let regs2 = il_data::load_roots(&[game, out]).unwrap_or_else(|d| panic!("{d}"));
    let scenario = il_cli::parse_scenario(
        r#"{
  map_id: "edt:made_here",
  seed: 7,
  sides: [
    { faction: "rome:rome", player: 0, deployment_zone: 0,
      general: { unit_type: "rome:general", rank: 1, name_key: "rome.generals.placeholder" },
      regiments: [{ id: 1, unit_type: "rome:hastati", count: 120, position: [300, 450], facing_deg: 270 }] },
    { faction: "rome:rome", player: 1, deployment_zone: 1,
      general: { unit_type: "rome:general", rank: 1, name_key: "rome.generals.placeholder" },
      regiments: [{ id: 2, unit_type: "rome:hastati", count: 20, position: [700, 100], facing_deg: 90 }] },
  ],
}"#,
    )
    .unwrap();
    let mut w = BattleWorld::new(&scenario.setup, Arc::new(regs2)).unwrap();
    let rid = RegimentId(0);
    let entity = w.ecs().resource::<Ids>().regiment_entity(rid).unwrap();
    let layout = |w: &BattleWorld| {
        let st = w.ecs().get::<FormationState>(entity).unwrap();
        w.registries().formations.get(st.template).layout
    };
    assert_eq!(layout(&w), Layout::Line);
    let target = il_core::V2::from_f32_data(300.0, 150.0);
    let order = Command {
        tick: Tick(1),
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::Move {
            regiments: vec![rid],
            target,
            facing: None,
            speed: SpeedMode::Run,
        },
    };
    assert!(w.step(&[order]).rejected.is_empty());
    // The path the world served touches the river only on the bridge.
    let path = w.ecs().get::<Path>(entity).unwrap().clone();
    let mut crossed = Vec::new();
    for pair in path.waypoints.windows(2) {
        let (a, b) = (pair[0].p, pair[1].p);
        for k in 0..=64 {
            let p = a + (b - a) * (il_core::S::from_i32(k) / il_core::S::from_i32(64));
            assert!(
                w.nav_grid().is_passable_at(p),
                "path point {p:?} impassable"
            );
            if w.map().river_at(p) {
                let z = w
                    .map()
                    .zone_at(p)
                    .map(|h| w.registries().zones.get(h).id.clone());
                crossed.push(z.map(|z| z.as_str().to_string()).unwrap_or_default());
            }
        }
    }
    assert!(!crossed.is_empty(), "the path crosses the river");
    assert!(crossed.iter().all(|z| z == "rome:bridge"), "{crossed:?}");
    let mut column = false;
    let mut arrived = false;
    for _ in 0..12_000 {
        w.step(&[]);
        column |= layout(&w) == Layout::Column;
        let order = w.ecs().get::<Order>(entity).unwrap().kind;
        if order == OrderKind::Idle {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "never arrived");
    assert!(column, "never morphed to a column for the bridge");
    let anchor = w.ecs().get::<Anchor>(entity).unwrap();
    assert!(
        anchor.pos.y < il_core::S::from_i32(300),
        "south of the river"
    );
}

/// The river points along `out`'s segments, by the zone they lie in.
fn river_crossings(map: &LoadedMap, regs: &Registries, out: &[il_core::V2]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for pair in out.windows(2) {
        for k in 0..=64 {
            let p = pair[0]
                + (pair[1] - pair[0]) * (il_core::S::from_i32(k) / il_core::S::from_i32(64));
            if map.river_at(p) {
                let z = map
                    .zone_at(p)
                    .map(|h| regs.zones.get(h).id.as_str().to_string())
                    .unwrap_or_default();
                if seen.last() != Some(&z) {
                    seen.push(z);
                }
            }
        }
    }
    seen
}

/// T3-063 done-when: rock painted across the test field's bridge closes
/// that corridor in the nav preview (the routes then use the ford), and
/// rock over the ford too leaves the banks unconnected; the preview's
/// worker gives the same grid as a synchronous build.
#[test]
fn painting_rock_across_the_bridge_closes_the_corridor() {
    let game = il_tests::game_root();
    let regs = Arc::new(
        il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}")),
    );
    let doc = MapDocument::from_registry(
        &regs,
        &ContentId::new("rome:test_field").unwrap(),
        std::slice::from_ref(&game),
    )
    .unwrap();
    let mut s = EditorSession::open(doc, regs.clone(), vec![game], None).unwrap();
    assert!(s.diagnostics.is_empty(), "{:?}", s.diagnostics);
    let v = il_core::V2::from_f32_data;
    let bridge = v(400.0, 310.0);
    let (north, south) = (v(300.0, 150.0), v(300.0, 450.0));
    let route = |s: &EditorSession| {
        let nav = s.nav.grid.as_ref().expect("a preview grid");
        let mut out = Vec::new();
        let r = il_sim_battle::AStar::new().find(nav, north, south, &mut out);
        (r, river_crossings(&s.loaded, &s.regs, &out))
    };
    {
        let nav = s.nav.grid.as_ref().expect("built when the session opens");
        assert!(nav.corridor_width_at(bridge) >= il_core::S::from_i32(8));
    }
    assert_eq!(
        route(&s),
        (
            il_sim_battle::PathResult::Found,
            vec!["rome:bridge".to_string()]
        )
    );

    s.tool = Tool::ZoneBrush;
    s.brush.zone = ContentId::new("rome:rock").unwrap();
    s.brush.zone_radius = 12.0;
    s.begin_stroke(Vec2::new(400.0, 310.0));
    s.dab(Vec2::new(400.0, 310.0), 0.0);
    s.end_stroke();
    // The worker path: quiet frames, then the thread, then the swap.
    let regs_arc = s.regs.clone();
    let mut frames = 0;
    while s.nav.busy() {
        s.nav.poll(&s.loaded, &regs_arc);
        frames += 1;
        assert!(frames < 100_000, "the preview never arrived");
        std::thread::yield_now();
    }
    let worker = s.nav.grid.clone().unwrap();
    {
        let nav = s.nav.grid.as_ref().unwrap();
        assert!(!nav.is_passable_at(bridge), "the bridge cell is rock now");
        for y in [300.0, 310.0, 320.0] {
            assert!(!nav.is_passable_at(v(400.0, y)), "bridge at y {y}");
        }
    }
    assert_eq!(
        route(&s),
        (
            il_sim_battle::PathResult::Found,
            vec!["rome:ford".to_string()]
        )
    );
    s.nav.rebuild_now(&s.loaded, &regs_arc);
    assert_eq!(
        *s.nav.grid.as_ref().unwrap().as_ref(),
        *worker,
        "the worker's grid is the synchronous one"
    );

    s.brush.zone_radius = 30.0;
    s.begin_stroke(Vec2::new(650.0, 297.0));
    s.dab(Vec2::new(650.0, 297.0), 0.0);
    s.dab(Vec2::new(650.0, 310.0), 0.0);
    s.end_stroke();
    s.nav.rebuild_now(&s.loaded, &regs_arc);
    let (r, _) = route(&s);
    assert_ne!(r, il_sim_battle::PathResult::Found, "no crossing is left");
}

/// T3-063 done-when, second half: a bad map shows its diagnostic with the
/// field name and Save is refused while the error stands.
#[test]
fn a_bad_map_names_its_field_and_cannot_be_saved() {
    let game = il_tests::game_root();
    let regs = Arc::new(
        il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}")),
    );
    let mut s = blank_session(&regs, "edt:bad", [400.0, 300.0]);
    let out = scratch_mod("bad");
    s.target = out.display().to_string();
    s.refresh_diagnostics();
    assert!(s.can_save(), "{:?}", s.diagnostics);
    // Deleting both deployment polygons leaves none.
    s.tool = Tool::Select;
    s.selected = Some(il_editor::vector::Feature::Deployment(0));
    s.delete_selected();
    s.selected = Some(il_editor::vector::Feature::Deployment(0));
    s.delete_selected();
    let errors: Vec<&str> = s
        .diagnostics
        .iter()
        .filter(|d| d.severity == il_data::Severity::Error)
        .map(|d| d.field.as_str())
        .collect();
    assert_eq!(errors, ["deployment"], "{:?}", s.diagnostics);
    assert!(!s.can_save());
    assert!(s.save().is_none(), "Save is refused");
    assert!(!out.join("content/maps/bad.json5").exists());
    s.undo();
    assert!(s.can_save(), "{:?}", s.diagnostics);
    assert!(s.save().is_some());
}

/// The T3-061 budget: a dab and its patches stay under 2 ms on the
/// 1600 × 1200 m map at a 100 m radius. Release only, by hand:
/// `cargo test --release -p il_tests --test editor -- --ignored --nocapture`.
#[test]
#[ignore = "timing, run by hand in release"]
#[allow(clippy::disallowed_methods)] // a wall-clock measurement that feeds no sim
fn brush_dab_timing() {
    let game = il_tests::game_root();
    let regs = Arc::new(
        il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}")),
    );
    let mut s = blank_session(&regs, "edt:timing", [1600.0, 1200.0]);
    for (tool, label) in [(Tool::HeightBrush, "height"), (Tool::ZoneBrush, "zone")] {
        for op in [HeightOp::Raise, HeightOp::Smooth] {
            if tool == Tool::ZoneBrush && op == HeightOp::Smooth {
                continue;
            }
            s.tool = tool;
            s.brush.op = op;
            s.brush.radius = 100.0;
            s.brush.zone_radius = 100.0;
            s.begin_stroke(Vec2::new(800.0, 600.0));
            let start = std::time::Instant::now();
            for k in 0..100 {
                s.dab(Vec2::new(500.0 + k as f32 * 6.0, 600.0), 1.0 / 60.0);
            }
            let mean = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
            s.end_stroke();
            println!("{label} {op:?}: {mean:.3} ms per dab at radius 100 m");
            assert!(mean < 2.0, "{label} dab {mean} ms");
        }
    }
}

fn editor_out() -> PathBuf {
    il_tests::workspace_root().join("tests/mods/editor_out")
}

/// T3-060 done-when: open `rome:test_field`, save it into the editor_out
/// mod, compare the bytes, validate the pair with warnings denied.
#[test]
fn test_field_saves_byte_identically_and_validates() {
    let game = il_tests::game_root();
    let regs = il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}"));
    let id = ContentId::new("rome:test_field").unwrap();
    let mut doc = MapDocument::from_registry(&regs, &id, std::slice::from_ref(&game)).unwrap();
    let out = editor_out();
    let saved = doc.save(&out).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(saved.json5, out.join("content/maps/test_field.json5"));
    assert_eq!(saved.hgt, out.join("assets/maps/test_field.hgt"));
    assert!(!doc.dirty);

    // A checkout may carry CRLF (CI's does); the editor writes LF.
    let original = std::fs::read_to_string(game.join("content/maps/test_field.json5"))
        .unwrap()
        .replace("\r\n", "\n");
    let written = std::fs::read_to_string(&saved.json5).unwrap();
    assert_eq!(written, original, "the JSON5 must be byte-identical");
    assert_eq!(
        std::fs::read(&saved.hgt).unwrap(),
        std::fs::read(game.join("assets/maps/test_field.hgt")).unwrap(),
        "the heightmap sidecar must be byte-identical"
    );

    let mut text = Vec::new();
    let report = validate(
        &ValidateOptions {
            roots: vec![game, out],
            deny_warnings: true,
            verbose: true,
        },
        &mut text,
    )
    .expect("validate runs");
    let text = String::from_utf8(text).unwrap();
    assert_eq!(report.errors, 0, "{text}");
    assert_eq!(report.warnings, 0, "{text}");
    assert_eq!(
        report.mods,
        vec!["rome".to_string(), "editor_out".to_string()]
    );
    assert!(
        text.contains("0 errors, 0 warnings in 2 mods (order: rome, editor_out)"),
        "{text}"
    );
}

/// T3-064: `rome:valley_crossing`, made through the editor's own tools in
/// the order of the docs/08 §4i walkthrough. 1200 × 900 m: a ridge in the
/// north-west with forest on its crest, two low rises, a west-east river
/// with an 8 m bridge under a south-north road and a 30 m ford in the
/// east, a marsh in the south-east, one deployment band per side along
/// the south and north edges.
fn valley_crossing_session(regs: &Arc<Registries>) -> EditorSession {
    let mut s = blank_session(regs, "rome:valley_crossing", [1200.0, 900.0]);
    let id = |z: &str| ContentId::new(z).unwrap();
    let v = Vec2::new;
    let stroke = |s: &mut EditorSession, from: Vec2, to: Vec2, dabs: u32, dt: f32| {
        s.begin_stroke(from);
        for k in 0..=dabs {
            s.dab(from + (to - from) * (k as f32 / dabs as f32), dt);
        }
        s.end_stroke();
    };
    let click_all = |s: &mut EditorSession, pts: &[(f32, f32)]| {
        for (x, y) in pts {
            s.tool_click(Vec2::new(*x, *y));
        }
        s.tool_finish();
    };

    // Height: the ridge, two rises, a smoothing pass, the valley floor.
    s.tool = Tool::HeightBrush;
    s.brush.op = HeightOp::Raise;
    s.brush.radius = 120.0;
    s.brush.strength = 7.0;
    stroke(&mut s, v(60.0, 600.0), v(480.0, 640.0), 40, 0.1);
    stroke(&mut s, v(120.0, 610.0), v(400.0, 630.0), 30, 0.1);
    s.brush.radius = 150.0;
    s.brush.strength = 4.0;
    stroke(&mut s, v(220.0, 230.0), v(260.0, 250.0), 20, 0.1);
    stroke(&mut s, v(880.0, 640.0), v(940.0, 660.0), 20, 0.1);
    s.brush.op = HeightOp::Smooth;
    s.brush.radius = 140.0;
    s.brush.strength = 1.0;
    stroke(&mut s, v(60.0, 620.0), v(480.0, 640.0), 30, 0.1);
    s.brush.op = HeightOp::Flatten;
    s.brush.radius = 60.0;
    s.brush.strength = 4.0;
    stroke(&mut s, v(0.0, 440.0), v(1200.0, 440.0), 120, 0.2);

    // Zones painted: forest on the ridge's crest, a marsh in the south-east.
    s.tool = Tool::ZoneBrush;
    s.brush.zone = id("rome:forest");
    s.brush.zone_radius = 40.0;
    stroke(&mut s, v(80.0, 605.0), v(460.0, 640.0), 40, 0.05);
    s.brush.zone = id("rome:marsh");
    s.brush.zone_radius = 35.0;
    stroke(&mut s, v(980.0, 260.0), v(1060.0, 280.0), 8, 0.05);

    // The river, west to east with two bends.
    s.tool = Tool::River;
    s.vector.river_width = 12.0;
    click_all(
        &mut s,
        &[
            (0.0, 430.0),
            (300.0, 470.0),
            (600.0, 450.0),
            (900.0, 420.0),
            (1200.0, 450.0),
        ],
    );
    // The bridge where the road will cross, the ford in the east.
    s.tool = Tool::Polygon;
    s.vector.polygon_zone = id("rome:bridge");
    click_all(
        &mut s,
        &[
            (596.0, 432.0),
            (604.0, 432.0),
            (604.0, 468.0),
            (596.0, 468.0),
        ],
    );
    s.vector.polygon_zone = id("rome:ford");
    click_all(
        &mut s,
        &[
            (935.0, 400.0),
            (965.0, 400.0),
            (965.0, 450.0),
            (935.0, 450.0),
        ],
    );
    // The road, south edge to north edge over the bridge.
    s.tool = Tool::Road;
    s.vector.road_zone = id("rome:road");
    s.vector.road_width = 8.0;
    click_all(&mut s, &[(600.0, 0.0), (600.0, 900.0)]);

    // Deployment: side 0 along the south edge, side 1 along the north; the
    // blank map's reinforcement edges (South, North) stay.
    s.tool = Tool::Deployment;
    s.vector.side = 0;
    click_all(
        &mut s,
        &[(60.0, 30.0), (1140.0, 30.0), (1140.0, 180.0), (60.0, 180.0)],
    );
    s.vector.side = 1;
    click_all(
        &mut s,
        &[
            (60.0, 720.0),
            (1140.0, 720.0),
            (1140.0, 870.0),
            (60.0, 870.0),
        ],
    );

    s.meta.tags = "river, hills, forest".into();
    s.apply_meta();
    s.doc.header = [
        "// Made in the Iron Legion map editor through its own tools by",
        "// tests/tests/editor.rs::build_valley_crossing (T3-064); rebuild it there.",
        "// 1200 x 900 m: a forested ridge in the north-west, a west-east river with",
        "// an 8 m bridge under a south-north road and a 30 m ford, a marsh.",
    ]
    .map(str::to_string)
    .to_vec();
    s
}

/// Rebuilds the committed map into `game/` (T3-064). By hand:
/// `cargo test -p il_tests --test editor -- --ignored build_valley_crossing`.
#[test]
#[ignore = "writes game/content/maps/valley_crossing.json5; run by hand"]
fn build_valley_crossing() {
    let game = il_tests::game_root();
    let regs = Arc::new(
        il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}")),
    );
    let mut s = valley_crossing_session(&regs);
    s.refresh_diagnostics();
    assert!(s.diagnostics.is_empty(), "{:?}", s.diagnostics);
    s.target = game.display().to_string();
    assert!(s.save().is_some(), "{:?}", s.note);
}

/// T3-064 done-when, the headless half: the committed map is what the
/// builder makes, byte for byte; it opens in the editor with no
/// diagnostic, validates clean with warnings denied, carries the `river`
/// tag, and routes from the south band to the north band cross the river
/// only on the bridge or the ford.
#[test]
fn valley_crossing_is_the_builders_map_and_clean() {
    let game = il_tests::game_root();
    let regs = Arc::new(
        il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}")),
    );
    let mut built = valley_crossing_session(&regs);
    let out = scratch_mod("valley_crossing");
    built.target = out.display().to_string();
    assert!(built.save().is_some(), "{:?}", built.note);
    let original = std::fs::read_to_string(game.join("content/maps/valley_crossing.json5"))
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(
        std::fs::read_to_string(out.join("content/maps/valley_crossing.json5")).unwrap(),
        original,
        "rerun build_valley_crossing"
    );
    assert_eq!(
        std::fs::read(out.join("assets/maps/valley_crossing.hgt")).unwrap(),
        std::fs::read(game.join("assets/maps/valley_crossing.hgt")).unwrap(),
        "rerun build_valley_crossing"
    );

    let id = ContentId::new("rome:valley_crossing").unwrap();
    let doc = MapDocument::from_registry(&regs, &id, std::slice::from_ref(&game)).unwrap();
    assert!(doc.def.campaign_terrain_tags.iter().any(|t| t == "river"));
    let s = EditorSession::open(doc, regs.clone(), vec![game.clone()], None).unwrap();
    assert!(s.diagnostics.is_empty(), "{:?}", s.diagnostics);
    let nav = s.nav.grid.as_ref().expect("built when the session opens");
    let v = il_core::V2::from_f32_data;
    for (from, to) in [
        (v(600.0, 100.0), v(600.0, 800.0)),
        (v(1000.0, 100.0), v(1000.0, 800.0)),
        (v(150.0, 100.0), v(150.0, 800.0)),
    ] {
        let mut path = Vec::new();
        let r = il_sim_battle::AStar::new().find(nav, from, to, &mut path);
        assert_eq!(r, il_sim_battle::PathResult::Found);
        let crossed = river_crossings(&s.loaded, &s.regs, &path);
        assert!(!crossed.is_empty(), "the route crosses the river");
        assert!(
            crossed
                .iter()
                .all(|z| z == "rome:bridge" || z == "rome:ford"),
            "{crossed:?}"
        );
    }

    let mut text = Vec::new();
    let report = validate(
        &ValidateOptions {
            roots: vec![game],
            deny_warnings: true,
            verbose: true,
        },
        &mut text,
    )
    .expect("validate runs");
    let text = String::from_utf8(text).unwrap();
    assert_eq!(report.errors, 0, "{text}");
    assert_eq!(report.warnings, 0, "{text}");
}

/// T3-064 done-when: a battle on the handcrafted map is fought to a
/// result (`valley_crossing_skirmish.json5`, both armies the engine's).
#[test]
fn a_battle_on_valley_crossing_ends_with_a_winner() {
    let path = il_tests::scenario_dir().join("valley_crossing_skirmish.json5");
    let scenario = il_tests::load_scenario(&path);
    assert_eq!(scenario.setup.map_id.as_str(), "rome:valley_crossing");
    let mut w = BattleWorld::new(&scenario.setup, il_tests::game_regs()).unwrap();
    while w.phase() != il_sim_battle::BattlePhase::Ended && w.tick().0 < 20_000 {
        w.step(&[]);
    }
    assert_eq!(w.phase(), il_sim_battle::BattlePhase::Ended);
    assert!(w.result().winner.is_some(), "{:?}", w.result());
}
