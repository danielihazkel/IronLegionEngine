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
| SIM-CORE-005 | Each Regiment has: stable `RegimentId`, `ArmyId`, `UnitType` handle(s), soldier list, anchor `(a, θ_a)`, `FormationTemplate` handle, formation state, current order, path, `morale`, morale state, speed mode (walk/run/march), `experience`, ability cooldowns, status effects, fire state (ranged units: mode, target regiment, volley cooldown; ammo is per soldier, SIM-PROJ-003), engagement flags. | REQ-VIS-003 |
| SIM-CORE-006 | The number of Soldier entities alive plus pending reinforcements shall never exceed 32,768. `BattleSetup` validation rejects setups above the cap; reinforcements that would exceed it are dropped with an Event. | REQ-PERF-004 |
| SIM-CORE-007 | Regiment and army sizes come from `BattleSetup`; the engine imposes no minimum or maximum except the cap. | REQ-SIM-023 |
| SIM-CORE-008 | Soldiers are removed from the world at death: Stage 15 despawns them in ascending id and drops them from every regiment list, the id lists and the spatial grid in the same tick, so no later system sees a dead soldier. The `SoldierDied` event carries the position; the application keeps a corpse from it for `combat.corpse_ticks` (render-only). | — |

### 1.1 Soldier finite state machine

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CORE-010 | Soldier FSM states: `Idle`, `MoveToSlot`, `Fighting`, `Routing`, `Withdrawing`, `Dead`. | REQ-AI-002 |
| SIM-CORE-011 | Transitions: `Idle ↔ MoveToSlot` when distance to slot crosses `movement.slot_arrive_radius` (enter Idle) or `movement.slot_leave_radius` (enter MoveToSlot); `→ Fighting` when a melee target is within reach (§6); `Fighting → MoveToSlot` when the target is lost and no other enemy within `combat.engage_radius`. A `Fighting` soldier does not seek its slot: it seeks its target's previous-tick position and stops at `r_i + r_j + reach` (second-rank fighters a `second_rank_reach_bonus` further back), separation and obstacle avoidance still apply, and its facing tracks the target; with no target (the target died) it holds still until its next retarget tick; `→ Routing` when the Regiment enters Routing (§7); `Routing → MoveToSlot` on Rally; `→ Withdrawing` when the Regiment withdraws; `→ Dead` when `hp ≤ 0`. | REQ-AI-002 |
| SIM-CORE-012 | Soldiers make no decisions beyond this FSM. Targets, destinations, and speed mode come from the Regiment. | REQ-VIS-003 |

## 2. Determinism contract

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-DET-001 | The battle seed is a `u64` from `BattleSetup.seed`. Each system owns a stream seeded as `hash(seed, stream_id)`. Streams: `combat_melee`, `combat_ranged`, `morale`, `ai_regiment`, `ai_army`, `abilities`, `deployment`, `weather`, `campaign`. | REQ-SIM-004 |
| SIM-DET-002 | Per-entity randomness that must not depend on entity iteration order is drawn as `hash(stream_seed, tick, entity_id, draw_index)` rather than from the sequential stream. Melee hit rolls and ranged scatter use this form. | REQ-SIM-004, REQ-SIM-007 |
| SIM-DET-003 | Every system that iterates entities and produces order-dependent results iterates in ascending stable id (`SoldierId`, `RegimentId`), never in ECS storage order. | REQ-SIM-007 |
| SIM-DET-004 | The state hash at the end of a tick covers, in this order: tick number; battle phase; per Regiment (ascending id) `morale`, morale state, soldier count, anchor (position, facing), order (kind, target, target regiment, facing, speed mode, since), formation state (template, ranks, files, integrity, `morph_until`, `needs_reform`, prior template, laid-out facing), path (waypoints with corridor widths, next, requested), fire state (present only for units with `ranged`: mode, target regiment, volley cooldown), combat state (engaged, last fighting tick, charge window end, experience, kills), casualty ring and initial strength; per Soldier (ascending id) `p`, `v`, facing `θ`, `hp`, `fatigue`, FSM state, slot, melee target, attack cooldown, ranged state (present only for units with `ranged`: ammo, reload cooldown); per Projectile (ascending id) id, shooter, shooter regiment, side, launch and land tick, start, end, apex, arc, damage, penetration (the position is derived from these, SIM-PROJ-005); the pending damage queue in queue order (apply tick, target, damage, shooter, shooter regiment); RNG stream states. Positions are hashed by their `S` bit pattern. (Phase 1 layout fixed in T1-047; the combat fields were appended in T2-020; the regiment ammo gave way to the fire, ranged and projectile fields in T2-030.) | REQ-SIM-005 |
| SIM-DET-005 | A snapshot contains everything the hash covers plus everything needed to continue (paths included, since a re-requested path would differ from the one in flight); cooldowns, status effects, timers, the projectiles in flight and the pending damage queue are stored; spatial and nav grids, flow fields, slot tables, ranks, attacker counts and the per-tick targeting gates are recomputed on restore (derived data is never stored). Restoring and stepping shall produce the same hash sequence as the uninterrupted run. | REQ-SIM-006 |
| SIM-DET-006 | No system reads wall-clock time, thread ids, allocation addresses, or environment. | REQ-TECH-008 |
| SIM-DET-007 | The stage order of §6.2 in the SAD is part of the determinism contract. | REQ-SIM-001 |
| SIM-DET-008 | Pause and speed multipliers do not exist inside the sim; they are app-level accumulator behaviour, but the `Pause`/`SetSpeed` Commands are recorded in the stream so replays and peers reproduce the player's experience. The sim applies them as no-ops. | REQ-SIM-031 |

