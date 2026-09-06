# Iron Legion Engine — Simulation Design Spec

| | |
|---|---|
| **Version** | 0.1 |
| **Status** | Draft for review |
| **Upstream** | [PRD v0.2](01-prd.md) · [SAD](02-sad.md) · [Glossary](00-glossary.md) |
| **Downstream** | [TDD](04-tdd.md) · [Modding SDK](06-modding-sdk-spec.md) |

## How to read this document

Every rule is numbered `SIM-<AREA>-nnn` and is written so that a developer can implement it without asking a design question. Tunables are named as the data fields that hold them (snake_case) and their antiquity default is given in §15. The same field names appear in the TDD structs and the Modding SDK schemas.

Conventions:

- `dt` is one tick, 50 ms (REQ-SIM-021). Speeds are in world units per second in data and converted to per-tick at load. One world unit is one metre.
- `S` denotes the `Scalar` type. All formulas are evaluated in `S`.
- `rng.<stream>` denotes a draw from a named RNG stream (§2).
- `clamp(x, lo, hi)`, `lerp(a, b, t)`, `sat(x) = clamp(x, 0, 1)`.
- "Data field" means a value read from a registry at tick time, never a constant in code.

---

## 1. Simulation model

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CORE-001 | The battle world is a rectangle `[0, map.width] × [0, map.height]` in metres with a heightmap `map.height_at(x, y)` in metres, sampled bilinearly from a grid of cell size `map.height_cell` (default 4 m). | REQ-SIM-020 |
| SIM-CORE-002 | The simulation advances in ticks of exactly 50 ms. All timers are integers in ticks. | REQ-SIM-021 |
| SIM-CORE-003 | Entity hierarchy: Faction → Army → Regiment → Soldier; Projectiles are owned by the world and reference their shooter's Regiment. | REQ-SIM-022 |
| SIM-CORE-004 | Each Soldier has: stable `SoldierId`, `RegimentId`, position `p`, velocity `v`, facing `θ`, radius `r = unit.soldier_radius`, mass `m = unit.mass`, `hp`, `fatigue`, `slot` (index or none), FSM state, `target` (SoldierId or none), attack cooldown, and a `UnitType` handle. | REQ-VIS-002 |
| SIM-CORE-005 | Each Regiment has: stable `RegimentId`, `ArmyId`, `UnitType` handle(s), soldier list, anchor `(a, θ_a)`, `FormationTemplate` handle, formation state, current order, path, `morale`, morale state, speed mode (walk/run/march), `experience`, ability slot cooldowns, status effects, energy (SIM-ABIL-006), fire state (ranged units: mode, target regiment, volley cooldown; ammo is per soldier, SIM-PROJ-003), engagement flags. | REQ-VIS-003 |
| SIM-CORE-006 | The number of Soldier entities alive plus pending reinforcements shall never exceed 32,768. `BattleSetup` validation rejects setups above the cap; reinforcements that would exceed it are dropped with an Event. | REQ-PERF-004 |
| SIM-CORE-007 | Regiment and army sizes come from `BattleSetup`; the engine imposes no minimum or maximum except the cap. | REQ-SIM-023 |
| SIM-CORE-008 | Soldiers are removed from the world at death: Stage 15 despawns them in ascending id and drops them from every regiment list, the id lists and the spatial grid in the same tick, so no later system sees a dead soldier. The `SoldierDied` event carries the position; the application keeps a corpse from it for `combat.corpse_ticks` (render-only). | — |

### 1.1 Soldier finite state machine

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CORE-010 | Soldier FSM states: `Idle`, `MoveToSlot`, `Fighting`, `Routing`, `Withdrawing`, `Dead`. | REQ-AI-002 |
| SIM-CORE-011 | Transitions: `Idle ↔ MoveToSlot` when distance to slot crosses `movement.slot_arrive_radius` (enter Idle) or `movement.slot_leave_radius` (enter MoveToSlot); `→ Fighting` when a melee target is within reach (§6); `Fighting → MoveToSlot` when the target is lost and no other enemy within `combat.engage_radius`; `→ Routing` for every soldier the tick its regiment enters Routing or Shattered (SIM-MOR-030), `Routing → MoveToSlot` on rally (SIM-MOR-031); a Routing or Withdrawing soldier follows the escape field (SIM-FLOW-002, checked before every other branch) and leaves the battle from an edge cell. A `Fighting` soldier does not seek its slot: it seeks its target's previous-tick position and stops at `r_i + r_j + reach` (second-rank fighters a `second_rank_reach_bonus` further back), separation and obstacle avoidance still apply, and its facing tracks the target; with no target (the target died) it holds still until its next retarget tick; `→ Routing` when the Regiment enters Routing (§7); `Routing → MoveToSlot` on Rally; `→ Withdrawing` when the Regiment withdraws; `→ Dead` when `hp ≤ 0`. | REQ-AI-002 |
| SIM-CORE-012 | Soldiers make no decisions beyond this FSM. Targets, destinations, and speed mode come from the Regiment. | REQ-VIS-003 |

## 2. Determinism contract

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-DET-001 | The battle seed is a `u64` from `BattleSetup.seed`. Each system owns a stream seeded as `hash(seed, stream_id)`. Streams: `combat_melee`, `combat_ranged`, `morale`, `ai_regiment`, `ai_army`, `abilities`, `deployment`, `weather`, `campaign`. | REQ-SIM-004 |
| SIM-DET-002 | Per-entity randomness that must not depend on entity iteration order is drawn as `hash(stream_seed, tick, entity_id, draw_index)` rather than from the sequential stream. Melee hit rolls and ranged scatter use this form. | REQ-SIM-004, REQ-SIM-007 |
| SIM-DET-003 | Every system that iterates entities and produces order-dependent results iterates in ascending stable id (`SoldierId`, `RegimentId`), never in ECS storage order. | REQ-SIM-007 |
| SIM-DET-004 | The state hash at the end of a tick covers, in this order: tick number; battle phase; per Regiment (ascending id) `morale`, morale state, soldier count, anchor (position, facing), order (kind, target, target regiment, facing, speed mode, since), formation state (template, ranks, files, integrity, `morph_until`, `needs_reform`, prior template, laid-out facing), path (waypoints with corridor widths, next, requested), fire state (present only for units with `ranged`: mode, target regiment, volley cooldown), combat state (engaged, last fighting tick, charge window end, experience, kills, fled, withdrawn), casualty ring, initial strength, rout count, engaged-since tick, last front/flank/rear attack ticks, regiment fatigue mean, energy, slot cooldowns (length-prefixed), status effects (length-prefixed: source ability id, remaining, stacks, hostile); per Soldier (ascending id) `p`, `v`, facing `θ`, `hp`, `fatigue`, FSM state, slot, melee target, attack cooldown, ranged state (present only for units with `ranged`: ammo, reload cooldown), general rank (present only for the general); per Projectile (ascending id) id, shooter, shooter regiment, side, launch and land tick, start, end, apex, arc, damage, penetration (the position is derived from these, SIM-PROJ-005); the pending damage queue in queue order (apply tick, target, damage, shooter, shooter regiment); the morale shock queue in queue order (regiment, kind); per side (ascending) the visibility mask (length-prefixed, regiment order; SIM-VIS-004); the AI state (T2-080: the command outbox for the next tick, length-prefixed, each command as tick, player, seq, kind; then per side, ascending, a zero byte or a one byte followed by the army plan: stance, stance score, decided-at tick, target, line anchor, line facing, line width, formed, the assignments (length-prefixed: regiment, role with its slot or target) and the charging flag); RNG stream states. Right after the phase: the battle flow (battle start tick, pursuit start tick, winner, ended-at tick; SIM-FLOW-010..015, 018), then per side (ascending index) escape edge, deployment confirmed, defeated, surrendered, reinforcement groups spawned, general soldier, general regiment, general dead. Positions are hashed by their `S` bit pattern. (Phase 1 layout fixed in T1-047; the combat fields were appended in T2-020; the regiment ammo gave way to the fire, ranged and projectile fields in T2-030; the morale-slice fields and the side state joined in T2-040, declared once for T2-041..043; the milestone 4 fields (battle flow, side flags, energy, cooldowns, statuses, withdrawn, visibility masks) joined in T2-050, declared once for T2-050..070; `ended_at` was appended to the battle flow in T2-071; the AI state joined in T2-080, declared once for T2-080..082.) | REQ-SIM-005 |
| SIM-DET-005 | A snapshot contains everything the hash covers plus everything needed to continue (paths included, since a re-requested path would differ from the one in flight); cooldowns, status effects (by source ability id, re-resolved on restore), energy, the battle-flow timers, the visibility masks and last-sighting memory, the projectiles in flight, the pending damage queue and the AI state (its outbox and army plans, T2-080) are stored; the cached status multipliers are recomputed on restore; spatial and nav grids, flow fields, slot tables, ranks, attacker counts and the per-tick targeting gates are recomputed on restore (derived data is never stored). Restoring and stepping shall produce the same hash sequence as the uninterrupted run. | REQ-SIM-006 |
| SIM-DET-006 | No system reads wall-clock time, thread ids, allocation addresses, or environment. | REQ-TECH-008 |
| SIM-DET-007 | The stage order of §6.2 in the SAD is part of the determinism contract. | REQ-SIM-001 |
| SIM-DET-008 | Pause and speed multipliers do not exist inside the sim; they are app-level accumulator behaviour, but the `Pause`/`SetSpeed` Commands are recorded in the stream so replays and peers reproduce the player's experience. The sim applies them as no-ops. | REQ-SIM-031 |

## 3. Command model

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CMD-001 | A Command is `{ tick, player, seq, kind }`. Commands are applied at Stage 0 of `tick`, sorted by `(player, seq)`. Commands for past ticks are rejected with an Event (never silently reordered). | REQ-SIM-003, REQ-NET-001 |
| SIM-CMD-002 | Command kinds (battle): `Move { regiments, target, facing, speed_mode }`, `AttackRegiment { regiments, target_regiment }`, `AttackMove { regiments, target }`, `Halt { regiments }`, `SetFormation { regiments, template, ranks }`, `SetFacing { regiments, facing }`, `SetSpeedMode { regiments, mode }`, `GroupFormation { regiments, group_template, anchor, facing, width }`, `FireMode { regiments, mode }` (fire_at_will / hold / target), `UseAbility { regiment, ability, target }`, `Withdraw { regiments }`, `Deploy { regiment, position, facing, template }`, `ConfirmDeployment`, `Pause`, `SetSpeed { mult }`, `Surrender`, `TransferControl { from, to }` (hands every regiment of `from` to `to`; `to = 255` means engine AI; used for drop-to-AI in multiplayer and for "let the AI command this side" in single-player: `il_app --ai <player>` issues it as the engine at tick 1, T2-081). | REQ-INP-006, REQ-SIM-030..033, REQ-NET-008 |
| SIM-CMD-003 | A Command referencing a Regiment not owned by `player` is rejected with an Event. AI players own their factions' regiments; `PlayerId(255)` is the engine AI and may own regiments transferred to it. | REQ-NET-001 |
| SIM-CMD-004 | A Command referencing a Routing or Shattered regiment is rejected (`Routing`) except `Withdraw`, which is accepted and ignored for that regiment (T2-070); Routing regiments cannot be ordered (SIM-MOR-030). | REQ-MOR-004 |
| SIM-CMD-005 | AI decisions are emitted as Commands for tick `t + 1` during Stage 1 of tick `t`, tagged `PlayerId(255)`, and pass through the same validation: they wait in the AI outbox (state, SIM-DET-004) and `step` appends them to the next tick's inbox before Stage 0; `StepOutput.ai_commands` returns them for the replay log (T2-080, SIM-AI-002). | REQ-AI-005 |

## 4. Formations

### 4.1 Slot layout

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FORM-001 | A Regiment's formation is defined by `(template, n, ranks, θ_a, a)`: template handle, soldier count, chosen rank count, facing, anchor. The layout function returns `n` slot offsets `o_i` in the formation's local frame (x = right, y = forward), in metres. World slot position `s_i = a + R(θ_a) · o_i`, where `R(θ_a)` maps local forward onto the facing direction `(cos θ_a, sin θ_a)` and local right onto `(sin θ_a, −cos θ_a)` (facing `0` is +x, TDD §2.2 `Angle`). | REQ-FORM-001 |
| SIM-FORM-002 | `files = ceil(n / ranks)`. Spacing: `sf = template.spacing_file × unit.soldier_radius × 2`, `sr = template.spacing_rank × unit.soldier_radius × 2`. | REQ-FORM-001 |
| SIM-FORM-003 | **Line**: slot `(k)` for `k in 0..n`: rank `q = k / files`, file `f = k % files`; `o = ((f − (files−1)/2) · sf, −q · sr)`. Front rank is `q = 0` at `y = 0`; the anchor is the centre of the front rank. The last rank may be short; its slots are centred. | REQ-FORM-002 |
| SIM-FORM-004 | **Column**: Line with `files = template.default_files_column` (default 4) and ranks derived. | REQ-FORM-002 |
| SIM-FORM-005 | **Square**: four outward-facing sides of `⌊n/4⌋` soldiers each, the remainder `n − 4·⌊n/4⌋` joining the rear side; each side is `depth = min(ranks, ⌈n/4⌉)` deep (rows inset by `sr` toward the centre) and holds `ceil(count / depth)` files at `sf`, with a corner band of `depth · sr` at both ends of every side so the sides never overlap; the side length is therefore `ceil((⌊n/4⌋ + remainder) / depth) · sf + 2 · depth · sr` (the rear side is the longest and sets it). The front side is the front rank at `y = 0` (the anchor is its centre) and the square extends one side length behind it; facing offsets are 0 (front, +y), −90° (right, +x), 180° (rear, −y), +90° (left, −x). | REQ-FORM-002 |
| SIM-FORM-006 | **Wedge**: rank `q` has `2q + 1` slots centred on the axis, spacing `sf`, until `n` is placed; the last rank is centred. Anchor is the apex. | REQ-FORM-002 |
| SIM-FORM-007 | **Phalanx**: Line with `spacing_file` and `spacing_rank` from the template (tighter defaults, §15) and `template.min_ranks` enforced (default 4). Grants `second_rank_attack` regardless of unit flag when ranks ≥ 2 (SIM-CMBT-012). | REQ-FORM-002 |
| SIM-FORM-008 | **Loose**: Line with spacing multiplied by `template.loose_mult` (default 2.0). | REQ-FORM-002 |
| SIM-FORM-009 | **Custom**: `template.custom_slots` gives offsets in units of `2 × soldier_radius`; if `n` exceeds the list, extra soldiers form a Line behind. | REQ-FORM-002 |
| SIM-FORM-010 | Slot facing equals `θ_a` for all templates except Square. | REQ-FORM-001 |
| SIM-FORM-011 | For mixed regiments, `template.role_zones` assigns rank ranges to unit categories; slots in a zone are only assigned to soldiers of that category; overflow of a category spills to the nearest rank of any zone. | REQ-FORM-008 |

