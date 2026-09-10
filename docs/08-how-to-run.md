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

- The positional argument is a scenario file. Without it the main menu opens (T2-091): **Custom battle** builds a battle in the menu (map, weather, seed, time limit, two to four sides with a faction, a controller, a general and roster rows; Start opens it in Deployment; Save writes it as a scenario file so it appears in the list), **Scenario file** lists every `*.json5` under `tests/scenarios/` (change the folder with `--scenarios-dir`), **Load battle** lists the saves under `saves/`, **Settings** opens the settings screen (§2a).
- `--settings <file>` points at a settings file; without it the app reads `%APPDATA%\IronLegion\settings.json5` (Windows), `$XDG_CONFIG_HOME/IronLegion/settings.json5` or `~/.config/IronLegion/settings.json5` elsewhere, and `./settings.json5` when none of those variables is set. A missing file means the defaults.
- `--threads 8` runs the sim on eight workers; the default is one thread.
- `--mod <folder>` loads an extra mod after the game, repeatable (see §6).
- `--ai <player>` hands that player's sides to the engine AI at tick 1, repeatable (T2-081): `--ai 1` turns any two-player scenario into a fight against the AI. The engine's sides deploy and confirm by themselves; the AI's commands join the command count in the title.
- `--show-keys` shows localisation keys instead of text, to spot a label that bypasses the locale.
- `--content-root <folder>` points at a different game root (default `game`).
- `--replay <file.ilrp>` watches a recorded battle instead of playing one (T2-101): no orders are taken, the camera, pause and speed still work, and the title shows `replay 120/600`, then `replay OK` or `replay MISMATCH at tick N`.
- `--replays-dir <folder>` (default `replays`) is where every battle's replay lands, `--saves-dir <folder>` (default `saves`) where the quick save lives (§4g).
- `--mute` starts with the master volume at 0 (T2-100); the settings file is untouched. Without an audio device the terminal prints `audio disabled: …` once and everything else works.

A regiment with a `position` in the file starts deployed there; a side whose every regiment has one skips the deployment phase, and when every side does the battle opens in the Battle phase (that is every file under `tests/scenarios/` except `phases_all_four.json5`). Leave the positions out and the battle opens in Deployment: the regiments stand in a battle line at their zone centre, right-click or right-drag moves the selection inside the zone (a red `OutsideDeploymentZone` in the event panel otherwise), the command card's "Deploy the army as…" picker re-lays the whole army in a group formation, `Enter` or the Confirm button next to the phase label starts the battle. A battle ends in a result window (winner, duration, per side survivors, killed, fled, the general's fate, loot) with a button back to the menu; the sim stops stepping then (T2-070).

