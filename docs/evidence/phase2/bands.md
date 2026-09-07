# Phase 2 band evidence (T2-113)

`il_cli bands tests/scenarios/bands --seeds 50 --jobs 12` in a release build on the target machine
(`machine.md`) on 2026-09-07, with the rules as committed in the close-out: 14 passed,
1 failed, 0 skipped, 12 rejected commands over every seed. Every row ran 50 seeds
here (`--seeds` overrides the AI rows' 20). Simulation Spec §15.3 carries each row's meaning.

| File | Assertion | Held | Need | Result |
|---|---|---|---|---|
| `ai_vs_charge.json5` | the AI beats a blind charge | 36/50 | >= 0.60 | pass |
| `ai_vs_passive.json5` | the AI beats a passive army | 8/50 | >= 1.00 | FAIL |
| `general_death_hastati_vs_hoplites.json5` | the side without a general routs first | 50/50 | >= 0.75 | pass |
| `melee_cavalry_rear_charge.json5` | rear-charged hastati lose | 50/50 | >= 0.80 | pass |
| `melee_cavalry_rear_charge.json5` | hastati rout within 30 s of the charge | 50/50 | >= 0.80 | pass |
| `melee_hastati_vs_velites.json5` | hastati win | 50/50 | >= 0.90 | pass |
| `melee_hastati_vs_velites.json5` | velites rout early | 50/50 | >= 0.90 | pass |
| `melee_hoplites_vs_cavalry.json5` | hoplites win | 50/50 | >= 0.85 | pass |
| `melee_hoplites_vs_cavalry.json5` | cavalry rout within 60 s of the charge | 50/50 | >= 0.85 | pass |
| `melee_hoplites_vs_hastati.json5` | hoplites win | 50/50 | >= 0.70 | pass |
| `volley_statistical.json5` | hastati lose 15-35 (statistical) | 49/50 | >= 0.90 | pass |
| `volley_statistical.json5` | matches the simulated path | 1.8% | <= 10% of 27.0 | pass |
| `volley_testudo.json5` | testudo halves the losses | 12.3 (46%) | <= 60% of 27.0 | pass |
| `volley_testudo.json5` | hastati lose 3-20 | 50/50 | >= 0.90 | pass |
| `volley_velites_vs_hastati.json5` | hastati lose 15-35 | 47/50 | >= 0.90 | pass |

The two AI rows are the Phase 2 exit criterion's open item (docs/07 T2-082): `ai_vs_charge` passes its
60 % bar, `ai_vs_passive` does not reach 100 %.