### 4.2 Reform and resize

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FORM-020 | A reform is triggered when soldier count changes, the template or rank count changes, `θ_a` changes by more than `formation.reform_angle` (default 10°), or a `Move` order is issued. | REQ-FORM-003, REQ-FORM-004 |
| SIM-FORM-021 | Resize on death: slots are recomputed with the new `n` keeping `ranks` if `files ≥ template.min_files` (default 2), else `ranks` decreases. Vacated front-rank slots are filled by soldiers from the rearmost rank (closing from the rear). | REQ-FORM-003 |
| SIM-FORM-022 | Assignment algorithm: soldiers sorted by ascending id; slots sorted by rank then file. Step 1: any soldier whose current slot still exists and is within `formation.keep_slot_radius` (default 1.5 m) keeps it. Step 2: remaining soldiers are assigned greedily to the nearest free slot, processing soldiers in ascending id, using the spatial grid to find candidates within `formation.assign_search_radius` (default 30 m); if none, the nearest free slot by brute force. Step 3: up to `formation.swap_passes` (default 2) passes over all pairs within one rank swap assignments if it reduces total squared distance. | REQ-FORM-004 |
| SIM-FORM-023 | Reform cost bound: the assignment for a regiment of 500 soldiers shall complete within 0.5 ms; the TDD verifies by benchmark. | REQ-PERF-005 |
| SIM-FORM-024 | Facing change (wheel): a `SetFacing` order rotates `θ_a` toward the target at `movement.wheel_rate` (default 45°/s) while soldiers track their moving slots. Turn-in-place: if the regiment is halted and `|Δθ| > formation.turn_in_place_angle` (default 120°), the anchor is instead re-placed so that the rear rank becomes the front rank and slot assignment is reformed (about-face). | REQ-FORM-005 |

### 4.3 Integrity and morphing

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FORM-030 | Formation integrity `I ∈ [0,1]` = fraction of soldiers within `formation.integrity_radius` (default `1.0 × sf`) of their slot, computed every `formation.integrity_period_ticks` (default 5). | REQ-FORM-006 |
| SIM-FORM-031 | Integrity modifies combat: attack `× (1 + template.integrity_bonus_attack × I)`, defence `× (1 + template.integrity_bonus_defence × I)`. Integrity below `formation.integrity_morale_threshold` (default 0.5) contributes a morale factor (§7). | REQ-FORM-006 |
| SIM-FORM-032 | Morphing: a `SetFormation` order to a different template starts a transition of `template_new.morph_ticks` during which `I` is computed against the new slots and the regiment's speed is `× formation.morph_speed_mult` (default 0.5). Soldiers move to new slots immediately; there is no intermediate template. | REQ-FORM-007 |
| SIM-FORM-033 | A regiment engaged in melee cannot morph to Square or Phalanx (order rejected with Event); it may morph to Line or Loose. | REQ-FORM-007 |

### 4.4 Group formations

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FORM-040 | A `GroupFormation` command applies a group template (`battle_line`, `double_line`, `echelon_left`, `echelon_right`, `refused_left`, `refused_right`, `custom`) to a set of regiments: the template assigns each regiment an anchor and facing given the group anchor, facing, and desired width. Regiments are ordered along the line by their current lateral position to minimise crossing. | REQ-FORM-009 |
| SIM-FORM-041 | `battle_line`: regiments side by side with gap `formation.group_gap` (default 6 m), widths from each regiment's current formation width; ranged regiments are placed in front by `formation.skirmish_offset` (default 20 m) if `group.skirmishers_forward` is set; cavalry on the flanks. | REQ-FORM-009 |
| SIM-FORM-042 | The player's drag-formation gesture produces a `GroupFormation { battle_line, width }`: the engine chooses `ranks` per regiment so that the total width matches the drag width within `formation.width_tolerance` (default 10 %), clamped to `[min_ranks, max_ranks]`: every regiment starts at its fewest ranks and the widest regiment gains one rank at a time until the line fits. Geometry of the other kinds: `double_line` alternates regiments into lines `2 · group_gap` apart; `echelon_left`/`echelon_right` step each successive regiment toward the named flank `2 · group_gap` back; `refused_left`/`refused_right` pull the flank-most regiment on the named side `3 · group_gap` back and turn it 45° inward. | REQ-FORM-010, REQ-INP-003 |

## 5. Movement and pathfinding

### 5.1 Nav grid and regiment paths

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOVE-001 | The nav grid has cell size `movement.nav_cell` (default 4 m). A cell is impassable if any part of it lies in a `rock` zone, a river not at a ford or bridge, a wall, or a closed gate. Cell cost = the largest zone `move_cost` (§5.4) among the zone cells inside it; slope is not part of the cost (it scales speed only, SIM-MOVE-030; Phase 1 decision 11). | REQ-PATH-001, REQ-PATH-002 |
| SIM-MOVE-002 | Regiment paths are computed by A* (Phase 1) or HPA* (Phase 3) from the anchor to the target on the nav grid with 8-connectivity and octile heuristic. The path is a list of waypoints after string-pulling (line-of-walkability smoothing). | REQ-PATH-001, REQ-PATH-002 |
| SIM-MOVE-003 | HPA* clusters are `movement.hpa_cluster` cells square (default 16); gates are maximal passable runs along cluster borders, one gate node per run at its centre plus at ends if the run exceeds `movement.hpa_gate_split` (default 6 cells). Intra-cluster costs are precomputed at map load; the abstract graph is searched first, then each cluster segment is refined with A*. | REQ-PATH-001 |
| SIM-MOVE-004 | Each waypoint stores the passable corridor width of its nav cell (`min(passable_run_x, passable_run_y) · nav_cell`). When the regiment's width (`files · sf`) exceeds the corridor of the waypoint it is heading for, it morphs to the first Column template in its unit's `formations` (remembering the prior template) and morphs back once no remaining waypoint is narrower than the prior formation's width, or on arrival. Automatic morphs carry no `morph_speed_mult` penalty. | REQ-PATH-006, REQ-SIM-042 |
| SIM-MOVE-005 | At most `movement.paths_per_tick` (default 8) new path requests are served per tick, in ascending regiment id; the rest wait with the regiment stationary. | REQ-PERF-005 |
| SIM-MOVE-006 | When gates change state or a wall segment is destroyed (Phase 5), affected nav cells are updated and the HPA* clusters touching them are recomputed; paths crossing them are invalidated and re-requested. | REQ-PATH-007 |

### 5.2 Regiment path following

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOVE-010 | The anchor moves toward the current waypoint at regiment speed `v_reg` (SIM-MOVE-020); a waypoint is reached within `movement.waypoint_radius` (default 2 m). At each waypoint the desired facing becomes the direction to the next waypoint; `θ_a` wheels at `movement.wheel_rate` while moving. | REQ-PATH-006 |
| SIM-MOVE-011 | Regiment speed is the minimum of the unit type speed for the speed mode and the speed of its slowest soldier category in mixed regiments, times `fatigue_speed_mult(F_mean)` (SIM-FAT-004/005) and the morale state's speed multiplier (SIM-MOR-004), times formation `speed_mult`, times terrain and slope factors at the anchor (the fatigue and morale terms since T2-040, so the anchor slows with its soldiers). | REQ-PATH-005 |
| SIM-MOVE-012 | Cohesion: if the fraction of soldiers farther than `movement.straggler_radius` (default `3 × sf`) from their slot exceeds `movement.straggler_fraction` (default 0.25), the anchor speed is scaled by `movement.straggler_slowdown` (default 0.5) until they catch up. | REQ-PATH-006 |
| SIM-MOVE-013 | On arrival at the final target the regiment sets `θ_a` to the ordered facing (if given) and stops; soldiers finish moving to slots. | REQ-FORM-005 |

