# Phase 2 performance evidence: the 10k fight (T2-111)

`il_cli bench --scenario tests/scenarios/perf_10k.json5 --ticks 1200 --threads 8` in a release build on the
target machine (`machine.md`): 10,042 soldiers in 52 regiments, side 0 attack-moving into the engine AI's
side 1 (the file's header explains why both sides are not the engine's). Ticks 1 to 1,200 cover the run in,
the missile exchange and the melee. Milliseconds per tick, mean and p95 over the 1,200 ticks.

"Before" is commit ecdcbb6 (T2-100) with only the scenario and `--scenario` added; "after" is the T2-111
commit: the melee gate's extents from one table scan instead of a lookup per soldier, the anchor-grid
queries with a caller scratch, the melee targeting pass marking a soldier changed only when it writes,
the fatigue tick's regiment table, and the morale tick's per-entry side table with the gather in
parallel chunks. Every change is semantics-preserving: the `il_cli run` hash logs of `perf_10k` (1,500
ticks, every 100), `ai_skirmish_300` and `move_reform_2000` (10,000 ticks, every 1,000) are identical
before and after, and the determinism test and bands pass.

| Stage | Budget at 20k (ms) | Before mean | Before p95 | After mean | After p95 |
|---|---|---|---|---|---|
| 0 ApplyCommands | 0.2 | 0.00 | 0.01 | 0.00 | 0.01 |
| 1 Ai | 2.0 | 0.18 | 0.43 | 0.15 | 0.35 |
| 2 Formation | 2.0 | 0.44 | 1.06 | 0.44 | 1.06 |
| 3 RegimentMovement | 1.0 | 0.40 | 0.55 | 0.38 | 0.53 |
| 4 SoldierSteering | 8.0 | 3.45 | 4.61 | 3.34 | 4.47 |
| 5 Integrate | 0.5 | 0.23 | 0.34 | 0.22 | 0.32 |
| 6 SpatialGrid | 2.0 | 0.78 | 1.10 | 0.74 | 1.12 |
| 7 Collision | 8.0 | 4.64 | 7.85 | 4.24 | 7.33 |
| 8 Visibility | 1.0 | 0.03 | 0.11 | 0.03 | 0.11 |
| 9 Targeting | 4.0 | 1.74 | 2.90 | 1.50 | 2.64 |
| 10 Combat | 4.0 | 0.42 | 0.57 | 0.40 | 0.55 |
| 11 Projectiles | 3.0 | 0.01 | 0.02 | 0.01 | 0.02 |
| 12 Abilities | 0.5 | 0.01 | 0.01 | 0.01 | 0.01 |
| 13 Fatigue | 0.5 | 0.46 | 0.91 | 0.36 | 0.81 |
| 14 Morale | 1.0 | 0.68 | 1.65 | 0.59 | 1.04 |
| 15 Death | 1.0 | 0.13 | 0.31 | 0.11 | 0.26 |
| 16 BattleFlow | 0.3 | 0.01 | 0.02 | 0.01 | 0.02 |
| 17 EventsAndHash | 3.0 | 0.73 | 1.76 | 0.64 | 1.52 |
| **tick** | **25 (P2)** | **14.34** | 19.64 (max 26.9) | **13.16** | 17.68 (max 24.5) |

Every stage is inside its 20k budget at 10k, and the tick mean is under the P2 bar of 25 ms with p95 under 18 ms;
the two Phase 1 giants (Collision, Steering) hold 58 % of the tick and stay Phase 3 items (SAD T-10). The
whole-battle profile (`autoresolve --ai none`, 6,000 ticks) put 2,555 dead and 903 fled on the field with
both sides still standing; the file's `time_limit_ticks` (12,000) ends it by the timeout verdict.

Two engine armies (the plan's first shape) never closed: with 25 regiments a side the army AI's battle line is
wider than the 800 m map, its laggards never settle inside `line_tolerance` and the advance never starts (after
6,000 ticks 74 and 122 dead, the rest of the losses routers). A Phase 3 item for the army AI (a second line
when the frontage exceeds the map); recorded in docs/07 T2-111.
