//! The custom battle builder (T2-091, decision 12, I16): the default draft
//! over the flagship content builds a setup the sim accepts and that opens
//! in Deployment; the draft's errors name what is wrong; a saved setup
//! parses back as a scenario file and builds the same world; the screens
//! draw headless.

use std::path::PathBuf;
use std::sync::Arc;

use il_core::PlayerId;
use il_data::{ContentId, UnitCategory};
use il_sim_battle::{BattlePhase, BattleWorld, SOLDIER_CAP, Scenario};
use il_ui::{
    BuildError, BuilderCatalog, BuilderState, Controller, FactionChoice, MapChoice, RowDraft,
    SideDraft, UnitChoice,
};

fn regs() -> Arc<il_data::Registries> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game");
    Arc::new(il_data::load_roots(&[root]).unwrap_or_else(|e| panic!("{e}")))
}

/// The catalog the app builds from the registries (`il_app::menus`), here by
/// hand so the test stays inside il_ui.
fn catalog(regs: &il_data::Registries) -> BuilderCatalog {
    let l = &regs.locale;
    let unit = |h| {
        let u = regs.units.get(h);
        UnitChoice {
            id: u.id.clone(),
            name: l.get(&u.name_key).to_string(),
            category: u.category,
        }
    };
    let factions = regs
        .factions
        .iter()
        .map(|(_, f)| {
            let (generals, units): (Vec<UnitChoice>, Vec<UnitChoice>) = f
                .units
                .iter()
                .map(|h| unit(*h))
                .partition(|u| u.category == UnitCategory::General);
            FactionChoice {
                id: f.id.clone(),
                name: l.get(&f.name_key).to_string(),
                units,
                generals,
            }
        })
        .collect();
    let maps = regs
        .maps
        .iter()
        .map(|(_, m)| MapChoice {
            id: m.id.clone(),
            name: l.get(&m.name_key).to_string(),
            zones: m.deployment.len(),
            weather: m.weather_allowed.clone(),
        })
        .collect();
    BuilderCatalog { maps, factions }
}

#[test]
fn the_default_draft_builds_a_setup_that_opens_in_deployment() {
    let regs = regs();
    let catalog = catalog(&regs);
    assert!(catalog.factions.len() >= 2 && !catalog.maps.is_empty());
    let draft = BuilderState::default_for(&catalog, 77);
    assert_eq!(draft.sides.len(), 2);
    assert_eq!(draft.sides[0].controller, Controller::You);
    assert_eq!(draft.sides[1].controller, Controller::EngineAi);
    let setup = draft.to_setup(&catalog).unwrap();
    assert_eq!(setup.seed, 77);
    assert_eq!(setup.sides[0].player, PlayerId(0));
    assert_eq!(setup.sides[1].player, PlayerId::ENGINE_AI);
    assert_eq!(setup.sides[1].deployment_zone, 1);
    assert_eq!(setup.time_limit_ticks, 40 * 1200);
    assert!(
        setup
            .sides
            .iter()
            .all(|s| s.regiments.iter().all(|r| r.position.is_none()))
    );
    assert_eq!(draft.soldiers(), 2 * (120 + 1));
    let world = BattleWorld::new(&setup, regs).expect("the sim accepts the setup");
    assert_eq!(world.phase(), BattlePhase::Deployment);
}

#[test]
fn the_draft_names_what_is_wrong() {
    let regs = regs();
    let catalog = catalog(&regs);
    let mut d = BuilderState::default_for(&catalog, 1);
    d.sides[1].controller = Controller::You;
    assert_eq!(d.to_setup(&catalog).unwrap_err(), BuildError::SeveralYou);
    d.sides[1].controller = Controller::Idle;
    d.sides[0].rows[0].count = 1000;
    d.sides[1].rows = (0..33)
        .map(|_| RowDraft {
            unit: 0,
            count: 1000,
            experience: 0,
        })
        .collect();
    assert!(d.soldiers() > SOLDIER_CAP);
    assert!(matches!(
        d.to_setup(&catalog).unwrap_err(),
        BuildError::OverCap { .. }
    ));
    let mut one = BuilderState::default_for(&catalog, 1);
    one.sides.pop();
    assert_eq!(one.to_setup(&catalog).unwrap_err(), BuildError::TooFewSides);
    let mut many = BuilderState::default_for(&catalog, 1);
    for _ in 0..5 {
        many.sides.push(SideDraft {
            faction: 0,
            controller: Controller::EngineAi,
            general: 0,
            rows: vec![RowDraft {
                unit: 0,
                count: 10,
                experience: 0,
            }],
        });
    }
    assert!(matches!(
        many.to_setup(&catalog).unwrap_err(),
        BuildError::TooManySides { .. }
    ));
    let mut empty = BuilderState::default_for(&catalog, 1);
    empty.sides[1].rows.clear();
    assert_eq!(
        empty.to_setup(&catalog).unwrap_err(),
        BuildError::EmptySide { side: 1 }
    );
    // Idle sides take the player ids after yours.
    let mut idle = BuilderState::default_for(&catalog, 1);
    idle.sides[1].controller = Controller::Idle;
    let setup = idle.to_setup(&catalog).unwrap();
    assert_eq!(setup.sides[1].player, PlayerId(1));
}

