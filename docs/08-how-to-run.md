# How to run and operate the engine

Everything here works on the code as it stands (Phase 2 in progress): one rendered battlefield, regiments you can select and move, melee, and ranged fire. Commands are run from the repository root in PowerShell or Git Bash.

## 1. Build

```
cargo build --workspace            # debug: fast to build, sim about 2x slower
cargo build --release -p il_app    # release: what the performance numbers were measured with
```

The first release build takes a few minutes. Debug builds already optimise dependencies, so they are fine for looking around; use release when you want the profiler numbers to mean something.

## 2. Run the app

```
cargo run --release -p il_app -- tests/scenarios/move_reform_2000.json5 --threads 8
```

- The positional argument is a scenario file. Without it the main menu opens and lists every `*.json5` under `tests/scenarios/` (change the folder with `--scenarios-dir`).
- `--threads 8` runs the sim on eight workers; the default is one thread.
- `--mod <folder>` loads an extra mod after the game, repeatable (see §6).
- `--show-keys` shows localisation keys instead of text, to spot a label that bypasses the locale.
- `--content-root <folder>` points at a different game root (default `game`).

`move_reform_2000.json5` starts with ten regiments north of the river and a scripted command stream: at one second everyone runs south over the bridge and the ford, later some change formation, wheel, form a battle line and march back. `idle_1000.json5` is a thousand soldiers standing still. The band files under `tests/scenarios/bands/` are small fights (§4a, §4b).

The window title is the quick telemetry line: tick, soldiers drawn, sim milliseconds per tick, speed, selection size, commands recorded, zoom, rotation.

## 3. Controls

Every key comes from `game/content/input/bindings.json5`; a mod may rebind any of them.

| Action | Keys |
|---|---|
| Pan | `W A S D`, arrow keys, mouse at the window edge, middle-drag |
| Zoom | mouse wheel (around the cursor), `=` / `-` |
| Rotate a quarter turn | `Q` / `E` |
| Select a regiment | left-click a soldier |
| Add to the selection | `Shift` + left-click |
| Box select | left-drag (`Shift` adds) |
| Select every regiment of that type on screen | double left-click |
| Select all | `Ctrl+A` |
| Save / recall control group | `Ctrl+0`..`Ctrl+9` / `0`..`9` |
| Move the selection | right-click on the ground |
| Drag a formation line | right-drag: the line's width is the drag, the regiments face away from where they stand; hold `Alt` to face the other way |
| Halt | `H` |
| Run toggle for new orders | `R` (the HUD shows `running` or `walking`) |
| Fire toggle for the selected ranged regiments | `F` (hold fire / fire at will; regiments start at fire at will) |
| Formation templates of the selected unit type | `F1`..`F4` in the order the unit lists them (hastati: line, column, loose) |
| Pause | `Space`, or the HUD button |
| Speed | `Ctrl+=` / `Ctrl+-` or the numpad `+` / `-`, or the HUD buttons |
| Back to the menu | `Escape` |

Only your own regiments (player 0 in the scenarios) can be selected. A single selected regiment that is right-dragged gets its rank count from the drag width; two or more get a battle line.

Developer keys (`dev` feature, on by default):

| Key | Overlay |
|---|---|
| `F12` | profiler window and the event panel |
| `F5` | nav grid (impassable cells) |
| `F6` | formation slots |
| `F7` | regiment paths |
| `F8` | regiment anchors |
| `F9` | spatial grid cells |

## 4. The M4 check: drag ten regiments into a line

This is the in-window checkpoint that has not been signed off yet.

1. Start `move_reform_2000.json5` as in §2 and press `Space` immediately to pause, before the scripted move at one second fires. Zoom out with the wheel until all ten regiments are in view.
2. Press `Ctrl+A` to select all ten. The selection card at the bottom lists them with their unit, soldier count, formation and order.
3. Unpause with `Space`. Right-drag a line about 300 m long on open ground south of the river (the bridge is the tan road crossing in the middle). Release.
4. Expected: every regiment turns and moves to its own place along the line, side by side, facing away from where the selection stood. Holding `Alt` while dragging flips the facing. The event panel (`F12`) shows the `GroupFormation` command; the title shows the command count going up.
5. Press `F6` to see the slots snap into place as they arrive, and `F7` for the paths they took.

Things that would be wrong: regiments overlapping, a regiment facing the opposite way from its neighbours, anyone stuck in the river or the forest polygon to the south-west.

### 4a. The melee check (Phase 2, T2-022)