## 3. Command model

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CMD-001 | A Command is `{ tick, player, seq, kind }`. Commands are applied at Stage 0 of `tick`, sorted by `(player, seq)`. Commands for past ticks are rejected with an Event (never silently reordered). | REQ-SIM-003, REQ-NET-001 |
| SIM-CMD-002 | Command kinds (battle): `Move { regiments, target, facing, speed_mode }`, `AttackRegiment { regiments, target_regiment }`, `AttackMove { regiments, target }`, `Halt { regiments }`, `SetFormation { regiments, template, ranks }`, `SetFacing { regiments, facing }`, `SetSpeedMode { regiments, mode }`, `GroupFormation { regiments, group_template, anchor, facing, width }`, `FireMode { regiments, mode }` (fire_at_will / hold / target), `UseAbility { regiment, ability, target }`, `Withdraw { regiments }`, `Deploy { regiment, position, facing, template }`, `ConfirmDeployment`, `Pause`, `SetSpeed { mult }`, `Surrender`, `TransferControl { from, to }` (hands every regiment of `from` to `to`; `to = 255` means engine AI; used for drop-to-AI in multiplayer and for "let the AI command this side" in single-player). | REQ-INP-006, REQ-SIM-030..033, REQ-NET-008 |
| SIM-CMD-003 | A Command referencing a Regiment not owned by `player` is rejected with an Event. AI players own their factions' regiments; `PlayerId(255)` is the engine AI and may own regiments transferred to it. | REQ-NET-001 |
| SIM-CMD-004 | A Command referencing a Routing or Shattered regiment is rejected except `Withdraw` (ignored) and none others; Routing regiments cannot be ordered (SIM-MOR-020). | REQ-MOR-004 |
| SIM-CMD-005 | AI decisions are emitted as Commands for tick `t + 1` during Stage 1 of tick `t`, tagged with the AI player id, and pass through the same validation. | REQ-AI-005 |

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
| SIM-MOVE-011 | Regiment speed is the minimum of the unit type speed for the speed mode and the speed of its slowest soldier category in mixed regiments, times formation `speed_mult`, times terrain and slope factors at the anchor. | REQ-PATH-005 |
| SIM-MOVE-012 | Cohesion: if the fraction of soldiers farther than `movement.straggler_radius` (default `3 × sf`) from their slot exceeds `movement.straggler_fraction` (default 0.25), the anchor speed is scaled by `movement.straggler_slowdown` (default 0.5) until they catch up. | REQ-PATH-006 |
| SIM-MOVE-013 | On arrival at the final target the regiment sets `θ_a` to the ordered facing (if given) and stops; soldiers finish moving to slots. | REQ-FORM-005 |

### 5.3 Soldier steering

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOVE-020 | Soldier max speed: `v_max = unit.speed_<mode> × fatigue_speed_mult(F) × zone.move_mult × slope_mult × status_mult`. Speed modes: `walk`, `run`, `march` (march is walk speed with reduced fatigue accumulation and cannot be used within `combat.engage_radius` of an enemy). | REQ-PATH-005, REQ-FAT-003 |
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
| SIM-FLOW-001 | At battle start each side gets an escape flow field over the nav grid toward its own map edge (deployment edge): a Dijkstra from all passable edge cells, storing per cell the direction to the lowest-cost neighbour. | REQ-PATH-004 |
| SIM-FLOW-002 | Routing and Withdrawing soldiers set `v_des = field(p) × v_max` and apply separation and avoidance as normal. When a soldier reaches an edge cell it leaves the battle (Routing: counted as fled; Withdrawing: counted as survivor). | REQ-PATH-004, REQ-SIM-033 |
| SIM-FLOW-003 | Flow fields are recomputed only when the nav grid changes (SIM-MOVE-006). | — |

## 6. Combat

### 6.1 Engagement and targeting

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CMBT-001 | A soldier is in melee reach of enemy `j` if `|p_i − p_j| ≤ r_i + r_j + unit_i.reach`. | REQ-CMBT-002 |
| SIM-CMBT-002 | Melee targeting runs every `combat.retarget_period_ticks` (default 4) per soldier (staggered by `id % period`): if the current target is alive and within `r_i + r_j + reach + combat.reach_slack` (default 0.5 m), keep it; else choose, among enemy soldiers whose centre lies within `combat.engage_radius` (default 3 m) in the spatial grid, the one with the fewest attackers (`attacker_count` ascending), then the nearest, then the lowest id. Soldiers without a target within `engage_radius` return to `MoveToSlot`. Only Idle, MoveToSlot and Fighting soldiers of regiments that may fight (Idle or attacking order, not Routing/Shattered, soldiers left) take part, and only when an enemy regiment lies within the two regiments' extents plus `engage_radius` of the anchor (a per-regiment gate, so distant armies cost no per-soldier work). Attacker counts are recomputed after targeting in ascending soldier id. | REQ-CMBT-003 |
| SIM-CMBT-003 | A regiment is engaged if any soldier is `Fighting` (recomputed after targeting; the false-to-true edge emits `Engaged`). Engaged regiments ignore `Move` orders' facing but obey the move (disengage), taking a morale penalty (SIM-MOR-025). While an attacking regiment (SIM-CMBT-004) is engaged its anchor holds and its path is kept; pursuit re-paths once no soldier has fought for `combat.retarget_period_ticks`. | — |
| SIM-CMBT-004 | Regiments in `AttackRegiment` or `AttackMove` orders path to the target regiment's anchor (re-pathed every `combat.pursue_repath_ticks`, default 20, staggered by regiment id and also on the tick the order is issued; the order stores the target regiment; reaching the anchor of a target that has moved does not end the order; an `AttackRegiment` whose target has no living soldiers halts; `AttackRegiment` is rejected with `InvalidTarget` for an own-side or empty target) and switch to `run` within `combat.charge_distance` (default 30 m) if `unit.charge_bonus > 0`; the speed mode stays `run` until another order changes it. | REQ-CMBT-005 |
| SIM-CMBT-005 | An `AttackMove` regiment has no target regiment until, on one of its re-path ticks, an enemy regiment with living soldiers has its anchor within `combat.attack_move_radius` (default 40 m) of the regiment's anchor; the nearest such regiment (ties by ascending id) becomes the target and SIM-CMBT-004 applies. When the target has no living soldiers the regiment resumes the move to its original point. | REQ-CMBT-005 |