#[test]
fn a_saved_setup_parses_back_as_a_scenario_and_builds_the_same_world() {
    let regs = regs();
    let catalog = catalog(&regs);
    let setup = BuilderState::default_for(&catalog, 5)
        .to_setup(&catalog)
        .unwrap();
    let scenario = Scenario {
        setup: setup.clone(),
        commands: Vec::new(),
    };
    let text = serde_json::to_string_pretty(&scenario).unwrap();
    let back: Scenario = json5::from_str(&text).expect("pretty JSON is JSON5");
    assert_eq!(back.setup, setup);
    assert_eq!(
        back.setup.map_id,
        ContentId::new("rome:test_field").unwrap()
    );
    let a = BattleWorld::new(&setup, regs.clone()).unwrap().hash();
    let b = BattleWorld::new(&back.setup, regs).unwrap().hash();
    assert_eq!(a, b);
}

#[test]
fn the_menu_screens_draw_headless() {
    let regs = regs();
    let catalog = catalog(&regs);
    let mut draft = BuilderState::default_for(&catalog, 9);
    let mut settings = il_ui::SettingsState::new(il_ui::SettingsDraft {
        ui_scale: 1.0,
        vsync: true,
        fullscreen: false,
        threads: 1,
        master: 1.0,
        effects: 0.5,
        music: 0.0,
        bindings: vec![il_ui::BindingRow {
            action: "order_halt".into(),
            keys: vec!["H".into(), "J".into()],
            default_keys: vec!["H".into()],
        }],
    });
    settings.tab = il_ui::Tab::Bindings;
    settings.capturing = Some(il_ui::Capture { row: 0, slot: 1 });
    assert_eq!(settings.conflicts(0, 0), Vec::<String>::new());
    let entries = vec![il_ui::SaveEntry {
        path: PathBuf::from("saves/quick.ilsv"),
        name: "quick.ilsv".into(),
        created: "2026-09-06T12:00:00Z".into(),
        tick: Some(1200),
        summary: "a battle".into(),
        problem: None,
    }];
    let sides = vec![il_ui::ResultSide {
        name: "Rome".into(),
        tint: [214, 66, 52, 255],
        fate: "alive".into(),
        loot: 120,
        rows: vec![il_ui::ResultRow {
            unit: "Hastati".into(),
            initial: 120,
            survivors: 80,
            killed: 30,
            fled: 10,
            experience: 3,
            ammo: 0,
            arrived: true,
        }],
    }];
    let ctx = egui::Context::default();
    for over_battle in [false, true] {
        ctx.begin_pass(egui::RawInput::default());
        assert!(il_ui::custom_battle(&ctx, &mut draft, &catalog, &regs.locale).is_none());
        assert!(il_ui::settings_screen(&ctx, &mut settings, &regs.locale, over_battle).is_none());
        assert!(il_ui::load_screen(&ctx, &entries, &regs.locale).is_none());
        assert!(
            il_ui::result_screen(
                &ctx,
                &il_ui::ResultScreenModel {
                    winner: Some(0),
                    duration: "12:34".into(),
                    sides: &sides,
                    replay_path: Some("replays/x.ilrp"),
                    can_rematch: true,
                    locale: &regs.locale,
                },
            )
            .is_none()
        );
        let mut out = ctx.end_pass();
        assert!(!out.shapes.is_empty());
        out.textures_delta.clear();
    }
    // A captured chord lands in the slot being captured.
    settings.captured("Ctrl+K".into());
    assert_eq!(settings.draft.bindings[0].keys, ["H", "Ctrl+K"]);
    assert!(settings.dirty && settings.capturing.is_none());
}
