# Phase 1 target machine

The machine every Phase 1 performance number was measured on (the plan's
"target machine = this PC"; REQ-PERF-001, REQ-PERF-005). `benches/baseline.json`
was recorded here on 2026-09-03 with `il_cli bench` in a release build.

| Part | Value |
|---|---|
| CPU | Intel Core i9-7900X, 10 cores / 20 threads, 3.3 GHz base (Skylake-X, 2017) |
| RAM | 32 GB DDR4-2400 |
| GPU | NVIDIA GeForce GTX 1080 Ti, 11 GB, driver 32.0.15.6636 |
| OS | Windows 11 Pro 10.0.26200 |
| Toolchain | rustc 1.98.0 (2026-08-18), `profile.release` with `codegen-units = 1`, `lto = "thin"`, no `target-cpu=native` |
| Power plan | Balanced |
| Sim threads | 8 (`BattleWorld::set_threads(8)`, the determinism test's upper count) |

Numbers from other machines are not comparable with the baseline; record a
new one with `il_cli bench --record-baseline` and note the machine here.

## Exit checklist evidence

`profiler_2000_moving.png`: `il_app tests/scenarios/move_reform_2000.json5 --threads 8`
(release, `dev` feature) at tick 268, all ten regiments running for the river
crossings, profiler overlay open (F12). Frame 16.67 ms (60 FPS, vsync); sim tick
5.87 ms last / 6.07 ms mean / 7.72 ms max over 60 ticks; Collision 2.21 ms and
SoldierSteering 1.69 ms mean. Every label on screen comes from `game/locale/en.json5`
(REQ-LOC-001). The tick is dearer than the headless `il_cli bench`
figure (4.0 ms) because the renderer and egui share the machine with the sim.
