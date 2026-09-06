//! Every battle panel draws headless with a filled model (T2-090, plan
//! decision 21): no panics, shapes produced, no click reported without a
//! pointer. The clicks themselves are the docs/08 §4f checkpoint.

use std::path::PathBuf;

use il_core::{RegimentId, Tick};
use il_data::ContentId;
use il_sim_battle::FireMode;
use il_sim_battle::components::MoraleState;
use il_ui::{
    AbilitySlot, CardStripModel, CommandCardModel, HudModel, PauseModel, RegimentCard,
    SelectedRegiment, SideTally, battle_hud, card_strip, casualties_line, command_card, pause_menu,
};

fn regs() -> il_data::Registries {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game");
    il_data::load_roots(&[root]).unwrap_or_else(|e| panic!("{e}"))
}

fn run(mut f: impl FnMut(&egui::Context)) -> egui::FullOutput {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    f(&ctx);
    let mut out = ctx.end_pass();
    // No renderer applies the font texture here (epaint asserts otherwise).
    out.textures_delta.clear();
    out
}

#[test]
fn the_card_strip_draws_and_reports_nothing_without_a_pointer() {
    let regs = regs();
    let cards = vec![
        RegimentCard {
            id: RegimentId(0),
            unit: "Hastati".into(),
            soldiers: 120,
            initial: 160,
            morale_state: MoraleState::Shaken,
            fatigue: "tired".into(),
            volleys: Some(2),
            engaged: true,
            selected: true,
            group: Some(3),
            tint: [214, 66, 52, 255],
        },
        RegimentCard {
            id: RegimentId(1),
            unit: "Velites".into(),
            soldiers: 0,
            initial: 0,
            morale_state: MoraleState::Shattered,
            fatigue: "fresh".into(),
            volleys: None,
            engaged: false,
            selected: false,
            group: None,
            tint: [214, 66, 52, 255],
        },
    ];
    let model = CardStripModel {
        cards: &cards,
        locale: &regs.locale,
    };
    let out = run(|ctx| assert!(card_strip(ctx, &model).is_empty()));
    assert!(!out.shapes.is_empty());
}

#[test]
fn the_command_card_draws_in_both_phases() {
    let regs = regs();
    let rows = vec![SelectedRegiment {
        id: RegimentId(4),
        unit: "Hoplites".into(),
        soldiers: 80,
        formation: "Phalanx".into(),
        ranks: 8,
        order: "idle".into(),
        morale: "morale 70 (steady)".into(),
        fatigue: "fresh".into(),
        abilities: vec!["1: Shield wall (ready)".into()],
        statuses: vec![],
    }];
    for deploying in [false, true] {
        let model = CommandCardModel {
            selection: &rows,
            fire: Some(FireMode::Hold),
            run: true,
            armed_attack_move: true,
            formations: vec![(1, "Line".into()), (2, "Column".into())],
            abilities: vec![AbilitySlot {
                key: 1,
                name: "Shield wall".into(),
                state: "ready".into(),
                ready: true,
            }],
            presets: vec![(
                ContentId::new("rome:battle_line").unwrap(),
                "Battle line".into(),
            )],
            deploying,
            locale: &regs.locale,
        };
        let out = run(|ctx| assert!(command_card(ctx, &model).is_none()));
        assert!(!out.shapes.is_empty());
    }
    // Nothing selected outside the deployment: nothing drawn, nothing asked.
    let empty = CommandCardModel {
        selection: &[],
        fire: None,
        run: false,
        armed_attack_move: false,
        formations: vec![],
        abilities: vec![],
        presets: vec![],
        deploying: false,
        locale: &regs.locale,
    };
    run(|ctx| assert!(command_card(ctx, &empty).is_none()));
}

#[test]
fn casualties_hud_and_pause_menu_draw() {
    let regs = regs();
    let tallies = vec![
        SideTally {
            name: "Rome".into(),
            tint: [214, 66, 52, 255],
            alive: 300,
            killed: 40,
            fled: 12,
        },
        SideTally {
            name: "Greece".into(),
            tint: [64, 96, 214, 255],
            alive: 250,
            killed: 90,
            fled: 0,
        },
    ];
    let hud = HudModel {
        tick: Tick(1234),
        phase: "Deployment".into(),
        deploying: true,
        paused: true,
        speed: 2.0,
        run: false,
        commands: 7,
        locale: &regs.locale,
    };
    let pause = PauseModel {
        can_surrender: true,
        has_settings: false,
        locale: &regs.locale,
    };
    let out = run(|ctx| {
        casualties_line(ctx, &tallies, &regs.locale);
        assert!(battle_hud(ctx, &hud).is_none());
        assert!(pause_menu(ctx, &pause).is_none());
    });
    assert!(!out.shapes.is_empty());
}