### 6.2 Melee resolution

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-CMBT-010 | Each `Fighting` soldier has a cooldown; when it reaches 0 it attacks its target and resets to `unit.attack_interval_ticks × fatigue_interval_mult(F) × morale_interval_mult(M) × status_mult`, rounded to nearest tick, minimum 2. Initial cooldown on entering `Fighting` is `rng-free`: `(id % attack_interval_ticks)` to stagger. | REQ-CMBT-001, REQ-CMBT-007 |
| SIM-CMBT-011 | Hit roll: `A = unit_i.attack × fatigue_attack_mult(F_i) × morale_attack_mult(M_i) × (1 + template_i.integrity_bonus_attack × I_i) × charge_mult × experience_mult × status_mult`; `D = unit_j.defence × fatigue_defence_mult(F_j) × morale_defence_mult(M_j) × (1 + template_j.integrity_bonus_defence × I_j) × flank_defence_mult × terrain_defence_mult × status_mult`. Hit probability `P = clamp(combat.base_hit + combat.hit_scale × (A − D) / (A + D), combat.min_hit, combat.max_hit)`; defaults 0.5, 0.5, 0.05, 0.95. The attack hits if `rng.combat_melee(tick, id_i, 0) < P` (draw index 0; later draws use 1, 2, …). `status_mult` is 1 until T2-050 and the general's aura multiplier 1 until T2-043; a braced anti-cavalry defender attacking cavalry (SIM-CMBT-015) multiplies `A` by `1 + anti_cavalry_bonus`. | REQ-CMBT-001 |
| SIM-CMBT-012 | Second-rank attack: a soldier whose slot is in rank 1 (second rank) and whose unit has `second_rank_attack` (or is in Phalanx) may target enemies in reach of the soldier in the slot directly ahead, using its own `reach + combat.second_rank_reach_bonus` (default 1.0 m). | REQ-CMBT-003 |
| SIM-CMBT-013 | Damage on hit: `dmg = max(unit_i.damage × charge_dmg_mult × flank_dmg_mult × experience_mult − unit_j.armour × (1 − unit_i.armour_penetration), combat.min_damage)` (default 1); `armour_penetration` is the unit's top-level melee field (default 0), distinct from `ranged.armour_penetration`. `hp_j −= dmg`. | REQ-CMBT-001 |
| SIM-CMBT-014 | Frontal arc: an attack is frontal if the attacker lies within `±unit_j.frontal_arc_deg / 2` (default 120°) of the defender's facing; flank if within ±150° (an engine constant, `FLANK_HALF_ARC_DEG`); rear otherwise. The arc is measured from the defending soldier's own facing, which tracks its target while it fights (SIM-CORE-011), so a flank or rear attack stays one only until the defender turns. `flank_dmg_mult` and `flank_defence_mult`: front 1.0/1.0, flank `combat.flank_dmg_mult` (1.25) / `combat.flank_def_mult` (0.8), rear `combat.rear_dmg_mult` (1.5) / `combat.rear_def_mult` (0.6). | REQ-CMBT-004 |
| SIM-CMBT-015 | Charge: when a regiment in `run` mode first gains an engaged soldier (the tick `engaged` turns true while the speed mode is `run` and no window is open; `charge_until` on the regiment marks the window's end and a `Charge` event names the regiment its first fighter struck), all its soldiers get `charge_mult = 1 + unit.charge_bonus` and `charge_dmg_mult = 1 + unit.charge_bonus × combat.charge_dmg_share` (default 0.5) for `combat.charge_window_ticks` (default 60). A defender unit with `anti_cavalry_bonus > 0`, not moving (an Idle order, or engaged), with `I ≥ combat.brace_integrity` (default 0.7), facing the charge within its frontal arc, negates the attacker's charge bonus if the attacker is cavalry and gains `attack × (1 + anti_cavalry_bonus)` versus cavalry. Charge push: while the window is open the regiment's soldiers push with mass `unit.mass × combat.charge_mass_mult` (default 2.0) in collision resolution (SIM-MOVE-040), so a charge shoves lighter defenders back without any extra force. | REQ-CMBT-005, REQ-CMBT-006 |
| SIM-CMBT-016 | Terrain defence: `terrain_defence_mult = zone.defence_mult × ford_mult × (1 + combat.height_defence × sat((h_j − h_i) / combat.height_ref))` where `zone` is the defender's zone type (`defence_mult` default 1; forest 1.1, marsh 0.8), `ford_mult = movement.ford_defence_mult` when that zone type has `ford: true` and 1 otherwise (SIM-MOVE-032), and `sat` clamps to [−1, 1]; defaults 0.15 and 5 m. | REQ-SIM-040 |
| SIM-CMBT-017 | Experience: regiment `experience` in [0, 9]; `experience_mult = 1 + combat.exp_step × experience` (default 0.03). | REQ-CAMP-042 |
| SIM-CMBT-018 | Attack results (hit or miss, damage, arc) are recorded in a shared buffer during the parallel phase, sorted by attacker id and applied in that order (one attack per attacker per tick makes the order total); a soldier whose hp crosses zero is queued with its killer and the killer's regiment for Stage 15, and damage on a soldier already at or below zero is applied but credits nobody. Projectile damage lands after the melee outcomes, at Stage 11 (SIM-PROJ-006), under the same crossing rule. Deaths are resolved in Stage 15: the queued kills sorted by victim id, each leaving its regiment's soldier list and slot assignment (`needs_reform` set, SIM-FORM-021), adding one to the regiment's casualty ring slot of the tick (SIM-MOR-010) and one kill to the killer's regiment, clearing every melee target that pointed at it, then despawned. | REQ-SIM-008 |

### 6.3 Ranged and projectiles

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-PROJ-001 | A ranged regiment in `fire_at_will` selects, every `combat.ranged_retarget_ticks` (default 10; staggered by regiment id, and at once when it has no target), the visible enemy regiment with the most soldiers inside its range annulus `[min_range, range × range_mult]` measured from the shooter's anchor (indirect arc ignores LOS occlusion by soldiers but not terrain; visibility is `1` for everyone until T2-060), keeping the current target while it still has a soldier there and taking the lower id on ties. `target` mode uses the ordered regiment while it has a soldier in the annulus and falls back to `fire_at_will` once the ordered regiment has no living (or, from T2-060, visible) soldiers; `hold` fires nothing. Each ranged regiment carries `Fire { mode, target, cooldown }`; `FireMode` at Stage 0 rejects a regiment without `ranged` (`NotRanged`) and a `target` that is own-side or empty (`InvalidTarget`), and clears the target so the new mode re-acquires it at the next Stage 9. | REQ-CMBT-013 |
| SIM-PROJ-002 | `range_mult = 1 + combat.height_range × clamp((h_shooter − h_target) / combat.height_ref, −1, 1)` (default 0.2). Target selection reads the two anchors' heights; a soldier's own shot reads its position and the aimed soldier's. | REQ-CMBT-014 |
| SIM-PROJ-003 | Each soldier with `ammo > 0` (per soldier, `RangedState { ammo, cooldown }`, from `unit.ranged.ammo`), in state `Idle` or `MoveToSlot` under any order (not `Fighting`, `Routing` or `Withdrawing`), whose regiment has a target, throws when its reload cooldown reaches 0 (reset to `unit.ranged.reload_ticks × fatigue_interval_mult`, rounded as SIM-CMBT-010), aiming at the position of a target soldier chosen deterministically (`rng.combat_ranged(tick, id, 1)`-th soldier of the target regiment by ascending id, draw index 1 of the `combat_ranged` stream in the SIM-DET-002 form) predicted forward by flight time; a pick outside the soldier's own annulus `[min_range, range × range_mult]` gives way to the nearest soldier of the target regiment inside it (ties lowest id), and with none there the soldier keeps its ammo and waits. A regiment fires in volleys by sharing the cooldown phase (`combat.volley` true: the regiment's `Fire.cooldown` gates every soldier, resets to `reload_ticks ×` the largest fatigue interval multiplier among the volley's shooters, and counts down after the reset so the period is exactly `reload_ticks`); with `volley` false each soldier's own cooldown counts. `ammo −= 1` per throw. The shots of a tick are recorded in parallel and turned into projectiles in ascending shooter id; one `VolleyFired { regiment, count }` per regiment per tick. | REQ-CMBT-010, REQ-CMBT-011 |
| SIM-PROJ-004 | Scatter: the aim point is offset by a vector with angle `rng.combat_ranged(tick, id, 0) × 2π` (draw index 0) and length `d × (1 − unit.ranged.accuracy) × combat.scatter_scale × weather.accuracy_penalty` (default scale 0.15) where `d` is the distance to the aimed soldier; `weather.accuracy_penalty` is `1` until the weather rules of Phase 4. | REQ-CMBT-011 |
| SIM-PROJ-005 | Projectile motion: a `direct` arc flies straight at `projectile_speed` with height following a shallow parabola (apex `combat.direct_apex`, default 2 m); an `indirect` arc launches at 45° with the speed that lands at `d` (`sqrt(d × g)`, `g = combat.gravity` default 9.81, capped at `projectile_speed`), so its flight time is `d × √2 / v` and its apex `d / 4`. The flight time is rounded up to whole ticks (at least 1) and the landing point, apex and `land_tick` are fixed at launch; the position at tick `t` is the closed form `start + (end − start) × u`, height `apex × 4u(1 − u)`, `u = (t − launch) / (land − launch)` (nothing is integrated, nothing derived is stored, T2-030/031). A projectile lands when its `land_tick` arrives. | REQ-CMBT-010 |
| SIM-PROJ-006 | Landing (Stage 11, in ascending projectile id): query the spatial grid at the landing point for soldiers (any side) with `|p − land| ≤ r + combat.projectile_radius` (default 0.3 m); the nearest by distance (ties ascending id) is hit. A hit applies `dmg = max(ranged.damage − armour × (1 − ranged.armour_penetration), combat.min_damage) × dmg_mult(arc)` with the arc of SIM-CMBT-014 read from the victim's facing toward the shooter's launch point (`combat.flank_dmg_mult`, `combat.rear_dmg_mult`); if `unit.shield` and the impact is frontal, `dmg × combat.shield_mult` (default 0.5). The damage is queued (`PendingDamage`: apply tick, target, damage, shooter, shooter regiment) and applied the same tick in `(apply tick, target id)` order, queue order breaking ties, after the melee outcomes of Stage 10; a soldier whose hp crosses zero is queued for Stage 15 with the shooter and its regiment (which is credited even if the shooter has since fallen), and a soldier already at or below zero credits nobody twice. Every landing emits `ProjectileLanded { pos, hit, victim }`. | REQ-CMBT-010, REQ-CMBT-012 |
| SIM-PROJ-007 | Projectiles in flight are blocked by walls higher than their current `z` at the crossing point (Phase 5) and never by soldiers. | REQ-SIM-043 |
| SIM-PROJ-008 | Cap: if `live_projectiles ≥ combat.projectile_cap` (default 8,192), a volley is resolved statistically: for each would-be projectile, a hit occurs with probability `P_hit = combat.stat_hit_base × density(target_regiment)` where density is soldiers per m² in the target's footprint clamped to [0, 1], and the victim is chosen as in SIM-PROJ-006 from the aim point without flight; damage as SIM-PROJ-006 applied `flight_time` ticks later via a delayed-damage queue. The statistical path draws from the same `rng.combat_ranged(tick, id, k)` slots so hash sequences remain comparable in tests. | REQ-CMBT-015, REQ-PERF-008 |
| SIM-PROJ-009 | Friendly soldiers at the landing point are hit like enemies; indirect fire may be ordered over friendly regiments; a direct shot is refused if a friendly regiment's footprint (the circle around its anchor of radius `extent`, its farthest soldier's distance) intersects the first `combat.friendly_block_dist` (default 15 m) of the shooter's line of fire. The refused soldier keeps its ammo and its cooldown stays at zero, the fire mode is unchanged, and one `FireBlocked { regiment, blocker }` event is emitted per regiment per tick, so fire resumes by itself when the line clears (plan decision, T2-030). | REQ-CMBT-012, REQ-CMBT-013 |

