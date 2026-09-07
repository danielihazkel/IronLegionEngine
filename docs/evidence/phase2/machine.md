# Phase 2 target machine

The same PC as Phase 1 (`../phase1/machine.md`: Intel Core i9-7900X, 10 cores / 20 threads, 32 GB DDR4-2400,
GeForce GTX 1080 Ti, Windows 11 Pro 10.0.26200, Balanced power plan). Differences on 2026-09-07:

| Part | Value |
|---|---|
| Toolchain | rustc 1.98.0 (2026-08-18), `profile.release` with `codegen-units = 1`, `lto = "thin"`, no `target-cpu=native` |
| Sim threads | 8 (`--threads 8`, the determinism test's upper count) |
| Bench | `benches/baseline.json` re-recorded for every key (2000, 10000, 20000, perf_10k) in one sitting on 2026-09-07 (T2-111), so the Phase 2 numbers compare with each other; the Phase 1 columns stay in the TDD budget table as history |

Numbers from other machines are not comparable with the baseline; record a new one with
`il_cli bench --record-baseline` and note the machine here.

## Exit checklist evidence

- `bench_perf_10k.md`: the 10k fight per stage, before and after T2-111.
- `bands.md`: the Simulation Spec §15.3 band table at the close-out (T2-113).
- `profiler_10k.png`: the owner's screenshot of `cargo run --release -p il_app -- tests/scenarios/perf_10k.json5 --threads 8` with the profiler (F12) open during the melee (T2-113).
