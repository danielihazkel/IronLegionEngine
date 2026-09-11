# The mixed cohort band (T3-041)

`cargo run --release -p il_cli -- bands tests/scenarios/bands/melee_mixed_cohort.json5 --seeds 50 --jobs 8`
on the target machine (`machine.md`) on 2026-09-11, with the rules as committed in T3-040/041.
Simulation Spec §15.3 row 11 carries the row's meaning.

| File | Assertion | Held | Need | Result |
|---|---|---|---|---|
| `melee_mixed_cohort.json5` | the cohort wins | 35/50 | >= 0.60 | pass |

50 seeds, mean end tick 3,308; mean survivors side 0 (the cohort of 40 velites, 100 hastati and
the general) 86.6 of 141, side 1 (120 hoplites in phalanx and the general) 10.8 of 121.

The clause was measured first and pinned one round step below the measurement (70 % held, 60 %
required), as the statistical rows of T2-032 were (owner's decision 2026-09-11). The outcomes are
bimodal, as the AI rows of T3-010 are: the seeds the cohort loses are the ones where the hastati
break at contact after the javelins and pila are spent.