### 5.3 Soldier steering

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOVE-020 | Soldier max speed: `v_max = unit.speed_<mode> × fatigue_speed_mult(F) × morale_speed_mult(state) × zone.move_mult × slope_mult × status_speed_mult` (`F` the soldier's own fatigue, the morale multiplier its regiment's state row of SIM-MOR-004, the status multiplier its regiment's `speed` of SIM-ABIL-005; T2-050). The anchor speed of SIM-MOVE-011 carries the same status multiplier. Speed modes: `walk`, `run`, `march` (march is walk speed with reduced fatigue accumulation and cannot be used within `combat.engage_radius` of an enemy). | REQ-PATH-005, REQ-FAT-003 |
| SIM-MOVE-021 | Desired velocity for `MoveToSlot`: `seek = (s − p)`, `v_des = seek.normalised × min(v_max, |seek| / dt × movement.arrive_damping)` (default 0.5). | REQ-PATH-003 |
| SIM-MOVE-022 | Separation: for each neighbour `j` within `2r_i + 2r_j + movement.sep_margin` (default 0.2 m), add `(p_i − p_j).normalised × movement.sep_weight × (1 − d / (2r_i + 2r_j + sep_margin))`. Neighbours are taken from the spatial grid in ascending id, at most `movement.sep_max_neighbours` (default 8) nearest. | REQ-PATH-003 |
| SIM-MOVE-023 | Obstacle avoidance: if the segment `p → p + v_des × dt × movement.lookahead_ticks` (default 4) crosses an impassable nav cell, `v_des` is rotated toward the nearest passable direction sampled at ±15°, ±30°, ±45°, ±60°, ±90° (first that is clear, in that order). | REQ-PATH-003 |
| SIM-MOVE-024 | Final velocity `v = clamp_length(v_des + separation, v_max)`. The soldier's facing `θ` tracks the slot facing when within `slot_arrive_radius`, else the velocity direction, turning at most `movement.soldier_turn_rate` (default 360°/s). | REQ-PATH-003 |
| SIM-MOVE-025 | `Fighting` soldiers do not seek their slot; they hold position against their target with separation only, and face the target. | — |

### 5.4 Terrain effects

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOVE-030 | Slope along the movement direction `g = (h(p + d) − h(p)) / |d|` (rise over run, `d` is 1 m ahead). `slope_mult = clamp(1 − movement.slope_penalty × max(g, 0) + movement.slope_bonus × max(−g, 0), movement.slope_min_mult, movement.slope_max_mult)`; defaults 2.0, 0.5, 0.4, 1.2. | REQ-SIM-040 |
| SIM-MOVE-031 | Zone types and data fields per type: `move_mult`, `move_cost`, `los_mult`, `conceal` (bool), `fatigue_mult`, `formation_integrity_mult`, `passable`, `crossing` (bool: river cells under a polygon of this type are passable). Built-in types: open, road, forest, marsh, rock (impassable), ford, bridge (both `crossing`). Mods may add types. A map names a `base_zone` for the ground outside every polygon; zone polygons are rasterised at `movement.zone_cell` cell centres, later polygons overriding earlier ones. | REQ-SIM-041 |
| SIM-MOVE-032 | Rivers are polylines with width; the zone cells whose centre lies within half the width of the polyline are river cells, impassable except where a `crossing` zone polygon (ford, bridge) covers them. Fords: `move_mult` 0.5, defence `× movement.ford_defence_mult` (default 0.7). Bridges: passable width equals the bridge polygon width; SIM-MOVE-004 applies. | REQ-SIM-042 |
| SIM-MOVE-033 | Walls are impassable line segments with height; gates are segments with `open/closed/destroyed` state; both are stored in the map format from Phase 1 and inert until Phase 5. | REQ-SIM-045, REQ-SIM-043 |

### 5.5 Collision

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOVE-040 | After integration, for every pair `(i, j)` with `i < j` (ascending id) from the spatial grid with `d = |p_i − p_j| < r_i + r_j`: overlap `o = r_i + r_j − d`; push `i` by `−n × o × m_j / (m_i + m_j)` and `j` by `+n × o × m_i / (m_i + m_j)` where `n = (p_j − p_i)/d`. Pushes are accumulated into per-soldier buffers and applied after all pairs are processed. | REQ-SIM-024 |
| SIM-MOVE-041 | The collision pass runs `movement.collision_iterations` (default 2) times. | REQ-SIM-024 |
| SIM-MOVE-042 | Positions are clamped to the map rectangle. A move (integration or collision push) whose destination cell is impassable is retried with its x component only, then its y component only, and otherwise the soldier stays where it was (deterministic push-out, Phase 1 plan S12). | REQ-SIM-024 |
| SIM-MOVE-043 | Charging soldiers (regiment in `run` mode within `combat.charge_window_ticks` of first contact) push with `m × combat.charge_mass_mult` (default 2.0). | REQ-CMBT-005 |

### 5.6 Flow fields

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FLOW-001 | At battle start each side gets an escape flow field over the nav grid toward its own map edge: a multi-source Dijkstra from every passable cell of that edge over the A\* graph (same costs, no corner cutting), storing per cell the direction (one of the eight neighbour offsets) to the neighbour with the strictly lowest cost, first in neighbour order on a tie; edge cells point off the map, unreachable and impassable cells have no direction. The side's escape edge is the map edge nearest the mean vertex of its deployment polygon (ties West, East, South, North; West without a polygon), stored in `SideState.escape_edge` and hashed; on `rome:test_field` zone 0 escapes South and zone 1 North (T2-042). | REQ-PATH-004 |
| SIM-FLOW-002 | Routing and Withdrawing soldiers set `v_des = field(p) × v_max` (the direction of the nav cell under `p`, no arrive damping; Routing at `run`, Withdrawing at `march`) and apply separation and avoidance as normal; the facing tracks the velocity; without a direction only separation acts. When a soldier stands in a cell of its side's escape edge at Stage 15 (after the deaths, so a soldier killed on that tick is a death, never both) it leaves the battle: it is removed like the dead (no corpse, no casualty ring, no kill credit), `Combat.fled` counts it and `SoldierFled` carries its position (Routing: `Combat.fled` and `SoldierFled`; Withdrawing: `Combat.withdrawn` and `SoldierWithdrew`, a survivor; T2-070). | REQ-PATH-004, REQ-SIM-033 |
| SIM-FLOW-003 | Flow fields are derived data (SIM-DET-005): rebuilt with the nav grid on `new` and `restore`, and recomputed only when the nav grid changes (SIM-MOVE-006). | — |

## 6. Combat

### 6.1 Engagement and targeting

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CMBT-001 | A soldier is in melee reach of enemy `j` if `|p_i − p_j| ≤ r_i + r_j + unit_i.reach`. | REQ-CMBT-002 |
| SIM-CMBT-002 | Melee targeting runs every `combat.retarget_period_ticks` (default 4) per soldier (staggered by `id % period`): if the current target is alive and within `r_i + r_j + reach + combat.reach_slack` (default 0.5 m), keep it; else choose, among enemy soldiers whose centre lies within `combat.engage_radius` (default 3 m) in the spatial grid, the one with the fewest attackers (`attacker_count` ascending), then the nearest, then the lowest id. Soldiers without a target within `engage_radius` return to `MoveToSlot`. Only Idle, MoveToSlot and Fighting soldiers of regiments that may fight (Idle or attacking order, not Routing/Shattered, soldiers left) take part, and only when an enemy regiment lies within the two regiments' extents plus `engage_radius` of the anchor (a per-regiment gate, so distant armies cost no per-soldier work). Attacker counts are recomputed after targeting in ascending soldier id. | REQ-CMBT-003 |
| SIM-CMBT-003 | A regiment is engaged if any soldier is `Fighting` (recomputed after targeting; the false-to-true edge emits `Engaged`). Engaged regiments ignore `Move` orders' facing but obey the move (disengage), taking a morale penalty (SIM-MOR-025). While an attacking regiment (SIM-CMBT-004) is engaged its anchor holds and its path is kept; pursuit re-paths once no soldier has fought for `combat.retarget_period_ticks`. | — |
| SIM-CMBT-004 | Regiments in `AttackRegiment` or `AttackMove` orders path to the target regiment's anchor (re-pathed every `combat.pursue_repath_ticks`, default 20, staggered by regiment id and also on the tick the order is issued; the order stores the target regiment; reaching the anchor of a target that has moved does not end the order; an `AttackRegiment` whose target has no living soldiers halts; `AttackRegiment` is rejected with `InvalidTarget` for an own-side or empty target) and switch to `run` within `combat.charge_distance` (default 30 m) if `unit.charge_bonus > 0`; the speed mode stays `run` until another order changes it. | REQ-CMBT-005 |
| SIM-CMBT-005 | An `AttackMove` regiment has no target regiment until, on one of its re-path ticks, an enemy regiment with living soldiers, visible to the regiment's side (SIM-VIS-004, T2-060), has its anchor within `combat.attack_move_radius` (default 40 m) of the regiment's anchor; the nearest such regiment (ties by ascending id) becomes the target and SIM-CMBT-004 applies. When the target has no living soldiers the regiment resumes the move to its original point. | REQ-CMBT-005 |

### 6.2 Melee resolution

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CMBT-010 | Each `Fighting` soldier has a cooldown; when it reaches 0 it attacks its target and resets to `unit.attack_interval_ticks × fatigue_interval_mult(F) × morale_interval_mult(M) × status_interval_mult`, rounded to nearest tick, minimum 2 (the status multiplier is the attacker regiment's `attack_interval` of SIM-ABIL-005, T2-050; it scales the ranged reload of SIM-PROJ-003 too). Initial cooldown on entering `Fighting` is `rng-free`: `(id % attack_interval_ticks)` to stagger. | REQ-CMBT-001, REQ-CMBT-007 |
| SIM-CMBT-011 | Hit roll: `A = unit_i.attack × fatigue_attack_mult(F_i) × morale_attack_mult(M_i) × (1 + template_i.integrity_bonus_attack × I_i) × charge_mult × experience_mult × status_attack_mult_i`; `D = unit_j.defence × fatigue_defence_mult(F_j) × morale_defence_mult(M_j) × (1 + template_j.integrity_bonus_defence × I_j) × flank_defence_mult × terrain_defence_mult × status_defence_mult_j`. Hit probability `P = clamp(combat.base_hit + combat.hit_scale × (A − D) / (A + D), combat.min_hit, combat.max_hit)`; defaults 0.5, 0.5, 0.05, 0.95. The attack hits if `rng.combat_melee(tick, id_i, 0) < P` (draw index 0; later draws use 1, 2, …). The status multipliers are each regiment's own (SIM-ABIL-005, T2-050) and the aura multiplier SIM-GEN-002's (T2-043); a braced anti-cavalry defender attacking cavalry (SIM-CMBT-015) multiplies `A` by `1 + anti_cavalry_bonus`. | REQ-CMBT-001 |
| SIM-CMBT-012 | Second-rank attack: a soldier whose slot is in rank 1 (second rank) and whose unit has `second_rank_attack` (or is in Phalanx) may target enemies in reach of the soldier in the slot directly ahead, using its own `reach + combat.second_rank_reach_bonus` (default 1.0 m). | REQ-CMBT-003 |
| SIM-CMBT-013 | Damage on hit: `dmg = max(unit_i.damage × charge_dmg_mult × flank_dmg_mult × experience_mult × status_damage_mult_i − armour_j × (1 − unit_i.armour_penetration), combat.min_damage)` (default 1), with `armour_j = unit_j.armour × armour_mult_j + armour_add_j` from the defender's statuses (SIM-ABIL-005, T2-050); `armour_penetration` is the unit's top-level melee field (default 0), distinct from `ranged.armour_penetration`. `hp_j −= dmg`. | REQ-CMBT-001 |
| SIM-CMBT-014 | Frontal arc: an attack is frontal if the attacker lies within `±unit_j.frontal_arc_deg / 2` (default 120°) of the defender's facing; flank if within ±150° (an engine constant, `FLANK_HALF_ARC_DEG`); rear otherwise. The arc is measured from the defending soldier's own facing, which tracks its target while it fights (SIM-CORE-011), so a flank or rear attack stays one only until the defender turns. `flank_dmg_mult` and `flank_defence_mult`: front 1.0/1.0, flank `combat.flank_dmg_mult` (1.25) / `combat.flank_def_mult` (0.8), rear `combat.rear_dmg_mult` (1.5) / `combat.rear_def_mult` (0.6). | REQ-CMBT-004 |
| SIM-CMBT-015 | Charge: when a regiment in `run` mode first gains an engaged soldier (the tick `engaged` turns true while the speed mode is `run` and no window is open; `charge_until` on the regiment marks the window's end and a `Charge` event names the regiment its first fighter struck), all its soldiers get `charge_mult = 1 + unit.charge_bonus` and `charge_dmg_mult = 1 + unit.charge_bonus × combat.charge_dmg_share` (default 0.5) for `combat.charge_window_ticks` (default 60). A defender unit with `anti_cavalry_bonus > 0`, not moving (an Idle order, or engaged), with `I ≥ combat.brace_integrity` (default 0.7), facing the charge within its frontal arc, negates the attacker's charge bonus if the attacker is cavalry and gains `attack × (1 + anti_cavalry_bonus)` versus cavalry. Charge push: while the window is open the regiment's soldiers push with mass `unit.mass × combat.charge_mass_mult` (default 2.0) in collision resolution (SIM-MOVE-040), so a charge shoves lighter defenders back without any extra force. | REQ-CMBT-005, REQ-CMBT-006 |
| SIM-CMBT-016 | Terrain defence: `terrain_defence_mult = zone.defence_mult × ford_mult × (1 + combat.height_defence × sat((h_j − h_i) / combat.height_ref))` where `zone` is the defender's zone type (`defence_mult` default 1; forest 1.1, marsh 0.8), `ford_mult = movement.ford_defence_mult` when that zone type has `ford: true` and 1 otherwise (SIM-MOVE-032), and `sat` clamps to [−1, 1]; defaults 0.15 and 5 m. | REQ-SIM-040 |
| SIM-CMBT-017 | Experience: regiment `experience` in [0, 9]; `experience_mult = 1 + combat.exp_step × experience` (default 0.03). | REQ-CAMP-042 |
| SIM-CMBT-018 | Attack results (hit or miss, damage, arc) are recorded in a shared buffer during the parallel phase, sorted by attacker id and applied in that order (one attack per attacker per tick makes the order total); a soldier whose hp crosses zero is queued with its killer and the killer's regiment for Stage 15, and damage on a soldier already at or below zero is applied but credits nobody. Projectile damage lands after the melee outcomes, at Stage 11 (SIM-PROJ-006), under the same crossing rule. Deaths are resolved in Stage 15: the queued kills sorted by victim id, each leaving its regiment's soldier list and slot assignment (`needs_reform` set, SIM-FORM-021), adding one to the regiment's casualty ring slot of the tick (SIM-MOR-010) and one kill to the killer's regiment, clearing every melee target that pointed at it, then despawned. | REQ-SIM-008 |

### 6.3 Ranged and projectiles

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-PROJ-001 | A ranged regiment in `fire_at_will` selects, every `combat.ranged_retarget_ticks` (default 10; staggered by regiment id, and at once when it has no target), the visible enemy regiment with the most soldiers inside its range annulus `[min_range, range × range_mult]` measured from the shooter's anchor (indirect arc ignores LOS occlusion by soldiers but not terrain; "visible" per SIM-VIS-004, T2-060), keeping the current target while it still has a soldier there and is still visible, and taking the lower id on ties. `target` mode uses the ordered regiment while it has a soldier in the annulus and falls back to `fire_at_will` once the ordered regiment has no living or visible soldiers; `hold` fires nothing. Each ranged regiment carries `Fire { mode, target, cooldown }`; `FireMode` at Stage 0 rejects a regiment without `ranged` (`NotRanged`) and a `target` that is own-side or empty (`InvalidTarget`) or hidden (`NotVisible`, T2-060), and clears the target so the new mode re-acquires it at the next Stage 9. | REQ-CMBT-013 |
| SIM-PROJ-002 | `range_mult = 1 + combat.height_range × clamp((h_shooter − h_target) / combat.height_ref, −1, 1)` (default 0.2). Target selection reads the two anchors' heights; a soldier's own shot reads its position and the aimed soldier's. | REQ-CMBT-014 |
| SIM-PROJ-003 | Each soldier with `ammo > 0` (per soldier, `RangedState { ammo, cooldown }`, from `unit.ranged.ammo`), in state `Idle` or `MoveToSlot` under any order (not `Fighting`, `Routing` or `Withdrawing`), whose regiment has a target, throws when its reload cooldown reaches 0 (reset to `unit.ranged.reload_ticks × fatigue_interval_mult`, rounded as SIM-CMBT-010), aiming at the position of a target soldier chosen deterministically (`rng.combat_ranged(tick, id, 1)`-th soldier of the target regiment by ascending id, draw index 1 of the `combat_ranged` stream in the SIM-DET-002 form) predicted forward by flight time; a pick outside the soldier's own annulus `[min_range, range × range_mult]` gives way to the nearest soldier of the target regiment inside it (ties lowest id), and with none there the soldier keeps its ammo and waits. A regiment fires in volleys by sharing the cooldown phase (`combat.volley` true: the regiment's `Fire.cooldown` gates every soldier, resets to `reload_ticks ×` the largest fatigue interval multiplier among the volley's shooters, and counts down after the reset so the period is exactly `reload_ticks`); with `volley` false each soldier's own cooldown counts. `ammo −= 1` per throw. The shots of a tick are recorded in parallel and turned into projectiles in ascending shooter id; one `VolleyFired { regiment, count }` per regiment per tick. | REQ-CMBT-010, REQ-CMBT-011 |
| SIM-PROJ-004 | Scatter: the aim point is offset by a vector with angle `rng.combat_ranged(tick, id, 0) × 2π` (draw index 0) and length `d × (1 − unit.ranged.accuracy) × combat.scatter_scale × weather.accuracy_penalty` (default scale 0.15) where `d` is the distance to the aimed soldier; `weather.accuracy_penalty` is `1` until the weather rules of Phase 4. | REQ-CMBT-011 |
| SIM-PROJ-005 | Projectile motion: a `direct` arc flies straight at `projectile_speed` with height following a shallow parabola (apex `combat.direct_apex`, default 2 m); an `indirect` arc launches at 45° with the speed that lands at `d` (`sqrt(d × g)`, `g = combat.gravity` default 9.81, capped at `projectile_speed`), so its flight time is `d × √2 / v` and its apex `d / 4`. The flight time is rounded up to whole ticks (at least 1) and the landing point, apex and `land_tick` are fixed at launch; the position at tick `t` is the closed form `start + (end − start) × u`, height `apex × 4u(1 − u)`, `u = (t − launch) / (land − launch)` (nothing is integrated, nothing derived is stored, T2-030/031). A projectile lands when its `land_tick` arrives. | REQ-CMBT-010 |
| SIM-PROJ-006 | Landing (Stage 11, in ascending projectile id): query the spatial grid at the landing point for soldiers (any side) with `|p − land| ≤ r + combat.projectile_radius` (default 0.3 m); the nearest by distance (ties ascending id) is hit. A hit applies `dmg = max(ranged.damage − armour × (1 − ranged.armour_penetration), combat.min_damage) × dmg_mult(arc)` (`armour` the victim's `unit.armour` under its regiment's statuses, SIM-ABIL-005) with the arc of SIM-CMBT-014 read from the victim's facing toward the shooter's launch point (`combat.flank_dmg_mult`, `combat.rear_dmg_mult`); if `unit.shield` and the impact is frontal, `dmg × combat.shield_mult` (default 0.5). The damage is queued (`PendingDamage`: apply tick, target, damage, shooter, shooter regiment) and applied the same tick in `(apply tick, target id)` order, queue order breaking ties, after the melee outcomes of Stage 10; a soldier whose hp crosses zero is queued for Stage 15 with the shooter and its regiment (which is credited even if the shooter has since fallen), and a soldier already at or below zero credits nobody twice. Every landing emits `ProjectileLanded { pos, hit, victim }`. | REQ-CMBT-010, REQ-CMBT-012 |
| SIM-PROJ-007 | Projectiles in flight are blocked by walls higher than their current `z` at the crossing point (Phase 5) and never by soldiers. | REQ-SIM-043 |
| SIM-PROJ-008 | Cap: a shot taken while `live_projectiles ≥ combat.projectile_cap` (default 8,192; checked per shot in ascending shooter id, so a volley may split at the cap) is resolved statistically: it hits with probability `P_hit = combat.stat_hit_base × density(target_regiment)`, where density is the target's living soldiers per m² of its footprint (the bounding box of its formation slots grown by the soldier radius; the circle of its `extent` when it has no layout) clamped to [0, 1], rolled as `rng.combat_ranged(tick, id, 2) < P_hit` (draw index 2, the same shooter slots as the simulated path, so hash sequences remain comparable); the victim is the target regiment's soldier nearest the scattered aim point of SIM-PROJ-004 (ties lowest id; only the target regiment, so this path never hits friends), the damage is that of SIM-PROJ-006 with the arc read from the victim's facing toward the shooter, and it is queued for the tick the projectile would have landed. Ammo, cooldowns and `VolleyFired` are as for a simulated shot; no `ProjectileLanded` is emitted (REQ-CMBT-015: nothing is visible). | REQ-CMBT-015, REQ-PERF-008 |
| SIM-PROJ-009 | Friendly soldiers at the landing point are hit like enemies; indirect fire may be ordered over friendly regiments; a direct shot is refused if a friendly regiment's footprint (the circle around its anchor of radius `extent`, its farthest soldier's distance) intersects the first `combat.friendly_block_dist` (default 15 m) of the shooter's line of fire. The refused soldier keeps its ammo and its cooldown stays at zero, the fire mode is unchanged, and one `FireBlocked { regiment, blocker }` event is emitted per regiment per tick, so fire resumes by itself when the line clears (plan decision, T2-030). | REQ-CMBT-012, REQ-CMBT-013 |

## 7. Morale

### 7.1 Value and states

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOR-001 | Regiment morale `M ∈ [0, 100]`, initialised to `unit.morale_base × (1 + morale.exp_bonus × experience)`, clamped; the general's aura acts only through the per-second factor (SIM-MOR-013). The starting state is the band `M` falls into (SIM-MOR-003): hastati at 60 spawn Unsettled and recover to Steady in the first seconds when no enemy stands within `safe_radius`. | REQ-MOR-001 |
| SIM-MOR-002 | Each tick: `M ← clamp(M + Σ_f w_f × x_f × dt_s, 0, 100)` where `dt_s = 0.05` and factors `f` are in §7.2 with weights `morale.w_<factor>` (points per second at full effect). As built (T2-041): Stage 14 gathers every regiment's factor inputs (`MoraleInputs`) from the state as it stood at the start of the stage, then applies, in ascending regiment id, first the queued one-time shocks (`MoraleShocks`, queue order) and then the factor delta, and finally the SIM-MOR-003 transition. Regiments with no soldiers are skipped. The casualty ring Stage 15 writes is read one tick later (the schedule order stays). | REQ-MOR-003 |
| SIM-MOR-003 | States by thresholds with hysteresis `morale.hysteresis` (default 5): Steady `M > t_unsettled`; Unsettled `t_shaken < M ≤ t_unsettled`; Shaken `t_broken < M ≤ t_shaken`; Broken `t_routing < M ≤ t_broken`; Routing `M ≤ t_routing`. Defaults 70 / 50 / 30 / 15. A state is left upward only when `M` exceeds the threshold plus hysteresis, one state per tick; downward a regiment drops straight to the band `M` falls into. Routing is entered by this rule (SIM-MOR-030 applies) but left only by rally (SIM-MOR-031) or shatter (SIM-MOR-032), never by threshold; Shattered is entered only by SIM-MOR-032. | REQ-MOR-002 |
| SIM-MOR-004 | Morale multipliers per state (data table `morale.state_mults`): attack, defence, attack interval, speed. Defaults: Steady 1/1/1/1; Unsettled 0.95/0.95/1.05/1; Shaken 0.85/0.85/1.15/1; Broken 0.7/0.7/1.3/1; Routing 0/0.5/—/1.1. | REQ-CMBT-007 |

### 7.2 Factors

Each factor's `x_f` is its activation in [0, 1]; the sign lives in the weight `w_f` (§15.1: draining factors carry negative weights, `casualty_rate` −6, `recovery` +3), so `w_f × x_f` is the signed rate. Exceptions: `high_ground` is bipolar in [−1, 1] and `flanked` reaches 2 when surrounded (SIM-MOR-019). `sat(x)` clamps to [0, 1]; a reference that is not positive makes its factor 0. (The formulas below were written with negated activations before T2-041, which double-signed them against the weights.)

| Rule | Factor | `x_f` |
|---|---|---|
| SIM-MOR-010 | `casualty_rate` | `sat(deaths_last_5s / (count × morale.casualty_rate_ref))`, ref 0.05 (5 % in 5 s = full drain). |
| SIM-MOR-011 | `casualty_total` | `sat((initial − count) / initial / morale.casualty_total_ref)`, ref 0.5 (a level, applied per tick like every other factor; soldiers that fled count as lost). |
| SIM-MOR-012 | `fatigue` | `sat((F_mean − morale.fatigue_start) / (1 − morale.fatigue_start))`, start 0.5. |
| SIM-MOR-013 | `general_aura` | `+1` if the regiment anchor is within the general's aura radius (the Stage 9 gate's `in_aura` flag, SIM-GEN-002), else 0; 0 until T2-043. |
| SIM-MOR-014 | `general_dead` | One-time shock: `M −= morale.general_death_shock` (default 20) to all regiments of the side (armies arrive in Phase 4), queued at Stage 15 and applied at the next tick's Stage 14; `−morale.general_death_shock × 0.5` for regiments already Shaken or worse at that moment. |
| SIM-MOR-015 | `allies_near` | `+sat(n_allied_steady_within_R / morale.allies_ref)` with `R = morale.ally_radius` (40 m), ref 3. |
| SIM-MOR-016 | `allies_routing` | `sat(n_allied_routing_within_R / morale.routing_ref)`, ref 2. Includes Shattered regiments still leaving (with soldiers on the field). Both ally counts use the anchor grid and the states as they stood at the start of Stage 14. |
| SIM-MOR-017 | `high_ground` | `+sat((h_anchor − h_nearest_enemy_anchor) / combat.height_ref)`; negative if lower (clamped to [−1, 1]). The nearest enemy anchor is found by a scan of every enemy regiment with soldiers (ties to the lower id); no enemy gives 0. |
| SIM-MOR-018 | `fear` | `1` while any active `fear` status effect; else 0 (always 0 until Phase 5: `fear` effects are rejected at load, SIM-ABIL-002). |
| SIM-MOR-019 | `flanked` | `0.5` if attacked from the flank arc in the last second; `1` if from the rear (the rear supersedes the flank); `1` more if attacked through all three arcs (front, flank, rear) in the last second (surrounded), so the range is [0, 2]. "Attacked" is every melee attack, hit or miss: Stage 10 stamps the target regiment's `arc_hit[arc]` with the tick. |
| SIM-MOR-020 | `outnumbered` | `sat((enemy_soldiers_within_R / own_soldiers_within_R − 1) / morale.outnumber_ref)`, `R = morale.outnumber_radius` (30 m) around the anchor, ref 2; "own" counts every allied soldier within `R`, the regiment's own included; no enemy within `R` gives 0, no ally with any enemy gives 1. |
| SIM-MOR-021 | `integrity` | `sat((formation.integrity_morale_threshold − I) / formation.integrity_morale_threshold)`. |
| SIM-MOR-022 | `engaged_duration` | `sat(ticks_engaged / morale.engage_fatigue_ticks)`, default 2,400 (2 min). |
| SIM-MOR-023 | `winning` | `+sat((enemy_deaths_5s − own_deaths_5s) / (count × morale.casualty_rate_ref))`, clamped at 0 below (losing is covered by casualty_rate); `enemy_deaths_5s` sums the casualty rings of the enemy regiments whose anchor lies within `morale.safe_radius` (*chosen*). |
| SIM-MOR-024 | `recovery` | `+1` when not engaged, no enemy within `morale.safe_radius` (60 m), and not Routing. |
| SIM-MOR-025 | `disengage` | One-time `−morale.disengage_penalty` (5) when an engaged regiment is ordered away: a `Move` or `AttackMove`, or an `AttackRegiment` at another target, queued at Stage 0 and applied at Stage 14 of the same tick. |
| SIM-MOR-026 | `charged` | One-time `−morale.charged_penalty` (8) when receiving a charge from the flank or rear; half of it from the front. The arc is the charging regiment's anchor seen from the charged anchor's facing (SIM-CMBT-014), queued on the `Charge` tick (Stage 9) and applied at Stage 14 of the same tick. |
| SIM-MOR-027 | `ability` | Status effects add or subtract per-second morale via `morale_per_s` effects: the regiment's summed `morale_per_s` (SIM-ABIL-005) × dt is added to the delta each tick, an additive term outside the fourteen weighted factors (T2-050; `persia:war_cry` drains 2 per second for 15 s). |

### 7.3 Routing, rally, shatter

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOR-030 | On entering Routing (SIM-MOR-003, the tick morale reaches `t_routing`) the regiment drops its order (`Idle` at `run`, target and path cleared), sets all soldiers to `Routing` and clears their melee targets, `rout_count += 1`, and follows the escape flow field (SIM-FLOW-002); its anchor follows its soldiers' centroid every tick while it routs. Commands are rejected (SIM-CMD-004). Routing soldiers do not attack (`melee_gate`) and use the Routing multipliers (SIM-MOR-004). | REQ-MOR-004 |
| SIM-MOR-031 | Rally: a Routing regiment rallies when `M ≥ t_routing + morale.rally_margin` (default 15, i.e. 30) and no enemy anchor lies within `morale.rally_safe_radius` (default 50 m) of its centroid; this is the only way out of Routing (never the SIM-MOR-003 hysteresis). On rally it enters Shaken, halts (`Idle` at `walk`), faces the nearest enemy anchor if any, reforms at its centroid (`needs_reform`) and its soldiers return to their slots (SIM-CORE-011). `rout_count` counts routs, not rallies (SIM-MOR-030). | REQ-MOR-004 |
| SIM-MOR-032 | Shatter: a regiment that would rout becomes Shattered instead when this would be its `morale.max_routs`-th rout (default 2: the second rout shatters), or when `count < initial × morale.shatter_strength` (default 0.25), or when its morale is 0 after that tick's factors. It flees exactly like a Routing regiment but never rallies; its soldiers count as fled when they leave, and the regiment entity stays (empty) for the battle result. | REQ-MOR-005 |
| SIM-MOR-033 | Contagion: SIM-MOR-016 implements spreading; additionally, on the tick a regiment routs (or shatters), allies with soldiers that are not themselves routing within `morale.rout_shock_radius` (30 m) of its anchor are queued `−morale.rout_shock` (5), applied at the next tick's Stage 14. | REQ-MOR-006 |
| SIM-MOR-034 | Pursuit: soldiers of non-routing regiments within reach of routing soldiers attack them with `min(hit probability × combat.pursuit_hit_mult, combat.max_hit)` (default 1.5, applied after the SIM-CMBT-011 clamp; Shattered targets too). Cavalry chasing routers move at `run` whatever their order's speed mode. | REQ-SIM-034 |

## 8. Fatigue

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FAT-001 | Soldier fatigue `F ∈ [0, 1]`, starts at `BattleSetup.roster[i].fatigue` (campaign may pass tired armies), default 0. | REQ-FAT-001 |
| SIM-FAT-002 | Per tick `F ← clamp(F + rate × unit.fatigue_rate_mult × zone.fatigue_mult × weather.fatigue_mult × dt_s, 0, 1)` with `rate` by activity from `fatigue.rate_<activity>` per second: idle `−0.010` (recovery), walk `0.004`, march `0.002`, run `0.020`, fighting `0.015`, routing `0.020`. Armour adds `fatigue.armour_rate × unit.armour` to all positive rates. Activity (T2-040): a dead soldier accumulates nothing; `Routing` or `Withdrawing` soldiers pay `routing`, `Fighting` soldiers `fighting`; every other soldier pays the order's speed mode (`walk`, `run`, `march`) while its regiment's anchor is following a path this tick (`movement::anchor_moves`: a moving order with a served, unfinished path and no engaged attacker) and recovers at `idle` otherwise. The regiment, not the soldier, decides "moving" because soldiers standing in formation churn at up to 1 m/s from separation and collision (measured 2026-09-05), so neither a soldier's velocity nor its displacement separates standing from walking. `weather.fatigue_mult` is `1` until the weather rules of Phase 4. | REQ-FAT-001 |
| SIM-FAT-003 | States by `fatigue.thresholds`: Fresh `F < 0.25`, Active `< 0.5`, Tired `< 0.75`, Exhausted otherwise. | REQ-FAT-002 |
| SIM-FAT-004 | Multipliers are continuous functions of `F`, not steps: `fatigue_speed_mult = 1 − fatigue.speed_loss × F` (0.3); `fatigue_attack_mult = 1 − fatigue.attack_loss × F` (0.3); `fatigue_defence_mult = 1 − fatigue.defence_loss × F` (0.2); `fatigue_interval_mult = 1 + fatigue.interval_gain × F` (0.4). States are for UI and morale only. | REQ-FAT-003 |
| SIM-FAT-005 | Regiment fatigue `F_mean` is the mean over living soldiers, recomputed every 10 ticks (on ticks divisible by 10, ascending regiment id, into the hashed `RegimentFatigue` component; an empty regiment reads 0; it starts at the roster fatigue). It feeds the anchor speed (SIM-MOVE-011), the `fatigue` morale factor (SIM-MOR-012) and the UI (SIM-FAT-003 state). | REQ-FAT-004 |

## 9. Generals and auras

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-GEN-001 | Each army's general is a soldier of category `general` (validation rejects any other category) inside a bodyguard regiment given in `BattleSetup`: `sides[].general.bodyguard` names one of the side's regiments by its setup id (the side's first regiment when absent), and the general spawns as that regiment's extra last soldier (`count + 1`, so the regiment's `initial` strength counts it) with its own unit type, `hp × general.hp_mult` (default 3) and its own `attack/defence`, tagged `GeneralTag { rank }`; `SideState.general` and `general_regiment` remember it (T2-043, plan decision 4). The cap counts one general per side. | REQ-CMBT-020 |
| SIM-GEN-002 | Aura: allied regiments whose anchor is within `general.aura_radius + general.aura_per_rank × (rank − 1)` (defaults 60 m and 5 m: a rank 1 general has exactly 60 m) of a living general whose bodyguard is not Routing or Shattered receive the `general_aura` morale factor and melee `attack × (1 + general.aura_attack)` (default 0.05). The Stage 9 gate computes the flag per regiment each tick (`MeleeGateRes.in_aura`) from the general's position that tick; Stage 10 and Stage 14 read it (T2-043, plan decision 15). | REQ-CMBT-021 |
| SIM-GEN-003 | On general death (Stage 15): `SideState.general_dead` is set, every regiment of the side is queued the SIM-MOR-014 shock for the next tick, `GeneralDied { side, soldier }` is emitted, the aura ends with the flag, `BattleResult.general_fate = Dead`. If the bodyguard regiment routs with the general alive, the general routs with it (aura suspended while Routing or Shattered). | REQ-CMBT-022 |
| SIM-GEN-004 | Fate at battle end (`BattleWorld::general_fate(side, lost)`, called by `BattleWorld::result` with `lost` = another side won, T2-071): `Dead` if the general died; `Captured` if alive on a losing side and the bodyguard is Shattered; `Wounded` if its hp is below `general.wounded_hp` (0.3) of `unit.hp × hp_mult`; else `Alive` (a general that fled the field alive is `Alive`, or `Captured` under the losing-side rule). | REQ-CMBT-023 |
| SIM-GEN-005 | The general may be ordered like any regiment (it is a soldier of its bodyguard); a bodyguard regiment engaging in melee applies the general's aura to itself, which holds by construction since the general stands inside its own anchor's radius. | — |

## 10. Abilities and status effects

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-ABIL-001 | An ability is data (`content/abilities/*.json5`, `ability.schema.json`, T2-050): `id, name_key, targeting` (`self` \| `regiment_ally` \| `regiment_enemy` \| `point` \| `area`), `radius` (metres, for `point` and `area`), `range` (metres from the user's anchor to the target anchor or point; 0 = unlimited), `cooldown_ticks, duration_ticks, energy_cost, effects: [Effect], stacking, max_stacks, requires_not_engaged, requires_not_moving`, plus the render-only `description_key` and `icon`. | REQ-ABIL-001 |
| SIM-ABIL-002 | Effect kinds and fields (`type` tags the object): `buff` / `debuff { stat, mult (default 1), add (default 0) }` where `stat ∈ {attack, defence, armour, damage, speed, attack_interval, morale_per_s, fatigue_rate, los_radius, accuracy}`; `damage { amount, armour_penetration, per_tick }`; `heal { amount, per_tick }`; `summon { unit_type, count, formation }`; `fear` (SIM-MOR-018); `area { effects, radius, duration_ticks }`; `teleport { max_distance }`. Antiquity content uses only buff and debuff; the others exist in the engine from Phase 5. Until then every kind parses and validates against the schema, and an ability using another kind is rejected at load with a diagnostic naming it (`effects[i]: effect kind "summon" is not executable before Phase 5`); a buff or debuff also needs `duration_ticks ≥ 1`. | REQ-ABIL-002 |
| SIM-ABIL-003 | `UseAbility { regiment, ability, target }` is validated at Stage 0 in this order: the ability exists (`UnknownContent`), the regiment owns a slot for it (its unit's `abilities`, then its general unit's while this is the side's bodyguard regiment and the general is alive and on the field: `NotOwned`), the slot's cooldown is 0 (`OnCooldown`), `energy ≥ energy_cost` (`NoEnergy`), the target fits the targeting kind (`self` and `area` take `SelfTarget`; `regiment_ally` / `regiment_enemy` a `Regiment` on the right side with soldiers, an enemy also visible to the user's side per SIM-VIS-004; `point` a `Point` on the map: `BadTarget`), it lies within `range` (`OutOfRange`), and the conditions hold (`requires_not_engaged` against `Combat.engaged`: `Engaged`; `requires_not_moving` against the anchor following a path, SIM-MOVE-011: `Moving`); the failures are `RejectReason::Ability { regiment, ability, why }`. Battle and Pursuit only (`WrongPhase`). On success the slot's cooldown is set, the energy deducted and a status applied to every target regiment (SIM-ABIL-004): the user itself, the named regiment, or (`point`, `area`) every regiment with soldiers whose anchor lies within `radius` of the point or of the user's anchor, in ascending id. A same-side target takes every effect of the ability; an enemy target takes only its debuffs (the status is marked hostile). `AbilityUsed { regiment, ability, targets }` is emitted. | REQ-ABIL-001 |
| SIM-ABIL-004 | A regiment's status effects carry `source` (the ability), `remaining` ticks, `stacks` and `hostile`, one entry per source ability in application order. A second application of the same ability combines per its `stacking`: `refresh` resets the duration; `stack` bumps the count up to `max_stacks` and resets the duration; `highest` keeps the longer remaining duration and the higher count. Stage 12 `ability_tick` counts every status down and removes the ones that reach zero (`StatusExpired`), refreshes the cached multipliers, counts the slot cooldowns down and regenerates energy, in ascending regiment id. | REQ-ABIL-003 |
| SIM-ABIL-005 | The ten stat multipliers of a regiment's active statuses (`StatMults`, cached per regiment and refreshed whenever the list changes and on restore): multiplicative parts multiply across every status and effect (once per status, whatever its stack count); additive parts sum, scaled by the stack count, and join the multiplier as `+ add`, except `armour` (`unit.armour × armour_mult + armour_add`, in points) and `morale_per_s` (purely additive, morale per second, SIM-MOR-027). A hostile status contributes its debuff effects only. Where they act: `attack` on `A` and `defence` on `D` (SIM-CMBT-011, each regiment's own); `damage` on the melee damage and `armour` on the defender's armour in SIM-CMBT-013, SIM-PROJ-006 and the statistical shot (SIM-PROJ-008); `attack_interval` on the melee cooldown (SIM-CMBT-010) and the ranged reload (SIM-PROJ-003); `speed` on the soldier and anchor speeds (SIM-MOVE-011/020); `fatigue_rate` on the accumulation rate (SIM-FAT-002); `accuracy` on the shooter's `ranged.accuracy` (SIM-PROJ-004); `los_radius` on SIM-VIS-001 (T2-060). | — |
| SIM-ABIL-006 | Energy: `regiment.energy ∈ [0, unit.energy_max]` starts full and regenerates `unit.energy_regen` per second at Stage 12 (both unit fields default 0). Antiquity units have `energy_max = 0` and abilities with `energy_cost = 0`; the fields exist so Phase 5 needs no schema change. Hashed and snapshotted. | REQ-ABIL-004 |
| SIM-ABIL-007 | Ability effects are applied in command order at Stage 0 (`(player, seq)`, SIM-CMD-001; the targets of one use in ascending regiment id) and ticked in ascending regiment id at Stage 12. | REQ-SIM-007 |

## 11. Visibility

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-VIS-001 | Each regiment has `los_radius = unit.los_radius × zone.los_mult(anchor) × weather.los_mult × (1 + visibility.height_bonus × sat((h_anchor − h_mean_map) / combat.height_ref)) × status_los_mult`, default height bonus 0.5, `sat` clamped to `[−1, 1]`; `h_mean_map` is the arithmetic mean of the heightmap samples (`LoadedMap.mean_height`, a fixed-order sum at load); `weather.los_mult` is `1` until the weather rules of Phase 4; `status_los_mult` is the regiment's `los_radius` status multiplier (SIM-ABIL-005). (T2-060.) | REQ-SIM-050 |
| SIM-VIS-002 | A point is visible to a regiment if within `los_radius` and the segment from the anchor (at eye height `visibility.eye_height`, 1.7 m) to the point (at 1.7 m) clears the heightmap: the segment is split into `floor(d / visibility.los_sample) + 1` equal intervals (`los_sample` 4 m) and the ground is read at every interior point; it is blocked where the ground rises strictly above the straight line between the two eyes. Walls taller than the line at the crossing block it from Phase 5. | REQ-SIM-050 |
| SIM-VIS-003 | An enemy regiment is visible to a side if its anchor or any of up to four sampled soldiers (`Regiment.soldiers[k × n / 4]`, `k ∈ 0..4`, deduplicated) is visible to any of the side's regiments with soldiers (observers ascending by id, the first sighting settles it); forests: a regiment whose anchor is in a `conceal` zone is visible exactly when an observer's anchor is within `visibility.conceal_radius` (default 25 m) of its anchor, regardless of LOS. A side always sees its own regiments. Visibility is keyed by side, not faction: two sides may share a faction id, and alliances arrive with the campaign. | REQ-SIM-051, REQ-SIM-052 |
| SIM-VIS-004 | Visibility is recomputed per side, staggered: side `s` refreshes at Stage 8 of every tick with `tick mod visibility.period_ticks = s mod period_ticks` (default 10), and `BattleWorld::new` computes every side's mask once so tick 1 is not blind. The mask is state (hashed and snapshotted, SIM-DET-004/005): a snapshot between refreshes cannot rederive it. Stage 0 reads a mask up to `period_ticks` old. Hidden regiments cannot be named by `AttackRegiment`, `FireMode::Target` or `UseAbility` (`RejectReason::NotVisible`), `fire_at_will` ignores them (the current target included), and `AttackMove` does not acquire them (SIM-CMBT-005); an `AttackRegiment` already given keeps chasing its target. Melee targeting (SIM-CMBT-002, at contact range, inside `conceal_radius`) and the morale factors and rally checks (§7) read every regiment. The AI queries only visible regiments (SIM-AI-003). | REQ-SIM-051, REQ-SIM-053 |
| SIM-VIS-005 | Once a side has seen an enemy regiment it remembers the regiment's anchor, facing and soldier count with the tick of the sighting; the memory restarts at every sighting and expires `visibility.memory_ticks` (default 400) after the last one. UI ghosting only: the sim never reads it; it is stored in snapshots (so a restored battle keeps its ghosts) but not hashed. | — |
| SIM-VIS-006 | During deployment, fog of war applies (blind deployment) unless `BattleSetup.reveal_deployment` is set, in which case every side's mask is full while the phase is Deployment (resolves OQ-6 with a data switch; default false). | REQ-SIM-030 |

## 12. Battle flow

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FLOW-010 | Phases: `Deployment → Battle → Pursuit → Ended`, moved at Stage 16 (`battle_flow`, T2-070) with a `PhaseChanged` event each. Each phase runs a fixed set of stages (`Stage::runs_in`): Deployment runs Stage 0 (commands), 1 (the AI places its sides, T2-080), 6 (grids), 8 (fog), 16 and 17; Ended runs Stage 0 and 17 only; Battle and Pursuit run everything. Stage 0 accepts per phase: Deployment `Deploy`, `ConfirmDeployment`, `SetFormation`, `SetFacing`, `Pause`, `SetSpeed`, `Surrender`, `TransferControl`; Ended `Pause` and `SetSpeed`; Battle and Pursuit everything but `Deploy` and `ConfirmDeployment` (`WrongPhase` otherwise). | REQ-SIM-030..035 |
| SIM-FLOW-011 | Deployment: a regiment with a `position` in the setup spawns deployed there (PRD OQ-9: the pre-deploy override, checked against the map only); the others are auto-placed at spawn in a battle line at the vertex mean of their side's deployment polygon, facing the mean of the other sides' zone centres (`arrange_group` on a built-in battle-line template with `formation.group_gap`). `Deploy { regiment, position, facing, template }` moves a regiment during the phase; the anchor must lie inside the side's polygon (`OutsideDeploymentZone`). A side whose every regiment is pre-placed starts confirmed; a side owned by the engine AI (`PlayerId(255)`) deploys and confirms through its own Stage 1 commands (SIM-AI-020, T2-081); every side confirms with `ConfirmDeployment` (every side its player owns, `DeploymentConfirmed`). The phase ends, and `BattleFlow.battle_start` is stamped, when every side is confirmed or `battle_flow.deploy_timeout_ticks` (default 0 = none) has passed; when every side starts confirmed the battle starts in `Battle` at tick 0 with no event. Regiments not moved by `Deploy` keep their auto-placement. | REQ-SIM-030 |
| SIM-FLOW-012 | Battle: the timer runs from `battle_start` to `BattleSetup.time_limit_ticks` (default 48,000 = 40 min). | REQ-SIM-032 |
| SIM-FLOW-013 | Evaluated every Battle tick after the reinforcements: a side is *defeated* when it surrendered, or has no regiment with soldiers in state Steady/Unsettled/Shaken/Broken whose order is not `Withdraw`, and no reinforcement group pending (`SideState.defeated`, hashed). When exactly one side is not defeated the phase becomes Pursuit with that side as the winner (`BattleFlow.winner`, `pursuit_start`); when every side is defeated at once the phase becomes Ended with no winner; when the timer expires the phase becomes Ended with the setup's `victory.timeout_winner` side if named, else per `battle_flow.timeout_winner`: `most_soldiers` the side with the most living soldiers on the field (a tie is a draw), `defender` a draw until the campaign names the defender (plan decision 18). A battle with fewer than two sides (tools and test fixtures) has no verdict: nothing is defeated and the timer does not run. A side whose every regiment routs is defeated the tick it happens; a rally during the pursuit does not revive it (it ends the pursuit instead, SIM-FLOW-015, and the rallied regiment counts as a survivor). | REQ-SIM-032 |
| SIM-FLOW-014 | Withdraw: the regiment's order becomes `Withdraw` at `march` with no target and no path, every soldier enters `Withdrawing` and drops its melee target (`Withdrawing` event); the soldiers follow the escape flow field (SIM-FLOW-002) and the anchor follows their centroid; they do not attack (`may_fight` excludes the order) but may be attacked normally. They do not count as standing for SIM-FLOW-013, so a side whose regiments all withdraw is defeated at once. A soldier that reaches the escape edge leaves as a survivor: `Combat.withdrawn` (hashed), `SoldierWithdrew`; those still on the field at the end are survivors too. `Withdraw` on a Routing or Shattered regiment is accepted and ignored (SIM-CMD-004). | REQ-SIM-033 |
| SIM-FLOW-015 | Pursuit: lasts `battle_flow.pursuit_ticks` (default 2,400) from `pursuit_start` or until no Routing soldier remains on the field. Pursuers act per SIM-MOR-034. At the end every Routing soldier still on the field escapes: counted as fled (`SoldierFled`, removed like the dead) before the phase becomes Ended. | REQ-SIM-034 |
| SIM-FLOW-016 | Reinforcements: `BattleSetup.side[i].reinforcements: [{ arrival_tick, edge, regiments }]`; `arrival_tick` counts from `battle_start`, `edge` names a map edge (`"north"` …) that the map's `reinforcement_edges` lists for the side's deployment zone (`SetupError::UnknownReinforcementEdge` otherwise). Groups spawn at Stage 16 in side then group order, each regiment in its unit's Column template (or its default) side by side along the edge `formation.group_gap` apart, centred on the edge midpoint, facing into the map, order Idle; the anchors are clamped to the map and Stage 5 pushes soldiers out of impassable cells. `SideState.reinforcements_spawned` counts the groups taken (hashed); a group that would pass the cap is dropped with `ReinforcementsDropped` (SIM-CORE-006), otherwise `ReinforcementsArrived`. The new regiments join the grids at the next Stage 6 and the fog masks grow. | REQ-SIM-036 |
| SIM-FLOW-017 | `Surrender` by a player marks every side it owns surrendered (`Surrendered`); the side is defeated at the next Stage 16. | — |
| SIM-FLOW-018 | `BattleWorld::result()` (T2-071, plan decision 23) is available at any tick: `BattleResult = { winner, duration_ticks, sides: [{ regiments: [{ id, initial, survivors, fled, killed, experience_gain, ammo_left, arrived }], general_fate, loot }], summary: { total_killed, total_fled } }`. `winner` is `None` unless the phase is Ended (an aborted battle has no winner); `duration_ticks` counts from the start of the Battle phase. Per regiment in ascending id: `id` the setup id, `initial` the soldiers at spawn (the general included in its bodyguard's), `survivors` the soldiers on the field plus the withdrawn (SIM-FLOW-014), `fled` per SIM-FLOW-002/015, `killed = initial − survivors − fled`, so `initial = survivors + fled + killed` always; `experience_gain = floor(battle_flow.exp_per_kill × kills_by_regiment + battle_flow.exp_survive × [the regiment has survivors])` (0.01, 1) in points, which the campaign maps to the 0..9 level through `experience_tiers` (Phase 4; plan decision 19); `ammo_left` the sum of the living soldiers' ammo (0 for units without `ranged`); `arrived` false for a reinforcement group that never entered, listed with its full count as survivors. Per side: `general_fate` per SIM-GEN-004 with `lost` = another side won; `loot = floor(battle_flow.loot_per_enemy_killed × enemy soldiers killed)` for the winner, 0 otherwise (fled and withdrawn enemies are not killed). Fled soldiers return to the campaign as survivors if their side won, or `battle_flow.fled_return_fraction` (0.5) of them if it lost (Phase 4). `il_cli autoresolve <scenario.json5> [--ai all|none|<players>]` runs a scenario headless to Ended or a tick cap and prints the result as JSON; by default every player's sides go to the engine AI before tick 1 (a `TransferControl` issued as the engine) and the scripted commands are dropped, `--ai none` keeps the scripted run, a player list hands over those players only (T2-082, plan decision 17). | REQ-SIM-061, REQ-SIM-063 |
| SIM-FLOW-019 | `BattleSetup` = `{ map_id, seed, weather, time_of_day, time_limit_ticks, reveal_deployment, sides: [{ faction, player (human/ai id), deployment_zone, general: { unit_type, rank, name_key, bodyguard? }, regiments: [{ id, unit_type, count, experience, fatigue, formation }], reinforcements, ai_profile? }], victory: { timeout_winner } }`. `sides[].ai_profile` overrides the faction's profile when the engine decides for the side (T2-080, plan decision 12; `SetupError::UnknownAiProfile`). `regiments[].position` / `facing_deg` are the optional pre-deploy override (SIM-FLOW-011, OQ-9); `reinforcements[].edge` is a map edge name. Validation: cap (one general per side counts), map exists, zones exist, every reinforcement edge is listed by the map for the side's zone, unit types exist, each side has a general of category `general` whose `bodyguard` (default: the side's first regiment) is one of its regiments (SIM-GEN-001, T2-043). | REQ-SIM-060 |

## 13. Battle AI

### 13.1 Framework

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-AI-001 | Utility AI (T2-080): a decision is made per *channel* (regiment: movement, formation, fire, one per ability slot; army: stance). Each candidate action of the channel that the engine finds applicable (an enemy exists to engage, the unit owns the layout, the ability slot is ready) is scored `score = base × Π_c curve_c(x_c)` over its considerations, where `x_c = sat(raw_c / scale_c)` normalises the input (`scale` is data, plan decision 21) and `curve_c` is one of `linear(m, b) = sat(m·x + b)`, `quadratic(k) = sat(k·x²)`, `logistic(k, mid) = 0.5 + 0.5·t / (1 + |t|)` with `t = k·(x − mid)` (an algebraic sigmoid: no exponential, bit-exact on every platform; a negative `k` decreases) and `step(threshold)` (`1` when `x ≥ threshold`), every curve output clamped to `[0, 1]`. Optional `noise` multiplies the score by `1 + noise·(2u − 1)` with `u` drawn from the `ai_regiment` / `ai_army` stream in list order, only when `noise > 0`. The highest score of at least the action's `threshold` wins the channel; ties go to the earlier action in the list; a channel whose best action falls short emits nothing. Actions, curves and inputs are data (`content/ai/actions/*.json5`, §15.4); the input names and the action `kind`s are closed vocabularies checked at load. | REQ-AI-001 |
| SIM-AI-002 | Cadence: every side owned by `PlayerId(255)` decides at Stage 1 (`ai_decide`, which also runs during Deployment): the army AI when `tick mod ai_profile.army_period_ticks = side mod army_period_ticks` (default 40), each regiment when `tick mod ai_profile.regiment_period_ticks = regiment_id mod regiment_period_ticks` (default 20). Decisions become Commands for `tick + 1` tagged `PlayerId(255)`, `seq` counting from 0 in emission order (sides ascending, army before regiments, regiments ascending, channels movement, formation, fire, abilities); they wait in the AI outbox, which is state (hashed and snapshotted, SIM-DET-004/005), and join the next tick's inbox before Stage 0 (SIM-CMD-005). `BattleWorld::set_ai_enabled(false)` silences Stage 1 for a replay that feeds the logged commands instead. | REQ-AI-006 |
| SIM-AI-003 | The AI reads only what a player could see: the regiments its side's visibility mask shows (SIM-VIS-004), its own state and the terrain, never the sighting memory (SIM-VIS-005); with no enemy visible it aims at the mean of the other sides' deployment zone centres (plan decision 9). It reads only state and the derived data a restore rebuilds (the anchor and soldier grids), never the per-tick targeting gates, so a restored battle decides exactly as the uninterrupted one. | REQ-SIM-053 |

### 13.2 Army level

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-AI-010 | Army AI (T2-082): every `ai_profile.army_period_ticks` (staggered by side) a side owned by the engine scores the stance channel of its army action set (`attack`, `defend`, `hold`, `retreat`; §15.4) and rebuilds its plan `{ stance, stance_score, decided_at, target, line_anchor, line_facing, line_width, formed, assignments, charging }` (`ArmyPlan`, hashed and snapshotted). Hysteresis: a new stance must beat the current stance's fresh score by `ai_profile.stance_margin`; `retreat` needs no margin. Army inputs (raw): `army_strength_ratio` own over own plus visible enemy by `count × unit.cost` over standing regiments (1 with nothing visible); `army_morale_mean` and `army_fatigue_mean` over standing regiments (morale over 100); `terrain_advantage` `0.5 + 0.5 · sat((h_line − h_target) / combat.height_ref)` with `sat` to `[−1, 1]`; `time_remaining` the fraction of `time_limit_ticks` left since `battle_start`; `aggression` the profile's; `casualty_fraction` `1 − alive / initial` over the side; `enemy_visible` 0/1; `constant` 1. `target` is the count-weighted centroid of the visible enemies, else the mean of the other sides' zone centres (SIM-AI-003). Roles: the general's bodyguard (`Bodyguard`, SIM-AI-022, while another regiment stands); cavalry `Flank` on attack, `Counter` on defend and hold, `Screen` on retreat; ranged and skirmisher units `Skirmish` (on defend they stand in the line); the smallest infantry by cost up to `reserve_fraction` of the infantry cost `Reserve`, the rest `Line`. Assignments ascend by regiment id. | REQ-AI-003 |
| SIM-AI-011 | On `attack` (as tuned in T2-082 against the §15.3 rows 9 and 10) the line forms where its regiments stand (the line regiments' centroid; `arrange_group` on a battle-line template with `formation.group_gap`, its ranks chosen to match the visible enemy's frontage so the regiments meet theirs, facing the target) and steps `ai_profile.advance_step` per period toward the *strike point*, the enemy's nearer wing (`0.6 ×` the enemy's lateral half extent from its centroid on the side of the own centroid), while its regiments lag their slots by less than twice `line_tolerance`; it holds at `approach_distance` from the strike point while its missile units stand off with ammo (the archers' stand-off is out of the enemy's reach) or while the line's mean fatigue is above the fresh threshold, then closes to 10 m; regiments march to their slots (`SetSpeedMode` never; `Move` at `March`, walk pace at half the fatigue) and `engage_nearest` takes over as the lines close. Once any line regiment is within `charge_trigger_dist` of a visible enemy the plan is `charging` (sticky for the stance): the line, the reserves and the flank groups run, and the flank groups charge the rear-most visible regiment (the largest forward offset from the target; `AttackRegiment`). Missile units with ammo stand off at `skirmish_range_frac × unit.ranged.range` from the nearest visible enemy on the line toward it, unless an enemy missile unit reaches that point (its range plus 10 m) and the charge is not on; spent, under fire or (for bows) once the charge is on they stand behind the line, bows half a `reserve_offset` back shooting over it, javelins a full one. Cavalry flank groups head at `run` for the enemy's nearer flank, `flank_offset` beyond the visible enemies' lateral extent, pulled back along their own approach to passable ground. `approach_distance` doubles as the campaign's deployment distance in Phase 4 (plan decision 22). | REQ-AI-003 |
| SIM-AI-012 | On `defend` the line forms on the highest heightmap sample within `ai_profile.defend_search_radius` of the line centroid, found in raster order on entering the stance and kept while it lasts; ranged units stand in the line and fire at will; cavalry holds `reserve_offset` behind each line end (`Counter`) and, once a visible enemy comes within `counter_charge_dist` of that end, charges it (`Committed`, kept while the target is visible and alive). `hold` is `defend` where the army stands. | REQ-AI-003 |
| SIM-AI-013 | On `retreat` every standing non-cavalry regiment gets `Withdraw` (once); the cavalry screens at the infantry centroid plus `ai_profile.screen_offset` toward the enemy, a slot fixed when the retreat began, and withdraws once the nearest own infantry is more than `screen_gap` away. | REQ-AI-003 |
| SIM-AI-014 | Reserves hold `reserve_offset` behind the line centre, spread by their widths plus `formation.group_gap`; every period a reserve is committed (`Committed { target }`, an `AttackRegiment` at walk) to the nearest visible enemy of the line regiment with the lowest morale below `ai_profile.commit_morale`, and one more to each visible enemy that threatens a flank (its lateral offset beyond the line's half width by at most `flank_offset`); a commitment stands while its target is visible and alive. | REQ-AI-003 |

### 13.3 Regiment level

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-AI-020 | Deployment (T2-081): at its first Stage 1 in the Deployment phase an unconfirmed side owned by the engine places its regiments (all but the general's bodyguard) with `arrange_group` in a battle line at its zone's vertex mean facing the mean of the other sides' zone centres, skirmishers and ranged units `formation.skirmish_offset` ahead and cavalry on the flanks, the bodyguard `ai_profile.reserve_offset` behind the centre; a placement outside the deployment polygon keeps the regiment's auto-placement (SIM-FLOW-011). One `Deploy` per regiment and a `ConfirmDeployment` are queued for the next tick (SIM-AI-002); a side whose regiments are all pre-placed is confirmed already and deploys nothing. | REQ-AI-003 |
| SIM-AI-021 | Regiment decisions (T2-081), every `ai_profile.regiment_period_ticks` staggered by regiment id, for regiments with soldiers that are not Routing or Shattered, over the side's regiment action set (§15.4) channel by channel: **movement** `engage_nearest` (applicable with a visible enemy; `AttackRegiment` on the nearest, unless the order already chases it), `hold_position` (a `Move` at walk to the plan slot facing the line when the anchor is more than `line_tolerance` away and no such move is current, a `Halt` when arrived but still moving, a `SetFacing` when arrived, idle and more than `formation.reform_angle` off; nothing without a slot or while engaged), `fall_back` (applicable with a plan; a `Move` to `reserve_offset` behind the line at the regiment's lateral position), `follow_centroid` (the bodyguard only, SIM-AI-022); **formation** `switch_formation { layout }` (applicable when the unit lists a template of that layout, it is not the current one and no corridor morph is active; `SetFormation`); **fire** `fire_at_will` / `hold_fire` (units with `ranged`; `FireMode` when the mode differs, a player's `Target` is left alone); **abilities** one channel per slot (`abilities::slots`), applicable when the slot is off cooldown, the energy suffices, `requires_not_engaged` / `requires_not_moving` hold and a target exists (self and area: the regiment; `regiment_enemy`: the nearest visible enemy within `range`; `regiment_ally`: the nearest other own regiment within `range`; `point`: the own anchor), the candidates being the `use_ability` actions naming the slot's ability (`UseAbility`). A regiment whose plan role is `Flank` with a charge target or `Committed` skips the movement channel and attacks that target (flank groups at `Run`). The AI never queues a command it can predict the sim would reject. Raw inputs (before a consideration's `scale`; distances in metres, `100000` when nobody qualifies): `constant` 1; `distance_to_nearest_enemy`; `strength_ratio` `own / (own + nearest)` by `count × unit.cost` (1 with none); `enemy_share` `1 − strength_ratio`; `own_morale` `morale / 100`; `own_fatigue` the regiment fatigue mean; `engaged` 0/1; `engaged_frontal` engaged and the nearest enemy inside the unit's `frontal_arc_deg`; `is_flank_exposed` the distance to the nearest other own regiment; `enemy_flank_open` 1 when the nearest enemy sees us outside its frontal arc; `cavalry_approaching` the distance to the nearest visible enemy cavalry whose order moves; `infantry_threat` the distance to the nearest visible non-cavalry enemy; `enemy_ranged_in_range` 1 when a visible enemy with `ranged` has our anchor within its range plus our half width; `enemy_in_own_range` 1 when we have `ranged` and a visible enemy anchor lies within our range; `friendly_in_line_of_fire` 1 when we are direct-fire and an own regiment's anchor lies within `combat.friendly_block_dist` plus its half width of the segment to the nearest enemy; `outnumbered` enemy soldiers within `morale.outnumber_radius` over enemy plus own; `slot_error` the distance to the plan slot (0 without one); `ammo` the best soldier's ammo over the unit's. The flagship curves are in `rome:regiment_default` (§15.4). | REQ-AI-003 |
| SIM-AI-022 | The general's bodyguard (T2-081): its movement channel offers `follow_centroid`, the `1/morale`-weighted centroid of the side's other regiments with soldiers, pushed to `ai_profile.reserve_offset` behind the line when a plan exists (a `Move` at walk unless within `line_tolerance`); `engage_nearest` is applicable for it only while `ai_profile.general_aggression` exceeds the enemy's share of the strength ratio (`1 − strength_ratio`). | — |

## 14. Campaign simulation

### 14.1 Turn structure

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CAMP-001 | Turn phases (SAD §6.3): `TurnStart` (events, income preview), `PlayerPhase`, `AIPhase` (factions in ascending faction id), `Resolution`, `TurnEnd`. | REQ-CAMP-001, REQ-CAMP-002 |
| SIM-CAMP-002 | Campaign Commands: `MoveArmy { army, path }`, `Recruit { settlement, unit_type }`, `Build { settlement, building }`, `Research { tech }`, `Diplomacy { target, action, terms }`, `SetTax { province, level }`, `MergeArmies`, `SplitArmy`, `DisbandRegiment`, `EndTurn`, `ApplyBattleResult { battle_id, result }`, `AutoResolve { battle_id }`. | REQ-CAMP-* |
| SIM-CAMP-003 | Turn = one season; `campaign.turns_per_year` = 4. Winter (`turn % 4 == 3`) applies `campaign.winter_attrition` (0.05 of soldiers) to armies outside friendly provinces. | REQ-CAMP-004 |

### 14.2 World and movement

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CAMP-010 | Province: `id, name_key, polygon, neighbours: [{ province, cost, kind ∈ {land, river_crossing, mountain_pass, sea} }], terrain_type, resources: [ContentId], settlement, owner, tax_level, public_order, population`. | REQ-CAMP-010 |
| SIM-CAMP-011 | Army movement: `movement_points = campaign.base_movement × min over regiments of unit.campaign_speed_mult`; a move along an edge costs `edge.cost × (1 if road else campaign.no_road_mult)`; a path is executed edge by edge until points run out; remaining path continues next turn. | REQ-CAMP-011 |
| SIM-CAMP-012 | Interception: entering a province containing a hostile army ends movement and creates a battle with attacker = mover; `BattleSetup` built by SIM-CAMP-040. Several hostile armies in the province join as reinforcements (SIM-FLOW-016) arriving at `arrival_tick = campaign.reinforce_delay_ticks` (600). | REQ-CAMP-012 |
| SIM-CAMP-013 | Battle map selection: candidate maps are those whose `campaign_terrain_tags` contain the province's `terrain_type` tag and, for an assault, the tag `settlement_tier_<n>`; field battles exclude maps with any `settlement_tier_*` tag. The map is `candidates[hash(seed, province_id, turn) % len]`, sorted by ContentId first. No candidate is a content validation error at campaign load. | REQ-SIM-060, A-5 |
| SIM-CAMP-014 | Siege (Phase 5): an army entering an enemy settlement province with a garrison enters `Besieging`; each turn the settlement loses `campaign.siege_supply` and surrenders at 0; `Assault` creates a siege battle. | REQ-CAMP-013 |

### 14.3 Economy

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CAMP-020 | Income per province per turn `= population × tax_rate[tax_level] × building_tax_mult + Σ resources.value × building_resource_mult`. Trade income `= Σ over trade partners of min(exports, partner imports) × campaign.trade_rate`, partners are factions with a trade agreement connected by a path of non-hostile provinces or sea edges. | REQ-CAMP-020, REQ-CAMP-021 |
| SIM-CAMP-021 | Expenses `= Σ regiments unit.upkeep × (1 + campaign.upkeep_growth × (regiments − campaign.free_upkeep)) + Σ buildings.maintenance`. Treasury may go negative; while negative, recruitment and building are refused and morale_base of all regiments is reduced by `campaign.debt_morale` (10) in battles. | REQ-CAMP-020 |
| SIM-CAMP-022 | Buildings: `id, name_key, cost, turns, requires: [building ids, tech ids], effects: { tax_mult, resource_mult, recruit: [unit_type], public_order, growth }`; one construction per settlement at a time; completion at TurnEnd. | REQ-CAMP-022 |
| SIM-CAMP-023 | Public order per province `= base + buildings − campaign.tax_unrest[tax_level] − garrison_deficit`; below 0 for `campaign.rebel_turns` turns spawns a rebel army of size proportional to population. | — |

### 14.4 Diplomacy

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CAMP-030 | Relation states between two factions: `war, peace, trade, alliance, vassal (Phase 5)`. Actions: `DeclareWar` (peace/trade → war; allies of the target join if `alliance.defensive`), `ProposePeace`, `ProposeTrade`, `ProposeAlliance`, `Break` (trade/alliance → peace with attitude penalty). | REQ-CAMP-030, REQ-CAMP-031 |
| SIM-CAMP-031 | Attitude `att ∈ [−100, 100]` per ordered pair updated at TurnEnd: `att ← att + Σ_k w_k × x_k` with factors: shared border (`−`), at war with common enemy (`+`), treaties (`+`), recent war (`−`, decays over `diplomacy.grudge_turns`), strength ratio (fear: `−` if the other is stronger by `diplomacy.fear_ratio`), personality bias (`diplomacy_personality.base_att`), broken treaties (`−`). Proposals are accepted if `att + offer_value × diplomacy.offer_scale ≥ diplomacy.accept_threshold[action]`. | REQ-CAMP-032 |
| SIM-CAMP-032 | Coalition (Phase 5): when a faction owns more than `diplomacy.coalition_share` (0.4) of provinces, other factions gain a `+` attitude factor toward each other and a `−` toward it. | REQ-CAMP-031 |

### 14.5 Research, recruitment, experience

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CAMP-040 | Technology: `id, name_key, category, cost_turns, requires: [tech], effects: { unlock_units, unlock_buildings, modifiers: [{ target, stat, mult|add }] }`. One research at a time; `cost_turns × campaign.research_mult(buildings)`; completion at TurnEnd. | REQ-CAMP-040 |
| SIM-CAMP-041 | Recruitment: a settlement's pool is the union of its buildings' `recruit` lists intersected with the faction's `units`; recruit cost `unit.cost` paid now, regiment appears after `unit.recruit_turns` at the settlement with `count = unit.regiment_size`, experience 0. At most `campaign.recruit_slots(buildings)` concurrent recruitments. | REQ-CAMP-041 |
| SIM-CAMP-042 | Experience persists on the campaign regiment: `experience = min(9, experience + gained / campaign.exp_per_level)`. | REQ-CAMP-042 |
| SIM-CAMP-043 | Replenishment: regiments in a friendly province with a settlement regain `campaign.replenish_rate` (0.1) × `regiment_size` per turn up to full, costing `unit.cost × replenished / regiment_size`. | REQ-CAMP-043 |
| SIM-CAMP-044 | `BattleSetup` from campaign: `map` per SIM-CAMP-013; `seed = hash(campaign_seed, turn, battle_index)`; weather from province season table; sides from the armies; deployment zones: attacker on the edge nearest its origin province's centroid direction, defender opposite; `time_limit_ticks` from `campaign.battle_time_limit`. Applying `BattleResult` (SIM-FLOW-018): survivors set regiment counts, empty regiments removed, experience added, general fate applied (Dead → army leaderless, next turn a replacement general with rank 0 if the faction has a settlement; Captured → same plus ransom event), losing army retreats to the nearest friendly province or is destroyed if none. | REQ-SIM-060..062, REQ-CMBT-023 |
| SIM-CAMP-045 | Auto-resolve (OQ-3, provisional): run the battle headless with AI on both sides at unlimited speed for at most `campaign.autoresolve_max_ticks` (12,000); if not ended, decide by remaining soldier cost ratio. This keeps one combat model; if too slow at P2 scale a statistical model replaces it behind the same `BattleResult` producer. | REQ-SIM-064 |

### 14.6 Campaign AI

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CAMP-050 | Each AI faction runs a utility decision per category per turn: `expansion` (target province scoring: value, defence, distance, owner relation), `army` (recruit to reach `ai_profile.army_strength_target` relative to neighbours; composition ratios from `ai_profile.composition`), `economy` (build order scoring by payback turns), `research` (category weights), `diplomacy` (propose trade/alliance when attitude high; declare war on weak neighbours when `aggression × strength_ratio` exceeds threshold). | REQ-AI-004 |
| SIM-CAMP-051 | AI army movement: armies move toward the highest scoring target (attack province, defend threatened province, merge) via the province graph; an AI never leaves a settlement ungarrisoned below `ai_profile.min_garrison`. | REQ-AI-004 |
| SIM-CAMP-052 | The campaign hash at TurnEnd covers: turn; per faction treasury, relations, research state; per province owner, buildings, population, order; per army position, regiments (type, count, experience); RNG states. | REQ-SIM-002 |

## 15. Tuning appendix (antiquity defaults)

These live in `game/content/rules/*.json5` and unit files. Values are starting points for Phase 2 balancing.

### 15.1 Rules files

One table per file under `game/content/rules/`; every field is required (the engine carries no numeric defaults) and each file has a schema `docs/schemas/rules-<file>.schema.json`. Values marked *chosen* were fixed in T2-010 without a rule stating them and are the first candidates for tuning.

**`movement.json5`** (§5)

| Field | Default | Rule |
|---|---|---|
| `nav_cell` | 4 | SIM-MOVE-001 |
| `hpa_cluster` / `hpa_gate_split` | 16 / 6 nav cells | SIM-MOVE-003 |
| `paths_per_tick` | 8 | SIM-MOVE-005 |
| `wheel_rate` | 45 °/s | SIM-MOVE-010 |
| `waypoint_radius` | 2 | SIM-MOVE-010 |
| `straggler_radius` / `straggler_fraction` / `straggler_slowdown` | 3 × sf / 0.25 / 0.5 | SIM-MOVE-012 |
| `slot_arrive_radius` / `slot_leave_radius` | 0.3 / 0.6 | SIM-CORE-011 |
| `sep_weight` / `sep_margin` / `sep_max_neighbours` | 1.5 / 0.2 / 8 | SIM-MOVE-022 |
| `arrive_damping` | 0.5 | SIM-MOVE-021 |
| `lookahead_ticks` | 4 | SIM-MOVE-023 |
| `soldier_turn_rate` | 360 °/s | SIM-MOVE-024 |
| `slope_penalty` / `slope_bonus` | 2.0 / 0.5 | SIM-MOVE-030 |
| `slope_min_mult` / `slope_max_mult` | 0.4 / 1.2 | SIM-MOVE-030 |
| `ford_defence_mult` | 0.7 | SIM-MOVE-032 |
| `collision_iterations` | 2 | SIM-MOVE-041 |
| `spatial_cell` / `anchor_cell` / `zone_cell` | 4 / 16 / 2 | TDD §5, §6.2 |

**`formation.json5`** (§4)

| Field | Default | Rule |
|---|---|---|
| `keep_slot_radius` | 1.5 | SIM-FORM-020 |
| `assign_search_radius` / `swap_passes` | 30 / 2 | SIM-FORM-022 |
| `reform_angle` / `turn_in_place_angle` | 10° / 120° | SIM-FORM-024 |
| `integrity_radius` / `integrity_period_ticks` | 1.0 × sf / 5 | SIM-FORM-030 |
| `integrity_morale_threshold` | 0.5 | SIM-MOR-021 |
| `morph_speed_mult` | 0.5 | SIM-FORM-032 |
| `group_gap` / `skirmish_offset` / `width_tolerance` | 6 / 20 / 0.1 | SIM-FORM-040..042 |

**`combat.json5`** (§6)

| Field | Default | Rule |
|---|---|---|
| `base_hit` / `hit_scale` | 0.5 / 0.5 | SIM-CMBT-011 |
| `min_hit` / `max_hit` | 0.05 / 0.95 | SIM-CMBT-011 |
| `min_damage` | 1 | SIM-CMBT-013 |
| `engage_radius` / `retarget_period_ticks` / `reach_slack` | 3 / 4 / 0.5 | SIM-CMBT-002 |
| `charge_window_ticks` / `charge_dmg_share` | 60 / 0.5 | SIM-CMBT-015 |
| `charge_distance` / `pursue_repath_ticks` | 30 / 20 | SIM-CMBT-004 |
| `charge_mass_mult` | 2.0 (*chosen*) | SIM-CMBT-015 |
| `brace_integrity` | 0.7 | SIM-CMBT-015 |
| `flank_dmg_mult` / `rear_dmg_mult` | 1.25 / 1.5 | SIM-CMBT-014 |
| `flank_def_mult` / `rear_def_mult` | 0.8 / 0.6 | SIM-CMBT-014 |
| `height_defence` / `height_range` / `height_ref` | 0.15 / 0.2 / 5 | SIM-CMBT-016, SIM-PROJ-002 |
| `second_rank_reach_bonus` | 1.0 | SIM-CMBT-012 |
| `exp_step` | 0.03 | SIM-CMBT-017 |
| `pursuit_hit_mult` | 1.5 | SIM-MOR-034 |
| `corpse_ticks` | 600 (*chosen*) | SIM-CORE-008 |
| `attack_move_radius` | 40 (*chosen*) | SIM-CMBT-005 |
| `projectile_cap` | 8192 | SIM-PROJ-008 |
| `projectile_radius` | 0.3 | SIM-PROJ-006 |
| `scatter_scale` | 0.17 (tuned in T2-031: 0.15 killed a mean 34 hastati on the §15.3 row 5 band, 0.20 a mean 17; 0.17 gives a mean 27 with every seed inside 15–35) | SIM-PROJ-004 |
| `direct_apex` / `gravity` | 2 / 9.81 | SIM-PROJ-005 |
| `shield_mult` | 0.5 | SIM-PROJ-006 |
| `stat_hit_base` | 0.36 (tuned in T2-032 on the §15.3 row 6 band: 0.6 killed a mean 57 hastati against the simulated path's 27, 0.30 a mean 20; 0.36 gives a mean 27.1, 0.0 % off the simulated mean) | SIM-PROJ-008 |
| `friendly_block_dist` | 15 | SIM-PROJ-009 |
| `volley` | true | SIM-PROJ-003 |
| `ranged_retarget_ticks` | 10 | SIM-PROJ-001 |

**`morale.json5`** (§7)

| Field | Default | Rule |
|---|---|---|
| `t_unsettled` / `t_shaken` / `t_broken` / `t_routing` | 70 / 50 / 30 / 15 | SIM-MOR-003 |
| `hysteresis` | 5 | SIM-MOR-003 |
| `rally_margin` / `rally_safe_radius` | 15 / 50 | SIM-MOR-031 |
| `max_routs` / `shatter_strength` | 2 / 0.25 | SIM-MOR-032 |
| `general_death_shock` | 20 | SIM-MOR-014 |
| `rout_shock` / `rout_shock_radius` | 5 / 30 | SIM-MOR-033 |
| `disengage_penalty` | 5 | SIM-MOR-025 |
| `charged_penalty` | 8 (half from the front) | SIM-MOR-026 |
| `casualty_rate_ref` / `casualty_total_ref` | 0.05 / 0.5 | SIM-MOR-010, 011 |
| `fatigue_start` | 0.5 | SIM-MOR-012 |
| `ally_radius` / `allies_ref` / `routing_ref` | 40 / 3 / 2 | SIM-MOR-015, 016 |
| `outnumber_ref` / `outnumber_radius` | 2 / 30 | SIM-MOR-020 |
| `engage_fatigue_ticks` | 2400 | SIM-MOR-022 |
| `safe_radius` | 60 | SIM-MOR-024 |
| `exp_bonus` | 0.02 (*chosen*) | SIM-MOR-001 |
| `w.<factor>` | see below | SIM-MOR-002 |
| `state_mults.<state>` | see below | SIM-MOR-004 |

Factor weights `w` (points per second at full effect): casualty_rate −6, casualty_total −2 (per second level), fatigue −1.5, general_aura +1, allies_near +1, allies_routing −3, high_ground +0.5, fear −4, flanked −3, outnumbered −2, integrity −1.5, engaged_duration −1, winning +2, recovery +3.

State multipliers `state_mults` (attack / defence / attack interval / speed): steady 1 / 1 / 1 / 1; unsettled 0.95 / 0.95 / 1.05 / 1; shaken 0.85 / 0.85 / 1.15 / 1; broken 0.7 / 0.7 / 1.3 / 1; routing 0 / 0.5 / 1 / 1.1 (routing soldiers never attack, so the interval is 1). Shattered uses the routing row.

**`fatigue.json5`** (§8)

| Field | Default | Rule |
|---|---|---|
| `rate_idle` | −0.010 | SIM-FAT-002 |
| `rate_walk` / `rate_march` / `rate_run` | 0.004 / 0.002 / 0.020 | SIM-FAT-002 |
| `rate_fighting` / `rate_routing` | 0.015 / 0.020 | SIM-FAT-002 |
| `armour_rate` | 0.0002 (*chosen*) | SIM-FAT-002 |
| `thresholds` | [0.25, 0.5, 0.75] | SIM-FAT-003 |
| `speed_loss` / `attack_loss` / `defence_loss` / `interval_gain` | 0.3 / 0.3 / 0.2 / 0.4 | SIM-FAT-004 |

**`general.json5`** (§9)

| Field | Default | Rule |
|---|---|---|
| `aura_radius` / `aura_attack` | 60 / 0.05 | SIM-GEN-002 |
| `aura_per_rank` | 5 (*chosen*) | SIM-GEN-002 |
| `hp_mult` | 3 | SIM-GEN-001 |
| `wounded_hp` | 0.3 | SIM-GEN-004 |

**`visibility.json5`** (§11)

| Field | Default | Rule |
|---|---|---|
| `period_ticks` | 10 | SIM-VIS-004 |
| `conceal_radius` | 25 | SIM-VIS-003 |
| `height_bonus` | 0.5 | SIM-VIS-001 |
| `eye_height` / `los_sample` | 1.7 / 4 | SIM-VIS-002 |
| `memory_ticks` | 400 | SIM-VIS-005 |

**`battle_flow.json5`** (§12)

| Field | Default | Rule |
|---|---|---|
| `time_limit_ticks` | 48000 | SIM-FLOW-012 |
| `deploy_timeout_ticks` | 0 (none) | SIM-FLOW-011 |
| `pursuit_ticks` | 2400 | SIM-FLOW-015 |
| `fled_return_fraction` | 0.5 | SIM-FLOW-018 |
| `timeout_winner` | `defender` | SIM-FLOW-013 |
| `exp_per_kill` / `exp_survive` | 0.01 / 1 | SIM-FLOW-018 |
| `loot_per_enemy_killed` | 10 (*chosen*) | SIM-FLOW-018 |

The `ai.*` tunables of §13 (`army_period_ticks`, `regiment_period_ticks`, the distances) are not a rules file: they are fields of the `AiProfile` content kind (§15.4, T2-080).

### 15.2 Example unit types

| Field | `rome:hastati` | `rome:velites` | `greece:hoplite` | `persia:cavalry` | `persia:archer` (*chosen*, T2-030) | `rome:general` / `greece:general` / `persia:general` (*chosen*, T2-043) |
|---|---|---|---|---|---|---|
| category | infantry | skirmisher | infantry | cavalry | ranged | general |
| soldier_radius / mass | 0.4 / 80 | 0.4 / 70 | 0.4 / 85 | 0.7 / 400 | 0.4 / 70 | 0.4 / 85 (persia: 0.6 / 450, mounted) |
| hp | 100 | 80 | 110 | 160 | 80 | 120 (× `general.hp_mult` in battle) |
| speed_walk / run / march | 1.6 / 4.0 / 1.6 | 1.8 / 4.5 / 1.8 | 1.4 / 3.6 / 1.4 | 3.0 / 9.0 / 3.0 | 1.6 / 4.0 / 1.6 | as the faction's bodyguard unit: 1.6 / 4.0 / 1.6, 1.4 / 3.6 / 1.4, 3.0 / 9.0 / 3.0 |
| attack / defence / armour / damage | 35 / 30 / 8 / 30 | 25 / 20 / 2 / 25 | 32 / 38 / 10 / 30 | 38 / 25 / 8 / 35 | 20 / 18 / 2 / 20 | 40 / 35 / 12 / 30 |
| attack_interval_ticks / reach | 30 / 0.6 | 32 / 0.5 | 34 / 1.2 | 30 / 1.0 | 32 / 0.5 | 20 / 0.6 (persia 1.0) |
| charge_bonus / anti_cavalry_bonus | 0.3 / 0 | 0.1 / 0 | 0.15 / 0.5 | 0.8 / 0 | 0.05 / 0 | 0.2 / 0 |
| second_rank_attack / shield | false / true | false / false | true / true | false / false | false / false | false / true |
| frontal_arc_deg / armour_penetration | 120 / 0 | 120 / 0 | 120 / 0 | 120 / 0 | 120 / 0 | 120 / 0 |
| ranged | pilum: range 25, min 5, acc 0.6, speed 20, reload 120, ammo 2, dmg 40, pen 0.5, direct | javelin: range 40, min 5, acc 0.5, speed 20, reload 80, ammo 8, dmg 30, pen 0.3, direct | none | none | bow: range 120, min 15, acc 0.35, speed 40, reload 100, ammo 20, dmg 25, pen 0.2, indirect | none |
| morale_base / los_radius | 60 / 200 | 50 / 250 | 65 / 200 | 60 / 300 | 45 / 250 | 80 / 250 |
| formations | line, column, loose, square (T2-080) | loose, line, column | phalanx, line, column, square (T2-080) | wedge, line, column | loose, line, column | the bodyguard unit's (unused: the general rides in its bodyguard's formation) |
| abilities (T2-050) | testudo | none | shield_wall | war_cry | none | none |
| energy_max / energy_regen (SIM-ABIL-006) | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| cost / upkeep / recruit_turns / regiment_size | 400 / 60 / 1 / 120 | 250 / 40 / 1 / 120 | 450 / 70 / 1 / 160 | 800 / 120 / 2 / 60 | 300 / 45 / 1 / 120 | 1000 / 100 / 4 / 1 |

The generals share one placeholder sprite set (`rome:sprites_general`) and ride at their faction's usual bodyguard pace; every value is *chosen*. The archer is the flagship's only indirect-fire unit (SIM-PROJ-005's lob, SIM-PROJ-009's fire over friends); at 40 m/s a 45° launch reaches 163 m, so the speed cap never bites inside its 120 m range.

The flagship abilities (T2-050, every value *chosen*; `content/abilities/`):

| Field | `rome:testudo` (hastati) | `greece:shield_wall` (hoplites) | `persia:war_cry` (cavalry) |
|---|---|---|---|
| targeting / range / radius | self / — / — | self / — / — | regiment_enemy / 60 / — |
| cooldown_ticks / duration_ticks | 1200 / 400 | 1200 / 600 | 1800 / 300 |
| effects | armour ×3.5 (buff), speed ×0.5, attack ×0.8 (debuffs) | defence ×1.25, armour +2 (buffs), speed ×0.7, attack_interval ×1.2 (debuffs) | morale_per_s −2, attack ×0.9 (debuffs) |
| stacking / max_stacks | refresh / 1 | refresh / 1 | refresh / 1 |
| requires_not_engaged / not_moving | true / false | false / false | false / false |

Testudo's armour factor was 2.0 at first: against the javelins of row 5 (damage 30, penetration 0.3, frontal shield ×0.5) that only trims a hit from 12.2 to 9.4 and left the hastati with 74 % of their bare losses; ×3.5 (armour 28, a hit 5.2) gives 46 % (row 8: 12.3 against 27.0 over 50 seeds).

### 15.3 Scenario tests

Outcome bands over 50 seeds; a failing band means a formula or default needs review, not that the test is wrong. Each band is a file under `tests/scenarios/bands/`: an ordinary scenario (`BattleSetup` plus scripted `commands`) with a `bands` block giving the seed count and base, a tick limit and the assertions; `il_cli bands` runs them and `tests/tests/scenarios.rs` drives it nightly (TDD §17, T2-110). Every seed stops at the tick limit or when a side has no living soldiers. Assertion kinds: `winner` (the side annihilates every other side or ends with the strictly higher surviving fraction), `casualties` (fraction dead at the end or a number of ticks after a regiment's first contact; since T2-042 soldiers that fled the field are neither survivors nor casualties, and `winner` counts the soldiers still on the field), `routed_before_loss` (dead fraction when the side first routs) and `rout_within` (read the Routing morale state). An assertion holds when its per-seed boolean is true in the required fraction of seeds. A band file may name `mods` (folders relative to the file, loaded after the game and the `--mod` folders for that file only), and the cross-file kind `mean_loss_matches { side, reference, tolerance }` compares the mean soldiers `side` lost over the file's seeds with the mean in the band file `reference` of the same run, within `tolerance`, and `mean_loss_below { side, reference, ratio }` requires the file's mean to be at most `ratio` times the reference's (T2-050); both are settled once every file has run (T2-032). A file may also list `pin_morale` sides whose regiments are held at morale 100 after every tick (T2-042; the volley rows). The rout clauses became active in T2-042. The melee files script `FireMode: Hold` at tick 1 for every regiment that carries pila or javelins (rows 1, 2 and 4) so they stay melee-only now that throwing exists (T2-030).

| Scenario (file) | Melee clause | Morale clause (active since T2-042) |
|---|---|---|
| 120 hastati (line) vs 120 velites (loose), melee only, flat (`melee_hastati_vs_velites`) | Hastati win 90–100 % of seeds (50/50 on 2026-09-04, mean 111 hastati and 6 velites left; 50/50 on 2026-09-05 with morale, the velites breaking and fleeing north, mean 120 hastati left). | Velites rout before losing 50 %, in ≥ 90 % of seeds (50/50 on 2026-09-05). |
| 160 hoplites (phalanx) vs 160 hastati frontal (`melee_hoplites_vs_hastati`) | Hoplites win ≥ 70 % (50/50 melee-only on 2026-09-04; 50/50 with morale on 2026-09-05: the hastati break first in every seed, mean 160 hoplites left). | The 70–90 % window (hastati winning 10–30 %) stays an open tuning target (T2-113): no §15.1 default makes a frontal assault on a phalanx a coin flip, and the band carries no ceiling until one does. |
| 160 hoplites vs 60 Persian cavalry frontal charge (`melee_hoplites_vs_cavalry`) | Hoplites win 85–100 % (50/50 on 2026-09-05, mean 159 left). Melee-only (2026-09-04) the cavalry had lost about 20 % at 30 s and 30 % by 60 s in every seed. | The cavalry rout within 60 s of their first contact in ≥ 85 % of seeds (50/50 on 2026-09-05: the wedge breaks about 16 s after contact at 5–10 % losses and 55 of 60 flee north), the reading the melee-era note anticipated; it replaced the casualty clause. |
| 60 Persian cavalry rear-charge 120 engaged hastati (`melee_cavalry_rear_charge`) | The charged side loses (side 1 wins) in ≥ 80 % of seeds (50/50 on 2026-09-04 and 2026-09-05). Since T2-042 the front is 80 hoplites, not 120, and the cavalry set off at tick 100 at `run`, not at tick 800 at `walk`: against 120 hoplites the hastati broke on their own eight seconds after contact, and a walking wedge only ran the last 30 m and arrived after the rout. | Hastati rout within 30 s of the charge in ≥ 80 % of seeds (50/50 on 2026-09-05: cavalry contact at tick 408, the rout within a second of it, about 95 of 120 hastati fleeing). |
| 120 velites (loose) fire 8 volleys at 120 hastati (line) at 35 m, no approach (`volley_velites_vs_hastati`) | Hastati lose 15–35 soldiers (`casualties` 0.125–0.292 of 120 in ≥ 90 % of seeds). Measured 2026-09-04 over 50 seeds after tuning `scatter_scale` to 0.17: 50/50, mean 27 lost (the loose line's wings stand beyond 40 m, so about 107 of the 120 velites throw each volley; every javelin lands, roughly half on a soldier); 50/50 and mean 27.1 again on 2026-09-05; 47/50 and mean 27.0 on 2026-09-06 with the general riding in each regiment (T2-043). | The file holds the hastati at morale 100 (`bands.pin_morale: [1]`, T2-042): unpinned they break after the third volley and run north, and only about 16 die. |
| Statistical vs simulated projectile path, same volley (`volley_statistical`: the row 5 file run with `mods: ["../../mods/projectile_cap_zero"]`) | Mean hastati casualties within 10 % of `volley_velites_vs_hastati` over the same 50 seeds (`mean_loss_matches`), and the same 15–35 band. Measured 2026-09-04 after tuning `stat_hit_base` to 0.36: mean 27.1 lost against 27.1 simulated (0.0 %), 50/50 inside the band; the same on 2026-09-05 (the hastati pinned as in row 5); 26.6 against 27.0 (1.8 %), 49/50, on 2026-09-06 with the generals. The statistical victim (the target regiment's soldier nearest the aim point) concentrates hits on the front rank more than a landing query does, which is why the base sits well under the simulated hit rate. | — |
| General killed at tick 600 in an even fight (`general_death_hastati_vs_hoplites`: 160 hastati a side, mirrored, 100 m apart; "hastati vs hoplite" is read as mirrored hastati because a phalanx beats hastati frontally every time, row 2; the harness kills side 0's general before tick 600 is stepped, `bands.harness`) | The side without a general routs first in ≥ 75 % of seeds (`routs_first`). Measured 2026-09-06 over 50 seeds: 50/50 (contact at tick 555, the shocked side breaks at about 745, the other never); at 50 m apart one side had broken by tick 400, before the kill. | — |
| 120 velites fire 8 volleys at 120 hastati in testudo (`volley_testudo`: row 5 with `UseAbility rome:testudo` at tick 1) | Hastati lose at most 60 % of what row 5's hastati lose over the same 50 seeds (`mean_loss_below`), and 3–20 soldiers (`casualties` 0.025–0.167 in ≥ 90 % of seeds). Tuned 2026-09-06: at armour ×2 the shielded hastati still lost 74 % of the bare figure; at ×3.5 the band measured 12.3 lost against 27.0 (46 %), 50/50 inside 3–20, over 50 seeds in release. | The file pins the hastati at morale 100 like row 5. |
| The engine AI (side 1) against a passive player army (side 0, no commands; 3 × 120 hastati, 120 velites, 60 archers, 60 cavalry and a 20-man bodyguard a side, pre-placed 250 m apart; `ai_vs_passive`) | The AI wins every seed (`winner` side 1, 20 seeds, `min_fraction` 1.0): the Phase 2 exit criterion. Measured 2026-09-06 in release: 3/20, an open tuning target (the T2-082 finding in docs/07: the assault breaks on the contact casualty shock as often as the defender does). | — |
| The same armies, the player charging everything straight ahead at tick 1 (`AttackMove` for every regiment; `ai_vs_charge`) | The AI wins in more than 60 % of seeds (20 seeds, `min_fraction` 0.6). Measured 2026-09-06 in release: 16/20 (borderline; the same finding). | — |
| Determinism | Every scenario above: identical hash on run 1 and run 2, and after snapshot/restore at tick 1,000 (T2-112 enrols the band files in the determinism test; `tests/scenarios/ai_skirmish_300.json5`, both sides the engine's, sits in the corpus since T2-082). | — |

### 15.4 AI content (T2-080)

Two kinds under `content/ai/` (Modding SDK §4.8): an **AI profile** (`ai/profiles/*.json5`, one per faction `ai_profile`, overridable per side by `BattleSetup.sides[].ai_profile`) carries the personality and the §13 tunables, every field required like a rules file; an **AI action set** (`ai/actions/*.json5`) lists one scope's candidate actions with their considerations (SIM-AI-001). The flagship ships `rome:default_ai` (named by all three factions) with `rome:army_default` and `rome:regiment_default`; every value is *chosen* and the T2-081/082 bands tune them.

| Profile field | Default | Rule |
|---|---|---|
| `aggression` / `general_aggression` / `reserve_fraction` / `stance_margin` | 0.6 / 0.3 / 0.2 / 0.1 | SIM-AI-010, SIM-AI-022, SIM-AI-014, hysteresis (a new stance must beat the current stance's score by the margin; retreat is exempt) |
| `army_period_ticks` / `regiment_period_ticks` | 40 / 20 | SIM-AI-002 |
| `approach_distance` / `advance_step` / `line_tolerance` | 150 / 8 / 12 m | SIM-AI-011 (the line steps `advance_step` toward the enemy per army period once its mean slot error is under `line_tolerance`) |
| `skirmish_range_frac` / `flank_offset` / `charge_trigger_dist` | 0.9 / 40 m (tuned from 80: the wider hook fell into the test map's river) / 60 m (tuned in T2-082 from 40: a line that walks the last 60 m under two missile units' fire breaks before contact) | SIM-AI-011 |
| `defend_search_radius` / `counter_charge_dist` | 200 m / 30 m | SIM-AI-012 |
| `screen_offset` / `screen_gap` | 40 m / 80 m | SIM-AI-013 |
| `reserve_offset` / `commit_morale` | 60 m / 45 | SIM-AI-014 |
| `action_sets` | one army set, one regiment set | SIM-AI-001 |
| `campaign` | absent | Phase 4 (`army_strength_target`, `composition`, `min_garrison`; parsed and hashed, unread) |

An action is `{ name, kind, base (1), threshold (0), noise (0), considerations: [{ input, scale (1), curve }] }` plus `ability` for `use_ability` and `layout` for `switch_formation`. Regiment kinds: `engage_nearest`, `hold_position`, `fall_back`, `follow_centroid` (movement channel), `switch_formation` (formation), `fire_at_will`, `hold_fire` (fire), `use_ability` (one channel per ability slot; several actions may name one ability, an OR of their conditions). Army kinds: `attack`, `defend`, `hold`, `retreat` (stance). The regiment inputs and their raw values are the table of §13.3 (T2-081), the army inputs that of §13.2 (T2-082); `constant` is 1 in both scopes. The flagship army set (`rome:army_default`): `attack` base 1.5 (strength ratio logistic at 0.45, aggression, morale mean), `defend` 0.8 (terrain advantage, enemy visible), `hold` 0.4, `retreat` 1.2 (strength ratio falling at 0.35, casualty fraction rising at 0.35): an even army on flat ground attacks, one at about a third of the enemy's cost defends, one that has lost a third of its men against such odds retreats.
