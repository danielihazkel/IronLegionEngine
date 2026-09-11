//! T2-071: the result reconciles with the initial counts for every
//! scenario in the test set (REQ-SIM-061, SIM-FLOW-018): every scenario
//! under `tests/scenarios/` and every band file, run to its end or 3,000
//! ticks, per regiment `initial == survivors + fled + killed`, per side the
//! sum of `initial` equals the setup's soldiers (the general included) plus
//! the pending reinforcements, and the summary equals the sums.

use il_sim_battle::{BattlePhase, BattleWorld};

#[test]
fn every_scenario_result_reconciles_with_its_setup() {
    let regs = il_tests::game_regs();
    let files: Vec<_> = il_tests::scenario_files()
        .into_iter()
        .chain(il_tests::band_scenario_files())
        .collect();
    assert!(files.len() >= 10, "{files:?}");
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let scenario = il_tests::load_scenario(&path);
        let mut script = scenario.script();
        let mut world = BattleWorld::new(&scenario.setup, regs.clone())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        while world.phase() != BattlePhase::Ended && world.tick().0 < 3_000 {
            let commands = script.take_for(world.tick().next());
            world.step(&commands);
        }
        let r = world.result();
        assert_eq!(r.sides.len(), scenario.setup.sides.len(), "{name}");
        let mut killed = 0u32;
        let mut fled = 0u32;
        for (s, side) in r.sides.iter().enumerate() {
            let setup_side = &scenario.setup.sides[s];
            let expected: u32 = setup_side
                .regiments
                .iter()
                .chain(
                    setup_side
                        .reinforcements
                        .iter()
                        .flat_map(|g| g.regiments.iter()),
                )
                .map(|x| u32::from(x.total()))
                .sum::<u32>()
                + 1;
            let initial: u32 = side.regiments.iter().map(|x| u32::from(x.initial)).sum();
            assert_eq!(initial, expected, "{name}: side {s} initial");
            for x in &side.regiments {
                assert_eq!(
                    x.initial,
                    x.survivors + x.fled + x.killed,
                    "{name}: side {s} regiment {} does not reconcile: {x:?}",
                    x.id
                );
                killed += u32::from(x.killed);
                fled += u32::from(x.fled);
                // SIM-FORM-015 (T3-041): one row per setup group, summing
                // to the regiment's totals.
                let groups = setup_side
                    .regiments
                    .iter()
                    .chain(
                        setup_side
                            .reinforcements
                            .iter()
                            .flat_map(|g| g.regiments.iter()),
                    )
                    .find(|r| r.id == x.id)
                    .map(|r| r.groups())
                    .unwrap_or_default();
                assert_eq!(
                    x.units.len(),
                    groups.len(),
                    "{name}: rows of regiment {}",
                    x.id
                );
                for (row, g) in x.units.iter().zip(&groups) {
                    assert_eq!(row.unit_type, g.unit_type, "{name}: regiment {}", x.id);
                    assert_eq!(
                        row.initial,
                        row.survivors + row.killed + row.fled,
                        "{name}: regiment {} group {} does not reconcile: {row:?}",
                        x.id,
                        g.unit_type
                    );
                }
                let sum = |f: fn(&il_sim_battle::UnitGroupResult) -> u16| -> u32 {
                    x.units.iter().map(|u| u32::from(f(u))).sum()
                };
                assert_eq!(sum(|u| u.initial), u32::from(x.initial), "{name}: {}", x.id);
                assert_eq!(
                    sum(|u| u.survivors),
                    u32::from(x.survivors),
                    "{name}: {}",
                    x.id
                );
                assert_eq!(sum(|u| u.killed), u32::from(x.killed), "{name}: {}", x.id);
                assert_eq!(sum(|u| u.fled), u32::from(x.fled), "{name}: {}", x.id);
            }
        }
        assert_eq!(r.summary.total_killed, killed, "{name}");
        assert_eq!(r.summary.total_fled, fled, "{name}");
        if world.phase() != BattlePhase::Ended {
            assert_eq!(r.winner, None, "{name}: no winner before the end");
        }
    }
}
