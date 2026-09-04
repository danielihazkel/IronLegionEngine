# Iron Legion Engine

Rust 2D strategy engine: turn-based campaign, real-time battles of tens of thousands of individually simulated soldiers. Solo hobby project.

## Read first
- `docs/README.md` is the map. The docs are the source of truth; if code and docs disagree, fix the docs in the same commit.
- `docs/08-how-to-run.md` is how a person runs and operates the engine; keep it current when commands or keys change.
- `docs/07-tasks-phase-0-2.md` is the active task list. Tick a box only when its **Done when** holds.
- `docs/04-tdd.md` §18 is the determinism checklist; every sim change is reviewed against it.

## Commands
```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p il_cli -- run tests/scenarios/idle_1000.json5 --ticks 10000 --hash-every 1000
cargo run --release -p il_cli -- bench --soldiers 2000 --baseline benches/baseline.json
```

## Crate map
- `crates/il_core` ids, `Scalar`, `Vec2`/`Angle`, state hash, RNG streams, tick, events
- `crates/il_data` JSON5 loading, registries, handles, diagnostics (only crate that touches the filesystem at load)
- `crates/il_sim_battle` headless battle ECS: 18-stage schedule, Commands in, Events out, snapshot, hash
- `crates/il_sim_campaign`, `crates/il_ai`, `crates/il_save` placeholders until their phase
- `crates/il_cli` headless runner; `crates/il_app` application shell
- `game/` the flagship game as a mod (`mod.json5`, `content/`); `game/rules` game-specific Rust behind engine traits
- `tests/` integration tests package (`il_tests`): tests in `tests/tests/`, scenarios in `tests/scenarios/`
- `benches/` benchmark package (`il_benches`)

## Rules that clippy cannot fully enforce
- Sim crates (`il_core`, `il_data`, `il_ai`, `il_sim_*`) never depend on wgpu, winit, egui, audio crates, `rand`, or `game_rules` (`tests/tests/dep_rules.rs`).
- No `HashMap` iteration, no `Instant`, no float literals outside `il_core::scalar`; all arithmetic through `Scalar`.
- Parallel systems write only their own entity or a per-entity buffer applied in ascending stable id order.

## Commits
One commit per task, titled `T<phase>-<nnn>: <task title>`, ticking the task's checkbox in `docs/07-tasks-phase-0-2.md` in the same commit.
