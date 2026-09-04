//! T2-030: the per-soldier reload path (`combat.volley = false`) through a
//! test mod, since the flagship rules synchronise volleys.

use std::sync::Arc;

use il_core::{RegimentId, Tick};
use il_data::Registries;
use il_sim_battle::components::{RangedState, Regiment};
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleEvent, BattleWorld};

#[test]
fn volley_off_lets_soldiers_throw_on_their_own_clocks() {
    let game = il_tests::game_root();
    let mod_dir = il_tests::workspace_root().join("tests/mods/volley_off");
    let regs = Registries::load_roots(&[game, mod_dir]).unwrap_or_else(|e| panic!("{e}"));
    assert!(!regs.rules.combat.volley);
    let regs = Arc::new(regs);
    let scenario = il_tests::load_scenario(
        &il_tests::band_scenario_dir().join("volley_velites_vs_hastati.json5"),
    );
    let mut w = BattleWorld::new(&scenario.setup, regs).unwrap();

    // The band file's loose velites are 96 m wide, so the wings drift in
    // and out of the 40 m range. Synchronised, a wing soldier that misses
    // the volley tick waits for the regiment's next one; without
    // synchronisation it throws as soon as it can (its own cooldown is 0)
    // and reloads on its own clock, so throws appear between the volleys.
    let mut per_tick: Vec<(Tick, u16)> = Vec::new();
    for _ in 0..800 {
        let out = w.step(&[]);
        for e in out.events {
            if let BattleEvent::VolleyFired { regiment, count } = e
                && regiment == RegimentId(0)
            {
                per_tick.push((w.tick(), count));
            }
        }
    }
    assert!(
        per_tick[0].0 == Tick(1) && per_tick[0].1 >= 100,
        "{per_tick:?}"
    );
    let full: Vec<&(Tick, u16)> = per_tick.iter().filter(|(_, c)| *c >= 100).collect();
    assert_eq!(full.len(), 8, "{per_tick:?}");
    for pair in full.windows(2) {
        assert_eq!(pair[1].0.0 - pair[0].0.0, 80, "{per_tick:?}");
    }
    assert!(
        per_tick.iter().any(|(t, _)| t.0 > 1 && t.0 < 81),
        "stragglers throw between the volleys: {per_tick:?}"
    );
    let total: u32 = per_tick.iter().map(|(_, c)| u32::from(*c)).sum();
    assert!((800..=960).contains(&total), "{total}");

    let ids = w.ecs().resource::<Ids>();
    let e = ids.regiment_entity(RegimentId(0)).unwrap();
    let dry = w
        .ecs()
        .get::<Regiment>(e)
        .unwrap()
        .soldiers
        .iter()
        .filter(|s| {
            w.ecs()
                .get::<RangedState>(ids.soldier_entity(**s).unwrap())
                .unwrap()
                .ammo
                == 0
        })
        .count();
    assert!(dry >= 100, "{dry} soldiers threw all eight");
}

#[test]
fn projectile_cap_zero_mod_overrides_only_the_cap() {
    let game = il_tests::game_root();
    let mod_dir = il_tests::workspace_root().join("tests/mods/projectile_cap_zero");
    let regs = Registries::load_roots(&[game, mod_dir]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(regs.rules.combat.projectile_cap, 0);
    assert!(
        regs.rules.combat.volley,
        "untouched fields survive the merge"
    );
    assert_eq!(regs.mods.len(), 2);
}
