# Phase 3 evidence: the 32,768 cap run (T3-025)

`il_cli bench --scenario tests/scenarios/large/cap_32768.json5 --ticks 3000 --threads 8 --memory` in a release
build on the target machine (`machine.md`), 2026-09-11. The file fills SIM-CORE-006's cap exactly: two engine
armies of 16,384 each (80 regiments of 200, a 383-man bodyguard and the general per side) on `rome:wide_field`.

| Measure | Value |
|---|---|
| Soldiers spawned | 32,768 in 162 regiments (the cap; one more is a `SetupError::OverCap`) |
| Ticks | 3,000 headless, no panic, no crash |
| Peak working set | **52 MB** (exact, `GetProcessMemoryInfo`; REQ-PERF-007 asks for under 4 GB) |
| Tick mean / p95 / max | 25.9 / 38.9 / 72.1 ms on the throttled afternoon clock (see `machine.md`, T3-022) |
| SoldierSteering / Collision / Targeting / EventsAndHash mean | 5.8 / 5.5 / 1.6 / 3.2 ms (same clock) |

The two armies stand in 16 columns of 5 with the front rows 200 m apart and do not close in 3,000 ticks (SAD §12
T-15, the wide-map army AI finding of T3-024), so the run measures the cap's memory and an idle-to-skirmish tick,
which is what REQ-PERF-004 and REQ-PERF-007 ask. The 32k hash stage (T3-023's open question, SAD T-4) read 3.2 ms
on the throttled clock against its 3 ms budget; the morning's healthy clock ran every stage 1.6 to 2 times faster,
so the figure is re-read there before any field leaves `compute_hash`.

No reinforcement group: SIM-CORE-006 counts pending reinforcements in the setup check, so a group beyond a full cap
is rejected at setup (`OverCap`) and never reaches the arrival-time `ReinforcementsDropped` of SIM-FLOW-016; the
drop stays a guard for a cap crossed by anything the setup check did not see. The task's clause on the dropped group
is recorded as unmet by construction in docs/09.

The app run (`cargo run --release -p il_app -- tests/scenarios/large/cap_32768.json5 --threads 8`) is the owner's
check; its outcome is written here when taken.
