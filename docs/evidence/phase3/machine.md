# Phase 3 target machine

The same PC as Phases 1 and 2 (`../phase2/machine.md`: Intel Core i9-7900X, 10 cores / 20 threads, 32 GB DDR4-2400,
GeForce GTX 1080 Ti, Windows 11 Pro 10.0.26200, Balanced power plan). Differences on 2026-09-10 (T3-001):

| Part | Value |
|---|---|
| Toolchain | rustc 1.98.1 (2026-09-01; 1.98.0 in Phase 2), `profile.release` unchanged (`codegen-units = 1`, `lto = "thin"`, no `target-cpu=native`) |
| Pins | egui / egui-wgpu / egui-winit 0.36.2, glam 0.33.7, jsonschema 0.56, `cargo update` of the transitive lock (TDD §1.2); `tracing` dropped from il_sim_battle |
| Sim threads | 8 |
| Bench | `benches/baseline.json` re-recorded for every key (2000, 10000, 20000, perf_10k) in one sitting on 2026-09-10 after the bump; the Phase 2 numbers below are the 2026-09-07 recording |

The `il_cli run` hash logs of `idle_1000` and `move_reform_2000` (10,000 ticks, every 1,000) and `perf_10k` (1,500 ticks,
every 100) were byte-identical before and after the bump, so no floating-point result moved with the toolchain or a pin.

## Delta of the re-record (ms per tick, mean; the Phase 2 baseline → 2026-09-10)

| Key | tick | Collision (7) | SoldierSteering (4) | Targeting (9) | RegimentMovement (3) |
|---|---|---|---|---|---|
| 2000 | 3.52 → 3.65 | 1.51 → 1.62 | 0.65 → 0.64 | 0.28 → 0.27 | 0.10 → 0.10 |
| 10000 | 14.45 → 11.64 | 6.01 → 5.36 | 3.43 → 2.39 | 1.34 → 1.12 | 0.30 → 0.21 |
| 20000 | 30.50 → 22.32 | 12.13 → 9.93 | 7.81 → 5.00 | 3.03 → 2.23 | 0.54 → 0.34 |
| perf_10k | 13.33 → 12.84 | 4.42 → 4.62 | 3.33 → 2.95 | 1.51 → 1.41 | 0.37 → 0.37 |

The larger keys ran faster than the Phase 2 recording (the machine was otherwise idle and the run followed a fresh
release build; the code paths are unchanged, so the difference is run-to-run and toolchain variance, not an
optimisation). Numbers from other machines are not comparable with the baseline; record a new one with
`il_cli bench --record-baseline` and note the machine here.

## Re-record after HPA\* (T3-021, 2026-09-10, later the same day)

Stage 3 (`RegimentMovement`, which serves the paths) is the only stage whose code changed; every key was
re-recorded in one sitting so the baseline stays self-consistent. Mean ms per tick, the T3-001 recording → T3-021:

| Key | tick | RegimentMovement (3) | Collision (7) | SoldierSteering (4) |
|---|---|---|---|---|
| 2000 | 3.65 → 4.74 | 0.10 → 0.13 | 1.62 → 1.74 | 0.64 → 0.78 |
| 10000 | 11.64 → 14.04 | 0.21 → 0.28 | 5.36 → 6.00 | 2.39 → 2.90 |
| 20000 | 22.32 → 27.62 | 0.34 → 0.51 | 9.93 → 11.34 | 5.00 → 6.68 |
| perf_10k | 12.84 → 13.43 | 0.37 → 0.41 | 4.62 → 4.75 | 2.95 → 3.03 |

Stage 3 at 20k moved by 0.17 ms and sits at 0.51 ms against its 1 ms budget. The rest of the change is
run-to-run variance: two unrecorded 20k runs straight after gave 25.3 and 26.9 ms (Collision 10.6 / 11.1,
Steering 6.0 / 6.7), so this machine swings about ±10 % between sittings on the same code; the T3-001 morning
numbers were at the fast end of that band. `--strict` comparisons on this machine should allow for it.

## Exit checklist evidence

- `bench_perf_20k.md`, `profiler_20k.png`: the 20k fight (T3-024).
- `memory_32k.md`: the 32,768 cap run (T3-025).
- `profiler_10k_threaded.png`: the render thread (T3-030).