```
cargo run --release -p il_app -- tests/scenarios/bands/melee_hastati_vs_velites.json5
```

The hastati line attack-moves into the velites on its own (the file scripts it). Expect: the line advances, the two regiments lock together with a ragged front, soldiers fall and stay on the ground as darkened sprites for thirty seconds, and the weaker side thins out first. Nobody routs yet: morale arrives with T2-041.

### 4b. The volley check (Phase 2, T2-031)

```
cargo run --release -p il_app -- tests/scenarios/bands/volley_velites_vs_hastati.json5
```

Nobody moves: the velites throw at will from 35 m. Expect: every four seconds a volley of pale javelins arcs from the loose line into the hastati, a few of them fall each time and stay as corpses, and after eight volleys the velites are out of javelins and stop. Select the velites and press `F` to make them hold fire, `F` again to resume.

## 5. Headless tools (`il_cli`)

```
cargo run -p il_cli -- run tests/scenarios/idle_1000.json5 --ticks 10000 --hash-every 1000
cargo run -p il_cli -- run tests/scenarios/move_reform_2000.json5 --ticks 10000 --hash-every 1000 --threads 8
cargo run -p il_cli -- validate game/ --deny-warnings --verbose
cargo run --release -p il_cli -- bench --soldiers 2000 --baseline benches/baseline.json
cargo run --release -p il_cli -- bands tests/scenarios/bands --seeds 50 --jobs 8
cargo run -p il_cli -- genmap
cargo run -p il_cli -- genart
```

- `run` prints `tick,hash` lines; two runs, or one thread against eight, must print identical hashes. `--snapshot-at N` writes `snapshot.bin` next to the scenario and `--restore-from` continues from it.
- `validate` loads the mod roots you list and prints every diagnostic with file, line and column; exit code 1 on errors.
- `bench` steps a generated move/reform battle (`--soldiers 2000|10000|20000`, `--ticks 600`) and prints mean, p95 and max per schedule stage. `--baseline` compares against the checked-in numbers, `--strict` fails at +20 %, `--record-baseline` writes a new one. Always run it in release.
- `bands` runs the Simulation Spec §15.3 outcome bands (`tests/scenarios/bands/*.json5`) over many seeds and prints one row per assertion (`held/seeds`, the required fraction, `pass`/`FAIL`/`skip`); `--seeds` and `--max-ticks` shrink a run, `--json` writes the full report, exit code 1 when an active assertion fails. Run it in release; a file's rout clauses print `skip` until morale exists (T2-041).
- `genmap` and `genart` regenerate the test map and the placeholder sprite sheets; commit the output.

Criterion micro-benches:

```
cargo bench -p il_benches --benches
```

## 6. Mods

A mod is a folder with a `mod.json5` and a `content/` tree (Modding SDK, `docs/06-modding-sdk-spec.md`). `tests/mods/speed_override/` is the smallest example: it changes one number of the hastati.

```
cargo run --release -p il_app -- tests/scenarios/move_reform_2000.json5 --threads 8 --mod tests/mods/speed_override
cargo run -p il_cli -- validate game/ tests/mods/speed_override --verbose
```

With the mod loaded the hastati walk at the overridden speed; the validate output lists both mods in load order and a different content hash.

## 7. Hot reload

In `dev` builds the app watches every loaded mod folder. Edit a number in `game/content/units/hastati.json5` or `game/content/rules/movement.json5` while a battle runs and save: the new value applies at the next tick and the terminal you launched from prints `hot reload: Swapped { .. }`. A file that fails validation keeps the old values and prints the diagnostics there. New content ids (a new unit) need a restart; manifests are read only at startup.

## 8. Tests and checks

```
cargo test --workspace                                   # everything, including the ten-thousand-tick determinism test (a few minutes)
cargo test -p il_tests --test determinism                # just determinism
cargo test --release -p il_tests --test scenarios -- --ignored --nocapture   # the 50-seed outcome bands (nightly; minutes)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI (`.github/workflows/ci.yml`) runs the same plus a release double-run of both scenarios and the bench comparison; `nightly.yml` runs the outcome bands every night and on demand.

## 9. Where things are

- `docs/07-tasks-phase-0-2.md`: the task list and exit checklists.
- `docs/evidence/phase1/`: the target machine spec and the profiler screenshot.
- `benches/baseline.json`: stage timings on the target machine.
- `game/`: the flagship game as a mod; `game/content/rules/*.json5` hold every engine tunable.