## 7. Morale

### 7.1 Value and states

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOR-001 | Regiment morale `M ∈ [0, 100]`, initialised to `unit.morale_base × (1 + morale.exp_bonus × experience) + general_aura_bonus`, clamped. | REQ-MOR-001 |
| SIM-MOR-002 | Each tick: `M ← clamp(M + Σ_f w_f × x_f × dt_s, 0, 100)` where `dt_s = 0.05` and factors `f` are in §7.2 with weights `morale.w_<factor>` (points per second at full effect). | REQ-MOR-003 |
| SIM-MOR-003 | States by thresholds with hysteresis `morale.hysteresis` (default 5): Steady `M > t_unsettled`; Unsettled `t_shaken < M ≤ t_unsettled`; Shaken `t_broken < M ≤ t_shaken`; Broken `t_routing < M ≤ t_broken`; Routing `M ≤ t_routing`. Defaults 70 / 50 / 30 / 15. A state is left upward only when `M` exceeds the threshold plus hysteresis. | REQ-MOR-002 |
| SIM-MOR-004 | Morale multipliers per state (data table `morale.state_mults`): attack, defence, attack interval, speed. Defaults: Steady 1/1/1/1; Unsettled 0.95/0.95/1.05/1; Shaken 0.85/0.85/1.15/1; Broken 0.7/0.7/1.3/1; Routing 0/0.5/—/1.1. | REQ-CMBT-007 |

