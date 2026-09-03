//! T1-022 golden test: the Modding SDK 4.1 worked example
//! (`mymod:thracian_peltast` derived from `rome:velites` with `$from`, a
//! nested merge, `$append` and `$replace`) loads from two mod folders and
//! merges to exactly the checked-in expected object.

use il_data::{ContentId, Registries, UnitType};

fn fixture(name: &str) -> std::path::PathBuf {
    il_tests::workspace_root()
        .join("tests/mods/sdk_example")
        .join(name)
}

#[test]
fn sdk_worked_example_merges_to_the_expected_object() {
    let roots = [fixture("rome"), fixture("mymod")];
    let found = il_data::discover(&roots).expect("both manifests parse");
    let set = il_data::ModSet::all(&found).expect("load order resolves");
    assert_eq!(
        set.mods
            .iter()
            .map(|m| m.manifest.id.as_str())
            .collect::<Vec<_>>(),
        vec!["rome", "mymod"]
    );

    let merged = il_data::pipeline::merged_json::<UnitType>(&set).unwrap_or_else(|e| panic!("{e}"));
    let got = &merged[&ContentId::new("mymod:thracian_peltast").unwrap()];
    let expected: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixture("expected/thracian_peltast.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        got,
        &expected,
        "merged object differs from tests/mods/sdk_example/expected/thracian_peltast.json:\n{}",
        serde_json::to_string_pretty(got).unwrap()
    );

    // The base is untouched by the derivation.
    let base = &merged[&ContentId::new("rome:velites").unwrap()];
    assert_eq!(base["speed_run"], 4.5);
    assert_eq!(base["abilities"], serde_json::json!([]));

    // And the merged result passes the schema, so both units load.
    let regs = Registries::load_roots(&roots).unwrap_or_else(|e| panic!("{e}"));
    let h = regs
        .units
        .lookup(&ContentId::new("mymod:thracian_peltast").unwrap())
        .expect("derived unit is registered");
    assert_eq!(
        regs.units.get(h).speed_run,
        <il_core::S as il_core::Scalar>::from_f32_data(5.2)
    );
    assert_eq!(regs.units.len(), 2);
}