The battle screen (T2-090): a column of regiment cards down the left edge, one per regiment of yours (unit, strength bar, a morale dot coloured like the `F10` overlay, fatigue, volleys left for ranged units, an "engaged" mark, the control group); the casualties line at the top (alive, killed, fled per side); the command card at the bottom for the selection (the regiment rows, then Halt, Attack-move, Withdraw, the fire and run toggles, the unit's formations, its abilities and the group-formation picker); the minimap bottom right (terrain, fog outside your regiments' sight, regiment blocks, the camera's viewport); the clock, speed and phase top right. Everything a key does, a button does too, so a battle can be fought with the mouse alone (§4f). Text scales with the window height (1440p rows give 1.33 × the 1080p size).

`move_reform_2000.json5` starts with ten regiments north of the river and a scripted command stream: at one second everyone runs south over the bridge and the ford, later some change formation, wheel, form a battle line and march back. `idle_1000.json5` is a thousand soldiers standing still. The band files under `tests/scenarios/bands/` are small fights (§4a, §4b).

The scenario file format is in the Modding SDK §4.13.

The window title is the quick telemetry line: tick, soldiers drawn, sim milliseconds per tick, speed, selection size, commands recorded, zoom, rotation.

### 2a. Settings

The settings screen (main menu → Settings, or the pause menu's Settings during a battle; T2-091) has three tabs. **Video**: the UI scale (multiplied by the window height over 1080, so 1440p text is a third larger by itself), vertical sync, borderless fullscreen, the simulation thread count for the next battle. **Audio**: the three volumes, applied at once (T2-100): master, effects (the battle sounds) and music (nothing plays on it before Phase 4). **Bindings**: every action with its keys; click a key and press the new key or mouse button (`+` adds a second chord, `−` removes one, Reset restores the mod's default); a chord bound to two actions is shown in red with the other action's name. Apply takes effect at once; Save writes `settings.json5` (the path is shown). The file holds only the bindings you changed, on top of the mods' bindings, plus the video and audio values and the replay and save folders.

## 3. Controls

Every key comes from `game/content/input/bindings.json5`; a mod may rebind any of them, and the settings screen (§2a) rebinds them for you alone.

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
| Attack a regiment | right-click on an enemy soldier you can see (T2-090) |
| Attack-move | `T` (or the command card's button) arms the cursor, then left-click the ground; `Escape` or a right-click cancels (T2-090) |
| Withdraw the selection | `Shift+H` (T2-090) |
| Drag a formation line | right-drag: the line's width is the drag, the regiments face away from where they stand; hold `Alt` to face the other way |
| Halt | `H` |
| Run toggle for new orders | `R` (the HUD shows `running` or `walking`) |
| Fire toggle for the selected ranged regiments | `F` (hold fire / fire at will; regiments start at fire at will) |
| Formation templates of the selected unit type | `F1`..`F4` in the order the unit lists them (hastati: line, column, loose, square) |
| Confirm the deployment (Deployment phase) | `Enter`, or the Confirm button beside the phase label |
| Select a regiment from its card | left-click the card, `Shift` adds, double-click centres the camera on it (T2-090) |
| Minimap | left-click or drag moves the camera there, right-click orders the selection there (T2-090) |
| Abilities of the selected regiments | `Z`, `X`, `C` for the first three slots (the unit's abilities, then its general's; T2-050): hastati testudo, hoplites shield wall, Persian cavalry war cry on the nearest enemy regiment within 60 m |
| Pause | `Space`, or the HUD button |
| Speed | `Ctrl+=` / `Ctrl+-` or the numpad `+` / `-`, or the HUD buttons |
| Pause menu: resume, surrender, quit to the menu | `Escape`, or the HUD's Menu button (T2-090; the battle pauses while it is open) |
| Quick save / quick load | `Ctrl+S` writes `saves/quick.ilsv`, `Ctrl+L` continues from it (T2-101; the event panel confirms the write) |

Only your own regiments (player 0 in the scenarios) can be selected. A single selected regiment that is right-dragged gets its rank count from the drag width; two or more get a battle line. The selection card at the bottom lists each selected regiment's soldiers, formation, order, morale (value and state), fatigue state (fresh, active, tired, exhausted; T2-040), the ability slots with their cooldowns and the active status effects with their remaining seconds (T2-050). A refused ability (on cooldown, engaged, out of range) shows in the event panel.

Developer keys (`dev` feature, on by default):

| Key | Overlay |
|---|---|
| `F12` | profiler window and the event panel |
| `F5` | nav grid (impassable cells) |
| `F6` | formation slots |
| `F7` | regiment paths |
| `F8` | regiment anchors |
| `F9` | spatial grid cells |
| `F10` | morale: a ring per regiment coloured by state (green steady, yellow unsettled, orange shaken, red-orange broken, red routing, grey shattered), one extra ring per fatigue state above fresh, and a morale bar (T2-041) |
| `F11` | escape flow field of the selected regiment's side (side 0 with nothing selected): one arrow per nav cell toward that side's escape edge (T2-042) |
| `Ctrl+F5` | line of sight (T2-060): a ring at each regiment's `los_radius` for the selected regiment's side and a red X on every enemy anchor that side cannot see |
| `Ctrl+F6` | the engine AI's plans (T2-081): each AI side's battle line with a facing tick, a faint link from every regiment to its planned slot, an orange line to its charge or commit target; the title shows the stance per AI side (`no plan` until the army AI of T2-082 decides) |

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

The hastati line attack-moves into the velites on its own (the file scripts it). Expect: the line advances, the two regiments lock together with a ragged front, soldiers fall and stay on the ground as darkened sprites for thirty seconds, and the weaker side thins out first. Since T2-041/042 the losing regiment breaks before it is wiped out: its soldiers turn and run for their own map edge (side 0 south, side 1 north on the test map) and vanish when they reach it.

### 4b. The volley check (Phase 2, T2-031)

```
cargo run --release -p il_app -- tests/scenarios/bands/volley_velites_vs_hastati.json5
```

Nobody moves: the velites throw at will from 35 m. Expect: every four seconds a volley of pale javelins arcs from the loose line into the hastati, a few of them fall each time and stay as corpses, and after eight volleys the velites are out of javelins and stop. Select the velites and press `F` to make them hold fire, `F` again to resume. Under the arrows the hastati's morale drains (`F10`); if they break they run north.

### 4c. The rout check (Phase 2, T2-042)

```
cargo run --release -p il_app -- tests/scenarios/bands/melee_cavalry_rear_charge.json5
```

The file scripts a hastati line fighting velites and, forty seconds in, Persian cavalry charging the hastati from behind. Expect: the charge shock and the rear attacks turn the hastati's `F10` ring red within a few seconds, the line dissolves and the soldiers run south for their edge, the cavalry chase them at the gallop and cut some down, and any hastati that get clear of the enemy by fifty metres with their morale back above thirty stop, turn to face the enemy and reform. `F11` draws the escape field the routers follow. Each side's general rides inside its first regiment (or the one named by `general.bodyguard` in the file, T2-043); `F10` also draws the general's aura circle.

### 4d. The hill check (Phase 2, T2-060)

The window shows your side's fog of war: enemy regiments you cannot see are not drawn, and one you saw earlier leaves a grey diamond with a facing tick at its last known anchor for twenty seconds (`visibility.memory_ticks`). Run any scenario and march a regiment behind the south-east hill of `rome:test_field` (around x 600, y 460) or into the forest in the south-west: the enemy vanishes as the crest or the trees come between you, and reappears within half a second of cresting the hill or closing to 25 m of the trees. `Ctrl+F5` draws your regiments' sight rings and marks the enemies you cannot see; an attack or fire order on a hidden regiment is refused (`NotVisible` in the event panel), and archers at will do not shoot what they cannot see.

### 4e. The AI check (Phase 2, T2-081)

```
cargo run --release -p il_app -- tests/scenarios/bands/melee_hoplites_vs_cavalry.json5 --ai 0
```

The Persian cavalry (yours, player 1) charge on their own from the file; the hoplites are the engine's now. Expect: as the wedge closes to about 40 m the hoplites switch from phalanx to square (`F6` shows the slots re-lay), take the charge, and the AI hoplites throw nothing they do not have. With `melee_hoplites_vs_hastati.json5 --ai 1` the hastati are the engine's: they raise the testudo when the hoplites are still out of reach only if javelins fly, engage the phalanx once it is within about 25 m, and their general's regiment (the first one) hangs back. `Ctrl+F6` draws the plan: the AI's battle line with its facing tick, each regiment's link to its slot, the cavalry's flank slot beyond your line and, once the lines are within 60 m, the orange charge line to your rear-most regiment (T2-082); against a static line the AI's archers stand off and shoot at the enemy's infantry while the army rests 100 m out until its legs are fresh, its velites step out in front, then the whole line marches together at one wing of the enemy line with the cavalry beside it, runs only the last stretch, and never chases the routers (T3-010); the title shows the stance (`attack` against an even enemy, `defend` when outmatched, `retreat` when beaten). A refused AI command would show in the event panel and never should. `cargo run -p il_cli -- autoresolve tests/scenarios/ai_skirmish_300.json5` fights the same kind of battle headless, AI against AI, and prints the result.

### 4f. The mouse-only battle (Phase 2, T2-090)

```
cargo run --release -p il_app -- tests/scenarios/bands/melee_hoplites_vs_hastati.json5 --ai 1
```

Put the keyboard aside. Expect, in order:

1. The three hastati cards down the left edge show full strength bars and green morale dots; the casualties line at the top shows both sides alive with nothing lost; the minimap bottom right shows the field, your blocks red, the hoplites' blocks blue where you can see them, the terrain dark outside your regiments' sight, and the white viewport rectangle.
2. Click a card: it gets a yellow outline, the regiment's soldiers get the selection ring, and the command card appears at the bottom with the regiment's row and the buttons. Shift-click a second card adds it. Double-click a card: the camera jumps to that regiment.
3. Click Attack-move: the button turns red and the cursor becomes a crosshair; left-click the ground ahead of the hoplites and the regiments attack-move there (`AttackMove` in the event panel). Right-click on a hoplite you can see: the selection attacks that regiment (`AttackRegiment`); the enemy's card is not needed.
4. Right-click on the minimap: the selection moves there. Left-click the minimap: the camera goes there.
5. Click Hold fire on a velites regiment if the file had one (this one has none: the fire button is absent for units without missiles), Run/Walk to toggle the run flag, a formation button (`F1: Line` …) to reform, the ability button (`1: Testudo (ready)`) to raise the testudo.
6. As soldiers fall the strength bars shrink, the morale dots turn yellow, orange, red, the casualties line counts the dead and the fled, and an engaged regiment shows "engaged" in orange.
7. Click Menu (top right) or press nothing at all: the pause menu opens and the clock stops. Resume continues. Open it again and click Surrender: the battle ends against you and the result screen appears (T2-091): the winner, the duration, per side the general's fate, the loot and a row per regiment with its initial strength, survivors, killed, fled, experience and ammo, the replay's path, and two buttons. Rematch starts the same battle with the next seed; Back to the menu returns.
8. From the menu, without a scenario file (`cargo run --release -p il_app`): Custom battle, pick the map and two factions, set one side to You and one to Engine AI, add a regiment row or two, Start. The battle opens in Deployment with your regiments in a line at your zone; pick a deployment preset or place them, Confirm, fight. Back in the builder, Save writes the setup under `tests/scenarios/<name>.json5` (the list under Scenario file shows it at once, and `il_cli autoresolve` runs it headless).

Things that would be wrong: a button that does nothing visible, a click on a card that also orders a move, an attack-move that goes off on the first click of the button, a minimap block for a hoplite regiment you cannot see, a builder that lets you Start with two sides marked You, and a `--show-keys` run (`cargo run -p il_app -- <scenario> --show-keys`, or the menu with `--show-keys`) that shows any English word instead of an `il.*` key.

### 4g. Save, load and replay (Phase 2, T2-101)

```
cargo run --release -p il_app -- tests/scenarios/bands/melee_hoplites_vs_hastati.json5 --ai 1
```

1. A minute in, press `Ctrl+S`: the event panel (`F12`) prints `Quick save written to saves\quick.ilsv`. Fight on for a while, then press `Ctrl+L`: the battle jumps back to the saved moment with the same clock, cards and casualties, and continues; the replay of the battle you left is written to `replays/` first (the terminal prints its path).
2. Let the battle end (or Surrender from the pause menu): the result window names the replay file it wrote, `replays/melee_hoplites_vs_hastati-<date>-<time>.ilrp`.
3. Verify it headless and watch it:

```
cargo run --release -p il_cli -- replay replays/<file>.ilrp --verify --threads 8
cargo run --release -p il_app -- --replay replays/<file>.ilrp
```

Expect `verified N ticks` from the first, and the same battle unfolding on its own in the second with `replay N/N` counting up in the title and `replay OK` at the end; a `MISMATCH` would mean the simulation no longer reproduces its own recording, which is a determinism bug. The replay of a battle you quick-loaded verifies from tick 0 too: the save carries the commands and hashes up to the save point.

Things that would be wrong: a quick load that changes the clock, the casualties line or any soldier's place compared with the moment of the save; a replay that verifies headless but shows `MISMATCH` in the app (or the reverse); an order accepted during a playback.

### 4h. The audio check (Phase 2, T2-100)

```
cargo run --release -p il_app -- tests/scenarios/ai_skirmish_300.json5 --threads 8
```

Two AI armies fight on their own. Expect: at the default zoom, as the lines close, a shout for each charge, clashes when they meet, whooshes for the volleys and thuds for the arrows, short cries for the dead, a falling tone when a regiment routs and a rising one when it rallies, a gong if a general dies, a drum at each phase change and a fanfare at the end. Zoom in (mouse wheel) and the individual sounds get louder and pan toward where they happen; zoom all the way out and they fade until only a low roar is left, whose loudness follows how many soldiers are fighting. The settings screen's Audio tab (from the pause menu) changes the volumes while the battle runs; `--mute` starts silent; `cargo run -p il_cli -- gensound` regenerates the placeholder samples under `game/assets/sounds/`.

Things that would be wrong: a charge still audible at the strategic zoom (only the roar belongs there), the roar playing with nobody engaged, a sound for a fight off the edge of the screen, a machine-gun of clashes (the intervals cap them), or an `audio:` warning naming a missing sample.

## 5. Headless tools (`il_cli`)

```
cargo run -p il_cli -- run tests/scenarios/idle_1000.json5 --ticks 10000 --hash-every 1000
cargo run -p il_cli -- run tests/scenarios/move_reform_2000.json5 --ticks 10000 --hash-every 1000 --threads 8
cargo run -p il_cli -- validate game/ --deny-warnings --verbose
cargo run --release -p il_cli -- bench --soldiers 2000 --baseline benches/baseline.json
cargo run --release -p il_cli -- bench --scenario tests/scenarios/perf_10k.json5 --baseline benches/baseline.json
cargo run --release -p il_cli -- bands tests/scenarios/bands --seeds 50 --jobs 8
cargo run -p il_cli -- autoresolve tests/scenarios/phases_all_four.json5
cargo run --release -p il_cli -- autoresolve tests/scenarios/ai_skirmish_300.json5 --record-replay target/skirmish.ilrp
cargo run --release -p il_cli -- replay target/skirmish.ilrp --verify --threads 8
cargo run -p il_cli -- genmap
cargo run -p il_cli -- genart
cargo run -p il_cli -- gensound
```

- `run` prints `tick,hash` lines; two runs, or one thread against eight, must print identical hashes. `--snapshot-at N` writes `snapshot.bin` next to the scenario and `--restore-from` continues from it.
- `validate` loads the mod roots you list and prints every diagnostic with file, line and column; exit code 1 on errors.
- `bench` steps a generated move/reform battle (`--soldiers 2000|10000|20000`, `--ticks 600`) or, with `--scenario <file>`, any scenario file (`tests/scenarios/perf_10k.json5` is the Phase 2 profile: 10,000 soldiers fighting, `--ticks 1200` by default; a run stops early at `Ended`), and prints mean, p95 and max per schedule stage. `--baseline` compares against the checked-in numbers (a scenario is keyed by its file stem), `--strict` fails at +20 %, `--record-baseline` writes a new one. Always run it in release; `docs/evidence/phase2/bench_perf_10k.md` holds the 10k table (T2-111).
- `bands` runs the Simulation Spec §15.3 outcome bands (`tests/scenarios/bands/*.json5`) over many seeds and prints one row per assertion (`held/seeds`, the required fraction, `pass`/`FAIL`/`skip`); `--seeds` and `--max-ticks` shrink a run, `--json` writes the full report, exit code 1 when an active assertion fails. Run it in release; the `casualties` and `routed_before_loss` clauses count the dead only; soldiers that fled the field (T2-042) are neither survivors nor casualties. A band file may load its own rules override through `bands.mods` (`volley_statistical.json5` runs with `projectile_cap: 0`), hold the morale of whole sides at 100 through `bands.pin_morale` (the volley rows: their hastati would otherwise break and run north, T2-042), kill a side's general at a tick through `bands.harness: [{ tick, kill_general }]` (row 7, T2-043), and a `mean_loss_matches` or `mean_loss_below` row compares two files' mean losses after both have run (`volley_testudo.json5` must lose at most 60 % of `volley_velites_vs_hastati.json5`, T2-050).
- `autoresolve <scenario.json5>` runs the scenario to its end (or `--max-ticks`) with the engine AI commanding every side (`--ai all`, the default; the file's scripted commands are dropped with a note) and prints the `BattleResult` as JSON (`--json F` writes it to a file); `--ai none` replays the scripted commands instead, `--ai 1` hands over player 1 only; exit code 2 when the battle did not end (T2-082). The AI band rows `ai_vs_passive` and `ai_vs_charge` (20 seeds each, the Phase 2 exit criterion) run with the others under `bands`.
- `autoresolve --record-replay <file>` also writes the battle's replay; `replay <file> --verify` re-simulates a replay (from the app or from `autoresolve`) with the loaded content and prints `verified N ticks`, or the first divergent tick with both hashes and exit code 1 (T2-101). Without `--verify` it prints the file's header (engine and schema versions, mods, content hash, time written, tick count). A replay written by different content is refused unless `--force`; `--threads 8` proves the recording on the parallel executor. The nightly workflow records `ai_skirmish_300` to its end and verifies it.
- `genmap`, `genart` and `gensound` regenerate the test map, the placeholder sprite sheets and the placeholder battle sounds with their sound set (T2-100); commit the output.

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
cargo test --workspace                                   # everything, including the determinism corpus (about 8 minutes)
cargo test -p il_tests --test determinism                # just determinism: the classic scenarios (10,000 ticks), the band files (1,500) and the 10k fight (800), three tests side by side
cargo test --release -p il_tests --test scenarios -- --ignored --nocapture   # the 50-seed outcome bands (nightly; minutes)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI (`.github/workflows/ci.yml`) runs the same plus a release double-run of `idle_1000`, `move_reform_2000` and the 10k fight `perf_10k` (T2-112) and the bench comparisons at 2k and on the fight; `nightly.yml` runs the outcome bands every night and on demand.

## 9. Where things are

- `docs/07-tasks-phase-0-2.md`: the task list and exit checklists.
- `docs/evidence/phase1/`: the target machine spec and the profiler screenshot.
- `docs/evidence/phase2/`: the machine delta, the 10k fight's stage table before and after T2-111, the band table at the close-out and the owner's 10k profiler screenshot.
- `benches/baseline.json`: stage timings on the target machine.
- `replays/` and `saves/` under the working directory (ignored by git): every battle's replay and the quick save (T2-101).
- `settings.json5` in the user's config directory (§2): the UI scale, video and audio values, the rebound keys (T2-091).
- `game/`: the flagship game as a mod; `game/content/rules/*.json5` hold every engine tunable.