### 7.2 Factors

Each factor's `x_f` is in [−1, 1] (negative drains morale); `w_f` defaults in §15.

| Rule | Factor | `x_f` |
|---|---|---|
| SIM-MOR-010 | `casualty_rate` | `−sat(deaths_last_5s / (count × morale.casualty_rate_ref))`, ref 0.05 (5 % in 5 s = full drain). |
| SIM-MOR-011 | `casualty_total` | `−sat((initial − count) / initial / morale.casualty_total_ref)`, ref 0.5. Applied as a level, not rate: contributes `w × x` once per second. |
| SIM-MOR-012 | `fatigue` | `−sat((F_mean − morale.fatigue_start) / (1 − morale.fatigue_start))`, start 0.5. |
| SIM-MOR-013 | `general_aura` | `+1` if the regiment anchor is within the general's aura radius, else 0. |
| SIM-MOR-014 | `general_dead` | One-time shock: `M −= morale.general_death_shock` (default 20) to all regiments of the army on the tick the general dies; `−morale.general_death_shock × 0.5` for regiments already Shaken or worse. |
| SIM-MOR-015 | `allies_near` | `+sat(n_allied_steady_within_R / morale.allies_ref)` with `R = morale.ally_radius` (40 m), ref 3. |
| SIM-MOR-016 | `allies_routing` | `−sat(n_allied_routing_within_R / morale.routing_ref)`, ref 2. Includes Shattered regiments leaving. |
| SIM-MOR-017 | `high_ground` | `+sat((h_anchor − h_nearest_enemy_anchor) / combat.height_ref)`; negative if lower. |
| SIM-MOR-018 | `fear` | `−1` while any active `fear` status effect; else 0. |
| SIM-MOR-019 | `flanked` | `−0.5` if attacked from the flank arc in the last second; `−1` if from the rear; `−1` additionally if enemies engage from ≥ 3 arcs (surrounded). |
| SIM-MOR-020 | `outnumbered` | `−sat((enemy_soldiers_within_R / own_soldiers_within_R − 1) / morale.outnumber_ref)`, R 30 m, ref 2. |
| SIM-MOR-021 | `integrity` | `−sat((formation.integrity_morale_threshold − I) / formation.integrity_morale_threshold)`. |
| SIM-MOR-022 | `engaged_duration` | `−sat(ticks_engaged / morale.engage_fatigue_ticks)`, default 2,400 (2 min). |
| SIM-MOR-023 | `winning` | `+sat((enemy_deaths_5s − own_deaths_5s) / (count × morale.casualty_rate_ref))`, clamped at 0 below (losing is covered by casualty_rate). |
| SIM-MOR-024 | `recovery` | `+1` when not engaged, no enemy within `morale.safe_radius` (60 m), and not Routing. |
| SIM-MOR-025 | `disengage` | One-time `−morale.disengage_penalty` (5) when an engaged regiment is ordered away. |
| SIM-MOR-026 | `charged` | One-time `−morale.charged_penalty` (8) when receiving a charge from the flank or rear; `−4` from the front. |
| SIM-MOR-027 | `ability` | Status effects may add or subtract per-second morale via `effect.morale_per_s`. |

### 7.3 Routing, rally, shatter

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-MOR-030 | On entering Routing the regiment drops its order, sets all soldiers to `Routing`, speed mode `run`, and follows the escape flow field (SIM-FLOW-002). Commands are rejected (SIM-CMD-004). Routing soldiers do not attack and use the Routing defence multiplier. | REQ-MOR-004 |
| SIM-MOR-031 | Rally: a Routing regiment rallies when `M ≥ t_routing + morale.rally_margin` (default 15, i.e. 30) and no enemy within `morale.rally_safe_radius` (default 50 m) of its centroid. On rally it enters Shaken, halts, and reforms at its centroid facing the nearest enemy. `rout_count += 1`. | REQ-MOR-004 |
| SIM-MOR-032 | Shatter: a regiment becomes Shattered if `rout_count ≥ morale.max_routs` (default 2) when it would rout again, or if `count < initial × morale.shatter_strength` (default 0.25) when it routs, or if it routs while `M` is 0. Shattered regiments run to the edge and are removed; their soldiers count as fled. | REQ-MOR-005 |
| SIM-MOR-033 | Contagion: SIM-MOR-016 implements spreading; additionally, on the tick a regiment routs, allies within `morale.rout_shock_radius` (30 m) take `−morale.rout_shock` (5). | REQ-MOR-006 |
| SIM-MOR-034 | Pursuit: soldiers of non-routing regiments within reach of routing soldiers attack them with `hit probability × combat.pursuit_hit_mult` (default 1.5). Cavalry chasing routers move at `run`. | REQ-SIM-034 |

## 8. Fatigue

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FAT-001 | Soldier fatigue `F ∈ [0, 1]`, starts at `BattleSetup.roster[i].fatigue` (campaign may pass tired armies), default 0. | REQ-FAT-001 |
| SIM-FAT-002 | Per tick `F ← clamp(F + rate × unit.fatigue_rate_mult × zone.fatigue_mult × weather.fatigue_mult × dt_s, 0, 1)` with `rate` by activity from `fatigue.rate_<activity>` per second: idle `−0.010` (recovery), walk `0.004`, march `0.002`, run `0.020`, fighting `0.015`, routing `0.020`. Armour adds `fatigue.armour_rate × unit.armour` to all positive rates. | REQ-FAT-001 |
| SIM-FAT-003 | States by `fatigue.thresholds`: Fresh `F < 0.25`, Active `< 0.5`, Tired `< 0.75`, Exhausted otherwise. | REQ-FAT-002 |
| SIM-FAT-004 | Multipliers are continuous functions of `F`, not steps: `fatigue_speed_mult = 1 − fatigue.speed_loss × F` (0.3); `fatigue_attack_mult = 1 − fatigue.attack_loss × F` (0.3); `fatigue_defence_mult = 1 − fatigue.defence_loss × F` (0.2); `fatigue_interval_mult = 1 + fatigue.interval_gain × F` (0.4). States are for UI and morale only. | REQ-FAT-003 |
| SIM-FAT-005 | Regiment fatigue `F_mean` is the mean over living soldiers, recomputed every 10 ticks. | REQ-FAT-004 |

