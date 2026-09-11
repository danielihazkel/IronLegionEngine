# Phase 3 performance evidence: the 20k fight (T3-024)

`il_cli bench --scenario tests/scenarios/large/perf_20k.json5 --ticks 6000 --threads 8` in a release build on the
target machine (`machine.md`): 20,042 soldiers in 102 regiments on `rome:wide_field` (1600 × 1200 m), side 0
(player 0) attack-moving its 51 regiments into the engine AI's side 1 at tick 20, the perf_10k shape. Ticks 1 to
6,000 cover the run in (first contact near tick 2,000, the engine's charge by 3,000) and the melee; the file's
time limit ends it at 6,000 with 1,293 killed and 1,808 fled and no winner (`autoresolve --ai none`).
Milliseconds per tick, mean and p95 over the 6,000 ticks.

"Before" is commit 52990c6 (T3-021) with the map and this file copied into a worktree; "after" is the tree at
T3-024 (T3-022's collision and steering pass and T3-023's recount). Both were taken in one sitting on the
afternoon of 2026-09-11, when the machine ran at 47 to 75 % of its clock (`machine.md`, the T3-022 note); the
ratio between the two columns is what the sitting proves, and the morning's healthy clock ran the same binary
1.6 to 2 times faster (the 20k idle bench's tick 17 ms against 30 ms). The `il_cli run` hash logs are the
proof that the sim's results did not move.

| Stage | Budget at 20k (ms) | Before mean | Before p95 | After mean | After p95 |
|---|---|---|---|---|---|
| 0 ApplyCommands | 0.2 | 0.01 | 0.02 | 0.01 | 0.02 |
| 1 Ai | 2.0 | 0.86 | 1.10 | 0.86 | 1.11 |
| 2 Formation | 2.0 | 1.07 | 2.20 | 1.07 | 2.18 |
| 3 RegimentMovement | 1.0 | 1.15 | 1.68 | 1.13 | 1.67 |
| 4 SoldierSteering | 8.0 | 11.69 | 13.24 | 8.83 | 10.00 |
| 5 Integrate | 0.5 | 0.60 | 0.71 | 0.60 | 0.68 |
| 6 SpatialGrid | 2.0 | 2.18 | 2.63 | 2.50 | 2.86 |
| 7 Collision | 8.0 | 19.90 | 24.12 | 7.74 | 11.05 |
| 8 Visibility | 1.0 | 0.13 | 0.62 | 0.11 | 0.55 |
| 9 Targeting | 4.0 | 6.66 | 8.18 | 3.79 | 5.80 |
| 10 Combat | 4.0 | 7.43 | 19.73 | 7.60 | 20.34 |
| 11 Projectiles | 3.0 | 0.04 | 0.09 | 0.04 | 0.09 |
| 12 Abilities | 0.5 | 0.02 | 0.03 | 0.02 | 0.03 |
| 13 Fatigue | 0.5 | 1.32 | 4.94 | 1.25 | 4.44 |
| 14 Morale | 1.0 | 2.99 | 3.82 | 2.93 | 3.69 |
| 15 Death | 1.0 | 1.44 | 3.64 | 1.49 | 3.54 |
| 16 BattleFlow | 0.3 | 0.04 | 0.06 | 0.04 | 0.05 |
| 17 EventsAndHash | 3.0 | 3.45 | 4.01 | 3.48 | 4.06 |
| **tick** | **50 (P3)** | **61.00** | 77.34 (max 91.6) | **43.48** | 60.28 (max 72.7) |

The tick mean is under the 50 ms P3 bar even on the throttled clock; Collision fell by 61 % and Targeting by
43 % in the melee, Steering by 24 %. The baseline key `perf_20k` holds the "after" run (`benches/baseline.json`,
recorded on the same clock; CI compares it warn-only at 300 ticks). Collision's narrowed pair set widened
a row list in the melee ticks as the T3-022 note describes (the 10k fight's 1.9 % of row lists).

What the melee shows beyond T3-022/023: `Combat` (Stage 10) costs 7.4 ms mean and 19.7 ms p95 against a 4 ms
budget once tens of regiments fight at once (`melee_attack`'s per-attack work and `apply_outcomes`'s sort),
`Morale` 3.0 against 1.0, `Death` 1.5 against 1.0, `Fatigue`'s p95 4.9 (its every-10-ticks regiment mean) and
`EventsAndHash` 3.5 against 3.0 with the event volume of a melee; all read on the throttled clock, so the
healthy-clock figures are about half. They go to the Phase 3 audit (T3-080) as the next `T-10`-style items.

Two engine armies (the task's first shape) never met on this map: each plan strikes the enemy line's edge away
from the enemy cavalry, and with the cavalry on opposite wings the lines slid to opposite ends of the field;
with both cavalry wings on the same end they converged but never dressed, the plan's 300 m line leaving 16
regiments of 200 no room to stand in their own formations (SAD §12 T-15). The scripted attacker is the
shape that fights; the AI side still runs Stage 1 for its 51 regiments (0.86 ms).

`profiler_20k.png` (the app at `--threads 8`, F12 open in the melee) is the owner's screenshot for the FPS
clause; its reading is added here when taken.