## 9. Generals and auras

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-GEN-001 | Each army's general is a soldier of category `general` inside a bodyguard regiment given in `BattleSetup`. The general soldier has `hp × general.hp_mult` (default 3) and its own `attack/defence` from its unit type. | REQ-CMBT-020 |
| SIM-GEN-002 | Aura: allied regiments whose anchor is within `general.aura_radius` (default 60 m) of the general receive the `general_aura` morale factor and combat `attack × (1 + general.aura_attack)` (default 0.05). The radius is modified by `general.aura_per_rank × general_rank`. | REQ-CMBT-021 |
| SIM-GEN-003 | On general death: SIM-MOR-014 shock, aura removed, `BattleResult.general_fate = Dead`. If the bodyguard regiment routs with the general alive, the general routes with it (aura suspended while Routing). | REQ-CMBT-022 |
| SIM-GEN-004 | Fate at battle end: `Dead` if hp ≤ 0; `Captured` if alive on a losing side and the bodyguard was Shattered; `Wounded` if hp < `general.wounded_hp` (0.3) fraction; else `Alive`. | REQ-CMBT-023 |
| SIM-GEN-005 | The general may be ordered like any regiment; a bodyguard regiment engaging in melee applies the general's aura to itself. | — |

## 10. Abilities and status effects

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-ABIL-001 | An ability is data: `id, name_key, targeting` (self | regiment_ally | regiment_enemy | point | area), `radius, range, cooldown_ticks, duration_ticks, energy_cost, effects: [Effect], requires_not_engaged, requires_not_moving`. | REQ-ABIL-001 |
| SIM-ABIL-002 | Effect kinds and fields: `buff/debuff { stat, mult or add }` where `stat ∈ {attack, defence, armour, damage, speed, attack_interval, morale_per_s, fatigue_rate, los_radius, accuracy}`; `damage { amount, armour_penetration, per_tick }`; `heal { amount, per_tick }`; `summon { unit_type, count, formation }`; `fear { }` (SIM-MOR-018); `area { child_effects, radius, duration }`; `teleport { max_distance }`. Antiquity content uses only buff/debuff; the others exist in the engine from Phase 5. Until then, content using them validates against the schema but is rejected at load with a diagnostic naming the unsupported effect kind (Modding SDK §4). | REQ-ABIL-002 |
| SIM-ABIL-003 | `UseAbility` validation: regiment owns the ability (unit type or general), cooldown 0, energy ≥ cost, target valid for targeting kind and within range from the anchor, conditions met. On success: cooldown set, energy deducted, effects applied to the target set (all soldiers of the regiment for stat effects; area collects regiments whose anchor is inside the radius). | REQ-ABIL-001 |
| SIM-ABIL-004 | Status effects on a regiment carry `source_ability, remaining_ticks, stacks`. Stacking rule per ability `stacking ∈ {refresh, stack(max), highest}`: refresh resets duration; stack increments up to max and multiplies additive parts; highest keeps the stronger of two. | REQ-ABIL-003 |
| SIM-ABIL-005 | Stat multipliers from all active status effects are multiplied together and applied as `status_mult` in the combat and movement formulas; additive parts are summed before the base multiplier. | — |
| SIM-ABIL-006 | Energy: `regiment.energy ∈ [0, unit.energy_max]` regenerates `unit.energy_regen` per second. Antiquity units have `energy_max = 0` and abilities with `energy_cost = 0`; the field exists so Phase 5 needs no schema change. | REQ-ABIL-004 |
| SIM-ABIL-007 | Ability effects are applied in ascending regiment id then ability list order. | REQ-SIM-007 |

## 11. Visibility

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-VIS-001 | Each regiment has `los_radius = unit.los_radius × zone.los_mult(anchor) × weather.los_mult × (1 + visibility.height_bonus × sat((h_anchor − h_mean_map) / combat.height_ref))`, default height bonus 0.5. | REQ-SIM-050 |
| SIM-VIS-002 | A point is visible to a regiment if within `los_radius` and the segment from the anchor (at eye height `visibility.eye_height`, 1.7 m) to the point (at 1.7 m) clears the heightmap sampled every `visibility.los_sample` (4 m) and is not blocked by a wall taller than the line at the crossing. | REQ-SIM-050 |
| SIM-VIS-003 | An enemy regiment is visible to a faction if its anchor or any of up to 4 sampled soldiers is visible to any of the faction's regiments; forests: a regiment whose anchor is in a `conceal` zone is visible only within `visibility.conceal_radius` (default 25 m) regardless of LOS. | REQ-SIM-051, REQ-SIM-052 |
| SIM-VIS-004 | Visibility is recomputed every `visibility.period_ticks` (default 10) per faction, staggered. Hidden regiments cannot be targeted by `AttackRegiment`, `UseAbility`, or ranged `target` mode; `fire_at_will` also ignores them. The AI queries only visible regiments. | REQ-SIM-051, REQ-SIM-053 |
| SIM-VIS-005 | Once a regiment has been seen its last known position is remembered for `visibility.memory_ticks` (default 400) for UI ghosting only; the sim does not use memory. | — |
| SIM-VIS-006 | During deployment, fog of war applies (blind deployment) unless `BattleSetup.reveal_deployment` is set (resolves OQ-6 with a data switch; default false). | REQ-SIM-030 |

## 12. Battle flow

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-FLOW-010 | Phases: `Deployment → Battle → Pursuit → Ended`. | REQ-SIM-030..035 |
| SIM-FLOW-011 | Deployment: each side's regiments are placed by `Deploy` commands inside its deployment zone (polygon in the map, chosen by `BattleSetup.side[i].deployment_zone`); AI deploys on tick 0 via SIM-AI-020. Regiments not deployed when a side confirms are auto-placed in a battle line at the zone centre. The phase ends when all human players have sent `ConfirmDeployment` or `battle_flow.deploy_timeout_ticks` (default 0 = none) expires. | REQ-SIM-030 |
| SIM-FLOW-012 | Battle: the timer starts at `BattleSetup.time_limit_ticks` (default 48,000 = 40 min). | REQ-SIM-032 |
| SIM-FLOW-013 | A side is *defeated* when it has no regiment in state Steady/Unsettled/Shaken/Broken on the field (all Routing, Shattered, withdrawn, or dead) and no pending reinforcements. When exactly one side is not defeated the phase becomes Pursuit; when all sides are defeated simultaneously or the timer expires, the phase becomes Ended with the winner decided by `battle_flow.timeout_winner` (`defender` default, or `most_soldiers`). | REQ-SIM-032 |
| SIM-FLOW-014 | Withdraw: regiments set to `Withdrawing`, move via the escape flow field at `march`, may be attacked normally; they do not count toward defeat until they leave the field. A withdrawn regiment's survivors count fully. A side whose all regiments are Withdrawing or gone is defeated. | REQ-SIM-033 |
| SIM-FLOW-015 | Pursuit: lasts `battle_flow.pursuit_ticks` (default 2,400) or until no routing soldiers remain on the field. Pursuers act per SIM-MOR-034. At the end, routing soldiers still on the field escape. | REQ-SIM-034 |
| SIM-FLOW-016 | Reinforcements: `BattleSetup.side[i].reinforcements: [{ arrival_tick, edge, regiments }]` spawn in a Column at the given edge midpoint, subject to SIM-CORE-006. | REQ-SIM-036 |
| SIM-FLOW-017 | `Surrender` by a player marks that side defeated immediately. | — |
| SIM-FLOW-018 | `BattleResult` = `{ winner, duration_ticks, per side: per regiment { id, initial, survivors, fled, killed, experience_gain, ammo_left }, general_fate, loot, summary }`. `experience_gain = floor(battle_flow.exp_per_kill × kills_by_regiment + battle_flow.exp_survive × survived)` (0.01, 1). Fled soldiers return to the campaign as survivors if their side won, or `battle_flow.fled_return_fraction` (0.5) of them if it lost. Loot = `battle_flow.loot_per_enemy_killed × enemy_dead` for the winner. | REQ-SIM-061 |
| SIM-FLOW-019 | `BattleSetup` = `{ map_id, seed, weather, time_of_day, time_limit_ticks, reveal_deployment, sides: [{ faction, player (human/ai id), deployment_zone, general: { unit_type, rank, name_key }, regiments: [{ id, unit_type, count, experience, fatigue, formation }], reinforcements }], victory: { timeout_winner } }`. Validation: cap, map exists, zones exist, unit types exist, each side has a general. | REQ-SIM-060 |

## 13. Battle AI

### 13.1 Framework

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-AI-001 | Utility AI: for a decision context, each candidate action is scored `score = base × Π_c curve_c(x_c)` over its considerations, where `x_c ∈ [0,1]` is a normalised input and `curve_c` is one of `linear(m, b)`, `quadratic(k)`, `logistic(k, mid)`, `step(threshold)`, defined in data (`content/ai/*.json5`). The highest score wins; ties by action list order. No randomness in selection; optional `noise` uses `rng.ai_*` deterministically. | REQ-AI-001 |
| SIM-AI-002 | Cadence: army AI every `ai.army_period_ticks` (default 40); regiment AI every `ai.regiment_period_ticks` (default 20), staggered by `regiment_id % period`. Decisions become Commands for the next tick (SIM-CMD-005). | REQ-AI-006 |
| SIM-AI-003 | The AI reads only what a player could see: visible enemy regiments (SIM-VIS-004), own state, terrain. | REQ-SIM-053 |

### 13.2 Army level

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-AI-010 | Army AI maintains a plan: `{ stance ∈ {attack, defend, hold, retreat}, main_line, reserves, flank_groups }`. Stance considerations: strength ratio (own vs visible enemy weighted by unit `cost`), morale mean, fatigue mean, terrain advantage (height of own line vs enemy), time remaining, `ai_profile.aggression`. | REQ-AI-003 |
| SIM-AI-011 | On `attack`: form a `battle_line` facing the enemy centroid at `ai.approach_distance` (150 m), then advance at `walk`, ranged forward until `ai.skirmish_range_frac` (0.9 × range) then behind; cavalry flank groups path to the enemy's nearer flank at `ai.flank_offset` (80 m) and charge rear-most visible regiments once the main lines are within `ai.charge_trigger_dist` (40 m). | REQ-AI-003 |
| SIM-AI-012 | On `defend`: choose the highest ground within `ai.defend_search_radius` (200 m) of the current centroid, form a line there, hold; ranged fire at will; cavalry counter-charge any enemy regiment that comes within `ai.counter_charge_dist` (30 m) of the line's flanks. | REQ-AI-003 |
| SIM-AI-013 | On `retreat`: `Withdraw` all regiments; cavalry screens (holds `ai.screen_offset` behind the line until the infantry is `ai.screen_gap` away). | REQ-AI-003 |
| SIM-AI-014 | Reserves: `ai_profile.reserve_fraction` (0.2) of infantry by cost held `ai.reserve_offset` (60 m) behind the line; committed to the segment with the lowest allied morale mean below `ai.commit_morale` (45) or where an enemy flank threatens. | REQ-AI-003 |

### 13.3 Regiment level

| Rule | Statement | Satisfies |
|---|---|---|
| SIM-AI-020 | Deployment: regiments are placed in a `battle_line` at the deployment zone's centre facing the enemy zone, infantry centre, ranged front, cavalry flanks, general behind centre; then `ConfirmDeployment`. | REQ-AI-003 |
| SIM-AI-021 | Regiment actions and key considerations: `engage_nearest` (distance, strength ratio, is flank), `hold_position` (in line, no threat), `fall_back` (morale < threshold, outnumbered), `use_ability` (per ability: condition curves, e.g. shield wall when enemy ranged within range), `switch_formation` (square if cavalry approaching within 60 m and no infantry threat; phalanx when engaged frontally; line default), `fire_mode` (hold when friendly in line of fire). | REQ-AI-003 |
| SIM-AI-022 | The general's bodyguard never engages unless `ai_profile.general_aggression` exceeds the strength-ratio consideration; it stays within aura range of the most regiments (position = centroid of regiments weighted by `1/morale`). | — |

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
| `stat_hit_base` | 0.6 (*chosen*) | SIM-PROJ-008 |
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
| `outnumber_ref` | 2 | SIM-MOR-020 |
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

The `ai.*` tunables (`army_period_ticks` 40, `regiment_period_ticks` 20 and the §13 distances) are not a rules file: they belong to the `AiProfile` content kind (T2-080).

### 15.2 Example unit types

| Field | `rome:hastati` | `rome:velites` | `greece:hoplite` | `persia:cavalry` | `persia:archer` (*chosen*, T2-030) |
|---|---|---|---|---|---|
| category | infantry | skirmisher | infantry | cavalry | ranged |
| soldier_radius / mass | 0.4 / 80 | 0.4 / 70 | 0.4 / 85 | 0.7 / 400 | 0.4 / 70 |
| hp | 100 | 80 | 110 | 160 | 80 |
| speed_walk / run / march | 1.6 / 4.0 / 1.6 | 1.8 / 4.5 / 1.8 | 1.4 / 3.6 / 1.4 | 3.0 / 9.0 / 3.0 | 1.6 / 4.0 / 1.6 |
| attack / defence / armour / damage | 35 / 30 / 8 / 30 | 25 / 20 / 2 / 25 | 32 / 38 / 10 / 30 | 38 / 25 / 8 / 35 | 20 / 18 / 2 / 20 |
| attack_interval_ticks / reach | 30 / 0.6 | 32 / 0.5 | 34 / 1.2 | 30 / 1.0 | 32 / 0.5 |
| charge_bonus / anti_cavalry_bonus | 0.3 / 0 | 0.1 / 0 | 0.15 / 0.5 | 0.8 / 0 | 0.05 / 0 |
| second_rank_attack / shield | false / true | false / false | true / true | false / false | false / false |
| frontal_arc_deg / armour_penetration | 120 / 0 | 120 / 0 | 120 / 0 | 120 / 0 | 120 / 0 |
| ranged | pilum: range 25, min 5, acc 0.6, speed 20, reload 120, ammo 2, dmg 40, pen 0.5, direct | javelin: range 40, min 5, acc 0.5, speed 20, reload 80, ammo 8, dmg 30, pen 0.3, direct | none | none | bow: range 120, min 15, acc 0.35, speed 40, reload 100, ammo 20, dmg 25, pen 0.2, indirect |
| morale_base / los_radius | 60 / 200 | 50 / 250 | 65 / 200 | 60 / 300 | 45 / 250 |
| formations | line, column, loose | loose, line, column | phalanx, line, column | wedge, line, column | loose, line, column |
| cost / upkeep / recruit_turns / regiment_size | 400 / 60 / 1 / 120 | 250 / 40 / 1 / 120 | 450 / 70 / 1 / 160 | 800 / 120 / 2 / 60 | 300 / 45 / 1 / 120 |

The archer is the flagship's only indirect-fire unit (SIM-PROJ-005's lob, SIM-PROJ-009's fire over friends); at 40 m/s a 45° launch reaches 163 m, so the speed cap never bites inside its 120 m range.

### 15.3 Scenario tests

Outcome bands over 50 seeds; a failing band means a formula or default needs review, not that the test is wrong. Each band is a file under `tests/scenarios/bands/`: an ordinary scenario (`BattleSetup` plus scripted `commands`) with a `bands` block giving the seed count and base, a tick limit and the assertions; `il_cli bands` runs them and `tests/tests/scenarios.rs` drives it nightly (TDD §17, T2-110). Every seed stops at the tick limit or when a side has no living soldiers. Assertion kinds: `winner` (the side annihilates every other side or ends with the strictly higher surviving fraction), `casualties` (fraction lost at the end or a number of ticks after a regiment's first contact), `routed_before_loss` and `rout_within` (read the Routing morale state). An assertion holds when its per-seed boolean is true in the required fraction of seeds. Rout clauses are carried in the files with `active: false` until morale lands (T2-041); until then the melee clause alone decides the band. The melee files script `FireMode: Hold` at tick 1 for every regiment that carries pila or javelins (rows 1, 2 and 4) so they stay melee-only now that throwing exists (T2-030).

| Scenario (file) | Melee clause (active) | Morale clause (T2-041) |
|---|---|---|
| 120 hastati (line) vs 120 velites (loose), melee only, flat (`melee_hastati_vs_velites`) | Hastati win 90–100 % of seeds (50/50 on 2026-09-04, mean 111 hastati and 6 velites left). | Velites rout before losing 50 %, in ≥ 90 % of seeds. |
| 160 hoplites (phalanx) vs 160 hastati frontal (`melee_hoplites_vs_hastati`) | Hoplites win ≥ 70 % (melee-only fights run to annihilation and are decisive: 50/50 on 2026-09-04, so the 90 % ceiling is a morale-era clause). | Hoplites win 70–90 % once routing exists (T2-041). |
| 160 hoplites vs 60 Persian cavalry frontal charge (`melee_hoplites_vs_cavalry`) | Hoplites win 85–100 %; cavalry has lost ≥ 30 % sixty seconds after its first contact, in ≥ 85 % of seeds. Measured 2026-09-04 over 50 seeds with the §15.1 defaults: about 20 % lost at 30 s (the wedge tip fights first, the bulk arrives later, and a hoplite attack lands every 34 ticks), 30 % by 60 s in every seed, the whole regiment by tick ≈ 4100; "on the charge" is therefore read as the first minute, a tuning candidate once morale (T2-041) can rout the cavalry instead. | — |
| 60 Persian cavalry rear-charge 120 engaged hastati (`melee_cavalry_rear_charge`) | Interim: the charged side loses (side 1 wins) in ≥ 80 % of seeds (50/50 on 2026-09-04). | Hastati rout within 30 s of the charge in ≥ 80 % of seeds. |
| 120 velites (loose) fire 8 volleys at 120 hastati (line) at 35 m, no approach (`volley_velites_vs_hastati`) | Hastati lose 15–35 soldiers (`casualties` 0.125–0.292 of 120 in ≥ 90 % of seeds). Measured 2026-09-04 over 50 seeds after tuning `scatter_scale` to 0.17: 50/50, mean 27 lost (the loose line's wings stand beyond 40 m, so about 107 of the 120 velites throw each volley; every javelin lands, roughly half on a soldier). | — |
| Statistical vs simulated projectile path, same volley | Mean casualties differ by ≤ 10 % (T2-032). | — |
| General killed at tick 600 in an even hastati vs hoplite fight | Side without general routs first in ≥ 75 % of seeds (T2-043). | — |
| Determinism | Every scenario above: identical hash on run 1 and run 2, and after snapshot/restore at tick 1,000 (T2-112 enrols the band files in the determinism test). | — |
