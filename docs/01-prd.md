# Iron Legion Engine — Product Requirements Document

| | |
|---|---|
| **Version** | 0.2 |
| **Status** | Draft for review |
| **Author** | Daniel Yehezkel |
| **Supersedes** | `iron_legion_engine_prd.tex` (v0.1, archived) |
| **Glossary** | [00-glossary.md](00-glossary.md) |
| **Downstream** | [SAD](02-sad.md) · [Simulation Spec](03-simulation-spec.md) · [TDD](04-tdd.md) · [Networking Spec](05-networking-spec.md) · [Modding SDK](06-modding-sdk-spec.md) |

## Change log vs v0.1

- Converted to Markdown; every requirement now has an ID, a MoSCoW priority, and a phase tag.
- Resolved the contradictory performance tiers into one ladder (§6) and separated simulation tick rate from render frame rate.
- Stated the campaign time model: turn-based (§9).
- Stated the presentation model: isometric fixed-angle 2.5D; "smooth rotation" replaced by snap rotation (§14).
- Defined the engine versus flagship game boundary (§3).
- Clarified the technology stack: `bevy_ecs` crate only, custom wgpu renderer, JSON5 content, Lua 5.4 via mlua (§7).
- Added sections: platforms, battle flow, terrain, visibility, commanders, campaign↔battle interface, UI, input, audio, localisation, tooling, testing, assumptions, risks, open questions.
- Roadmap rewritten with Phase 0 (foundations) and measurable exit criteria; data framework moved from Phase 6 to Phase 1.

## How to read this document

Requirements are written as tables:

| Column | Meaning |
|---|---|
| **ID** | `REQ-<AREA>-nnn`. Stable forever; retired IDs are marked *retired*, never reused. |
| **Priority** | **M**ust (MVP cannot ship without it), **S**hould (planned, MVP degrades without it), **C**ould (desirable, first to cut), **W**on't (explicitly out of scope for now). |
| **Phase** | Roadmap phase in which the requirement is first satisfied (§20). `—` for Won't. |

Downstream documents cite these IDs. The traceability matrix lives in [README.md](README.md).

---

## 1. Vision

Iron Legion Engine is a specialised 2D strategy game engine for one family of games: turn-based grand campaign plus real-time tactical battles with massive armies of individually simulated soldiers, in historical or fantasy settings, built to be modded.

> **Design goal.** Enable battles of tens of thousands of individual soldiers while keeping strategic depth, moddability, and a deterministic simulation that future lockstep multiplayer can be built on.

The engine is not, and will not become, a general-purpose engine.

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-VIS-001 | The engine shall support games consisting of a turn-based campaign layer and a real-time battle layer exchanging data through a defined interface. | M | 4 |
| REQ-VIS-002 | Every soldier in a battle shall be an individually simulated entity with its own position, health, and fatigue. | M | 1 |
| REQ-VIS-003 | Decision-making shall live at Army and Regiment level; soldiers execute orders and hold formation. | M | 1 |
| REQ-VIS-004 | All game content shall be defined in external data, loadable without engine code changes. | M | 1 |
| REQ-VIS-005 | The battle and campaign simulations shall be deterministic (see §8). | M | 0 |
| REQ-VIS-006 | Gameplay quality and simulation scale take priority over graphical fidelity when they conflict. | M | — |

## 2. Non-goals

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-VIS-010 | The engine will not support first-person or third-person character gameplay. | W | — |
| REQ-VIS-011 | The engine will not support 3D environments; the world is 2D with an isometric presentation and a heightmap. | W | — |
| REQ-VIS-012 | The engine will not provide a physics sandbox (rigid bodies, ragdolls, destructible physics). | W | — |
| REQ-VIS-013 | The engine will not target general-purpose game development or genres outside strategy. | W | — |
| REQ-VIS-014 | The engine will not provide character-centric RPG systems (inventories, dialogue trees, character progression beyond regiment experience). | W | — |
| REQ-VIS-015 | The engine will not provide a real-time (continuous clock) campaign layer. | W | — |

## 3. Engine and flagship game boundary

The engine and the flagship game share one repository and one build today. The boundary is enforced by folder and dependency rule so that a later extraction is mechanical.

| Belongs to the **Engine** (`crates/il_*`) | Belongs to the **Game** (`game/`) |
|---|---|
| Formation, movement, combat, morale, fatigue, ability *systems* and their tunable parameters as data fields | The antiquity factions, unit types, formation templates, technologies, buildings, and the numbers that fill those fields |
| Campaign turn engine, province graph, economy, diplomacy, research, recruitment *mechanics* | The antiquity campaign map, provinces, starting positions, diplomatic personalities |
| Utility-AI framework, considerations vocabulary | Consideration weights and AI personalities |
| Data loader, mod system, editors, scripting host | Lua scripts for the flagship campaign events and missions |
| Renderer, UI framework, audio engine | Sprites, sounds, music, UI skin |
| Generic effect layer for abilities (buff, debuff, damage, heal, summon, fear, area, teleport) | Which factions get which abilities; the fantasy layer when it arrives |

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-VIS-020 | Engine crates shall not depend on anything under `game/`. | M | 0 |
| REQ-VIS-021 | The flagship game shall be packaged as a mod (manifest, JSON5 content, Lua) loaded through the same path as third-party mods. | M | 1 |
| REQ-VIS-022 | Game-specific *rules* that cannot be expressed as data shall live in `game/` Rust code behind engine-defined traits, and each such case shall be logged as an open question for later generalisation. | S | 1 |
| REQ-VIS-023 | The flagship setting is historical antiquity (Rome, Greece, Persia). Engine documentation and default content use this setting for examples. | M | 1 |

## 4. Target users

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-VIS-030 | Initial user: the engine author building the flagship game. | M | 0 |
| REQ-VIS-031 | Future users: indie strategy developers, historical strategy enthusiasts, mod creators without programming knowledge. | S | 6 |
| REQ-VIS-032 | Documentation shall be sufficient for a competent Rust developer to contribute to any subsystem without talking to the author. | S | 0 |

## 5. Platforms

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-PLAT-001 | Windows 10/11 x86-64 is the only supported platform for MVP. | M | 0 |
| REQ-PLAT-002 | Linux x86-64 and macOS (Apple silicon) shall be supported after MVP. Code shall not introduce Windows-only dependencies without a documented abstraction. | S | 7 |
| REQ-PLAT-003 | Determinism (§8) shall hold across all machines running the same build on the same OS for MVP. | M | 0 |
| REQ-PLAT-004 | Determinism across operating systems and CPU vendors shall be achievable by swapping the scalar implementation to fixed-point without touching gameplay code. | S | 7 |
| REQ-PLAT-005 | Steam Deck and controller-first input are out of scope. | W | — |

## 6. Performance

Simulation tick rate and render frame rate are independent. The simulation runs at a fixed 20 Hz (50 ms per tick). Frame rate targets below are for the renderer with interpolation and assume the sim tick fits its budget.

### 6.1 Performance ladder

| Tier | Soldiers on field | Render FPS | Sim tick budget | Phase |
|---|---|---|---|---|
| P1 | 2,000 | 60 | ≤ 10 ms | 1 |
| P2 | 10,000 | 60 | ≤ 25 ms | 2 |
| P3 | 20,000 | 30 minimum, 60 target | ≤ 50 ms (one full tick period) | 3 |
| Cap | 32,768 | best effort, no crash | — | 3 |

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-PERF-001 | The engine shall meet tier P1 on the target hardware (§6.2) with all Phase 1 systems active. | M | 1 |
| REQ-PERF-002 | The engine shall meet tier P2 with melee, ranged, morale, fatigue, and routing active. | M | 2 |
| REQ-PERF-003 | The engine shall meet tier P3 with all battle systems active. | S | 3 |
| REQ-PERF-004 | The engine shall enforce a hard cap of 32,768 soldier entities per battle and degrade gracefully (refuse reinforcements, log) rather than crash beyond it. | M | 1 |
| REQ-PERF-005 | Each simulation system shall have a documented per-tick time budget at 20,000 soldiers (owned by the TDD), and the benchmark suite shall track them. | S | 2 |
| REQ-PERF-006 | Battle load time from campaign to deployment phase shall be under 10 seconds at P2 scale. | S | 4 |
| REQ-PERF-007 | Peak battle memory shall stay under 4 GB at P3 scale. | S | 3 |
| REQ-PERF-008 | Live projectile count shall be capped (default 8,192) with a documented pooling and culling policy. | M | 2 |
| REQ-PERF-009 | The campaign turn for 30 AI factions shall resolve in under 5 seconds. | S | 4 |

### 6.2 Target hardware

| Component | Target |
|---|---|
| CPU | Modern 8-core desktop processor (2020 or newer) |
| RAM | 16 GB minimum, 32 GB recommended |
| GPU | Mid-range discrete GPU with Vulkan or DirectX 12 support |
| Storage | SSD |

## 7. Technology stack

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-TECH-001 | The engine shall be written in Rust (stable toolchain, MSRV pinned in the workspace). | M | 0 |
| REQ-TECH-002 | The ECS shall be the `bevy_ecs` crate used standalone. The Bevy app, renderer, asset, and input crates shall not be used. | M | 0 |
| REQ-TECH-003 | Rendering shall use `wgpu` directly with a custom renderer. Windowing and input events via `winit`. | M | 1 |
| REQ-TECH-004 | UI shall use `egui` rendered through `egui-wgpu`. | M | 1 |
| REQ-TECH-005 | Content shall be JSON5 (comments, trailing commas, unquoted keys) parsed via Serde. | M | 1 |
| REQ-TECH-006 | Saves shall use a compact binary Serde format (postcard or bincode, decided in the TDD) with a JSON header. | M | 4 |
| REQ-TECH-007 | Scripting shall be Lua 5.4 via `mlua`, sandboxed (§18). | M | 6 |
| REQ-TECH-008 | Simulation crates shall have no dependency on rendering, windowing, UI, audio, or wall-clock APIs. | M | 0 |
| REQ-TECH-009 | All simulation arithmetic shall go through a `Scalar` trait abstraction; `f32` is the MVP implementation. | M | 0 |
| REQ-TECH-010 | The sim crates shall be compiled without fast-math and without target-specific intrinsics that could alter floating-point results. | M | 0 |
| REQ-TECH-011 | RON is not used. | W | — |

## 8. Determinism

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-SIM-001 | Given identical BattleSetup, command stream, and seed, the battle simulation shall produce identical state hashes at every tick. | M | 0 |
| REQ-SIM-002 | Given identical campaign start state, command stream, and seed, the campaign simulation shall produce identical state hashes at every turn. | M | 4 |
| REQ-SIM-003 | The simulation shall be driven only by Commands; no external code may mutate simulation state directly. | M | 0 |
| REQ-SIM-004 | Randomness shall come only from seeded RNG streams owned by the simulation, one stream per system. | M | 0 |
| REQ-SIM-005 | The simulation shall compute a 64-bit state hash at the end of every tick over a documented, ordered set of components. | M | 0 |
| REQ-SIM-006 | The simulation shall serialise and restore a full snapshot such that resuming from a snapshot yields the same hashes as the uninterrupted run. | M | 0 |
| REQ-SIM-007 | Iteration order over entities in any system whose output depends on order shall be defined (sorted by stable entity id), never by ECS storage order. | M | 0 |
| REQ-SIM-008 | Parallel execution inside the simulation is permitted only where the result is provably order-independent, or where results are gathered and applied in stable id order. | M | 0 |
| REQ-SIM-009 | The battle simulation shall run headless (no window, no GPU) for tests, benchmarks, and future servers. | M | 0 |

## 9. Campaign time model

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-CAMP-001 | The campaign shall be turn-based. Each turn, factions act in a fixed order; the human player's faction acts first. | M | 4 |
| REQ-CAMP-002 | A turn shall have phases: start-of-turn events, player actions, AI actions, end-of-turn resolution (economy, research, recruitment, movement, battles). | M | 4 |
| REQ-CAMP-003 | Battles triggered during a turn shall pause the campaign, run to a BattleResult, and resume the turn. | M | 4 |
| REQ-CAMP-004 | A turn represents one season; four turns per year. Configurable in data. | S | 4 |

## 10. Battle simulation

### 10.1 World model

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-SIM-020 | Battlefields shall use a continuous 2D coordinate system in world units, with a heightmap for elevation. | M | 1 |
| REQ-SIM-021 | The simulation tick shall be fixed at 20 Hz. | M | 0 |
| REQ-SIM-022 | The entity hierarchy shall be Faction → Army → Regiment → Soldier, plus Projectiles. | M | 1 |
| REQ-SIM-023 | Regiment sizes, army sizes, and battle sizes shall be defined by data, bounded only by the entity cap (REQ-PERF-004). | M | 1 |
| REQ-SIM-024 | Every soldier shall have a collision circle and shall be pushed apart from overlapping soldiers deterministically. | M | 1 |
| REQ-SIM-025 | Soldiers shall never run pathfinding individually; they steer toward assigned slots or follow flow fields (§13). | M | 1 |

### 10.2 Battle flow

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-SIM-030 | A battle shall begin with a deployment phase in which each side places regiments inside its deployment zone; the phase ends when all human players confirm and AI has deployed. | M | 2 |
| REQ-SIM-031 | The battle phase shall support pause and speed multipliers (0.5×, 1×, 2×, 4×) as Commands in the command stream. | M | 1 |
| REQ-SIM-032 | A battle shall end when one side has no non-routing regiments on the field, when one side has fully withdrawn, or when the battle timer (data-defined, default 40 minutes of sim time) expires. | M | 2 |
| REQ-SIM-033 | A player shall be able to order a withdraw; withdrawing regiments move to their own map edge and leave the field without routing penalties. | S | 2 |
| REQ-SIM-034 | After one side has fully routed or withdrawn, a pursuit phase shall run in which pursuers inflict casualties on routing regiments until they exit the map or the pursuit timer expires. | S | 2 |
| REQ-SIM-035 | The battle shall produce a BattleResult (§11) at the end of every battle, including aborted ones. | M | 2 |
| REQ-SIM-036 | Reinforcement groups defined in BattleSetup shall enter from specified map edges at specified ticks. | C | 4 |

### 10.3 Terrain

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-SIM-040 | Elevation shall modify movement speed (uphill slower), ranged range and accuracy (height advantage), and morale (holding high ground). | M | 2 |
| REQ-SIM-041 | Terrain zones shall have data-defined types (open, forest, marsh, rock, road) with movement, visibility, and formation modifiers. Rock is impassable. | M | 1 |
| REQ-SIM-042 | Rivers shall be impassable except at fords (slow, exposed) and bridges (narrow passages). | S | 3 |
| REQ-SIM-043 | Settlements with walls, gates, and towers shall be supported; walls block movement and line of sight; gates open, close, and can be destroyed. | C | 5 |
| REQ-SIM-044 | Siege equipment (ladders, rams, towers) shall be supported as special regiments with interaction rules. | C | 5 |
| REQ-SIM-045 | The battle map format shall reserve fields for walls, gates, and siege attachment points from Phase 1 so siege maps do not require a format migration. | M | 1 |
| REQ-SIM-046 | Weather (clear, rain, fog) shall modify visibility, ranged accuracy, and fatigue accumulation. | C | 4 |

### 10.4 Visibility

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-SIM-050 | Each regiment shall have a line-of-sight radius modified by elevation, terrain, and weather. | S | 2 |
| REQ-SIM-051 | Fog of war shall hide enemy regiments outside all allied regiments' line of sight; hidden regiments are not selectable or targetable. | S | 2 |
| REQ-SIM-052 | Forests shall conceal regiments inside them until an enemy is within a data-defined short range. | S | 2 |
| REQ-SIM-053 | Battle AI shall obey the same fog of war rules as the player. | S | 2 |

## 11. Campaign ↔ battle interface

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-SIM-060 | A battle shall be fully specified by a serialisable BattleSetup: map id, seed, weather, time of day, per-side rosters (regiment type, count, experience, general), deployment zones, reinforcement groups, victory conditions. | M | 2 |
| REQ-SIM-061 | A battle shall return a serialisable BattleResult: winner, per-regiment survivors and experience gained, general fate (alive, wounded, dead, captured), loot, duration, and a summary for the UI. | M | 2 |
| REQ-SIM-062 | The campaign shall apply BattleResult to its state without any other data path from the battle. | M | 4 |
| REQ-SIM-063 | Battles shall be launchable from a standalone scenario file (a BattleSetup on disk) without a campaign, for testing and custom battles. | M | 2 |
| REQ-SIM-064 | Auto-resolve shall exist for the campaign. Its model is an open question (OQ-3). | S | 4 |

## 12. Formations

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-FORM-001 | A formation shall be a set of slots generated by a layout function from a data-defined template (ranks, files, spacing, facing). | M | 1 |
| REQ-FORM-002 | Built-in templates: Line, Column, Square, Wedge, Phalanx, Loose. Custom templates shall be definable in data by parameterising built-in layout functions. | M | 1 |
| REQ-FORM-003 | Formations shall resize when soldiers die, closing ranks from the rear to preserve frontage. | M | 1 |
| REQ-FORM-004 | Reform shall assign soldiers to slots minimising travel cost with a bounded algorithm suitable for 500 soldiers per tick budget. | M | 1 |
| REQ-FORM-005 | Regiments shall rotate (wheel) and change facing while holding formation. | M | 1 |
| REQ-FORM-006 | Formation integrity shall be measured continuously and shall modify combat effectiveness and morale. | M | 2 |
| REQ-FORM-007 | Formation morphing between templates shall be supported as an order, with a transition period during which integrity is reduced. | S | 2 |
| REQ-FORM-008 | Mixed regiments (multiple unit types with role zones) shall be supported. | S | 3 |
| REQ-FORM-009 | Army-level group formations (battle line, echelon, refused flank) shall arrange multiple regiments and shall be usable by the player via drag-formation and by the AI. | S | 3 |
| REQ-FORM-010 | Formation depth and width shall be adjustable by the player with a drag gesture (§15). | M | 1 |

## 13. Movement and pathfinding

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-PATH-001 | Regiments shall path on a coarse nav grid using hierarchical A* (clusters and gates). | M | 3 |
| REQ-PATH-002 | Until Phase 3, regiments may path with plain A* on the nav grid, behind the same interface. | M | 1 |
| REQ-PATH-003 | Soldiers shall steer toward slots with seek, separation, and obstacle avoidance behaviours. | M | 1 |
| REQ-PATH-004 | Routing soldiers shall follow a precomputed escape flow field toward their own map edge. | M | 2 |
| REQ-PATH-005 | Regiments shall have walk, run (charge), and march speeds, modified by terrain, slope, fatigue, and formation type. | M | 1 |
| REQ-PATH-006 | Regiments shall follow paths as formations (front rank leads, wheeling at waypoints), not as blobs. | M | 1 |
| REQ-PATH-007 | The nav grid shall update when gates open or close or walls are destroyed, and the hierarchical graph shall repair locally. | C | 5 |
| REQ-PATH-008 | Campaign pathfinding shall run on the province graph with per-edge costs (terrain, roads, rivers) and movement points per turn. | M | 4 |
| REQ-PATH-009 | A uniform spatial grid shall serve all neighbour queries (targeting, collision, visibility). A quadtree is a possible future replacement behind the same interface. | M | 1 |

## 14. Combat

### 14.1 Melee

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-CMBT-001 | Soldiers shall have hit points; melee attacks shall resolve as a timed attack cycle, a hit roll of attack skill against defence, and armour subtracting from damage. | M | 2 |
| REQ-CMBT-002 | Melee attributes per unit type: attack, defence, armour, damage, attack interval, reach, charge bonus, mass. | M | 2 |
| REQ-CMBT-003 | Only soldiers in contact fight; rear ranks push forward into vacated slots. Spear and pike units shall be able to attack from the second rank (data flag). | M | 2 |
| REQ-CMBT-004 | Flank and rear attacks shall multiply damage and morale damage. | M | 2 |
| REQ-CMBT-005 | Charges shall grant a temporary attack bonus and shall push defenders based on mass difference. | M | 2 |
| REQ-CMBT-006 | Anti-cavalry units shall have a data-defined bonus versus cavalry, negating charge bonus when braced in formation. | S | 2 |
| REQ-CMBT-007 | Fatigue and morale state shall modify attack, defence, and attack interval. | M | 2 |

### 14.2 Ranged

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-CMBT-010 | Every projectile shall be a simulated entity with a ballistic arc and a hit check against soldier collision circles at its landing point. | M | 2 |
| REQ-CMBT-011 | Ranged attributes: range, minimum range, accuracy, projectile speed, reload ticks, ammunition, damage, armour penetration, arc type (direct, indirect). | M | 2 |
| REQ-CMBT-012 | Friendly fire shall occur when projectiles land on allies. | M | 2 |
| REQ-CMBT-013 | Ranged regiments shall support fire-at-will, hold fire, and target-regiment orders; indirect-fire units may fire over friendly regiments. | M | 2 |
| REQ-CMBT-014 | Elevation shall extend range downhill and reduce it uphill. | S | 2 |
| REQ-CMBT-015 | Live projectiles shall be pooled and capped (REQ-PERF-008). When capped, further volleys are resolved statistically without visible projectiles, deterministically. | M | 2 |

### 14.3 Abilities

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-ABIL-001 | Abilities shall be data-defined with cooldown, duration, energy cost, targeting (self, regiment, area, point), and a list of effects. | M | 2 |
| REQ-ABIL-002 | Effect types: buff, debuff, damage, heal, summon, fear, area, teleport. Buff and debuff are required for MVP (e.g. shield wall, testudo, war cry); the rest are Phase 5. | M / C | 2 / 5 |
| REQ-ABIL-003 | Status effects shall stack according to data-defined rules (refresh, stack count, highest wins). | S | 2 |
| REQ-ABIL-004 | A magic and energy resource system shall be a Phase 5 layer on top of abilities, defined entirely in data. | C | 5 |

## 15. Morale

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-MOR-001 | Morale shall be a regiment-level value in [0, 100]. | M | 2 |
| REQ-MOR-002 | Morale states: Steady, Unsettled, Shaken, Broken, Routing, with hysteresis thresholds defined in data. | M | 2 |
| REQ-MOR-003 | Morale factors, each with a data-defined weight: casualties (rate and total), fatigue, general aura and general death, nearby allies and nearby routing allies, terrain (high ground), fear effects, being flanked or surrounded, being outnumbered, formation integrity, engagement duration. | M | 2 |
| REQ-MOR-004 | A Routing regiment shall ignore orders and flee via the escape flow field; it may rally when morale recovers and no enemy is within a data-defined distance. | M | 2 |
| REQ-MOR-005 | A regiment that routs a data-defined number of times, or is reduced below a data-defined strength, becomes Shattered and leaves the battle. | M | 2 |
| REQ-MOR-006 | Routing shall spread: a routing regiment applies a morale penalty to allies within a data-defined radius. | M | 2 |

## 16. Fatigue

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-FAT-001 | Fatigue shall be a soldier-level value accumulated per tick by activity (idle, walk, run, fight) and terrain, and recovered when idle. | M | 2 |
| REQ-FAT-002 | Fatigue states: Fresh, Active, Tired, Exhausted, mapped from the value by data-defined thresholds. | M | 2 |
| REQ-FAT-003 | Fatigue state shall modify movement speed, attack interval, attack, defence, and morale. | M | 2 |
| REQ-FAT-004 | Regiment fatigue displayed in UI shall be the mean of its soldiers. | M | 2 |

## 17. Commanders

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-CMBT-020 | Each army shall have one general, a special soldier in a bodyguard regiment. | M | 2 |
| REQ-CMBT-021 | The general shall project a leadership aura (radius and bonuses in data) affecting allied regiments' morale and combat. | M | 2 |
| REQ-CMBT-022 | General death shall apply an immediate army-wide morale shock and remove the aura. | M | 2 |
| REQ-CMBT-023 | General fate (alive, wounded, dead, captured) shall be reported in BattleResult and applied in the campaign. | M | 4 |
| REQ-CMBT-024 | Named heroes with abilities are a Phase 5 extension of the general mechanism. | C | 5 |

## 18. Campaign layer

### 18.1 World

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-CAMP-010 | The campaign map shall consist of provinces defined as polygons with an adjacency graph, each with one owner, terrain type, resources, and one settlement. | M | 4 |
| REQ-CAMP-011 | Armies shall occupy provinces and move along the province graph using movement points. | M | 4 |
| REQ-CAMP-012 | Moving into a province containing a hostile army shall trigger a battle (interception). | M | 4 |
| REQ-CAMP-013 | Sieging a settlement shall be a multi-turn state that triggers a siege battle on assault (Phase 5 battle support). | C | 5 |
| REQ-CAMP-014 | Agents (diplomats, spies) are post-MVP. | C | 5 |

### 18.2 Economy

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-CAMP-020 | Each faction shall have a treasury; income from taxation and resource production, expenses from army upkeep and building maintenance, resolved each turn. | M | 4 |
| REQ-CAMP-021 | Trade shall generate income along trade routes between friendly factions. | S | 4 |
| REQ-CAMP-022 | Buildings shall be data-defined with cost, build time, prerequisites, and effects on province and recruitment. | M | 4 |

### 18.3 Diplomacy

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-CAMP-030 | Diplomatic actions: declare war, peace treaty, trade agreement, alliance. | M | 4 |
| REQ-CAMP-031 | Additional actions: vassalage, coalition against a dominant faction. | C | 5 |
| REQ-CAMP-032 | Each faction pair shall have an attitude value driven by data-defined factors (borders, wars, treaties, strength, personality). | M | 4 |

### 18.4 Research and recruitment

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-CAMP-040 | Technologies shall be data-defined with prerequisites, cost in turns, and effects (unit unlocks, modifiers). Categories are data (military, economic, political for antiquity; magic for Phase 5). | M | 4 |
| REQ-CAMP-041 | Recruitment shall depend on settlement buildings and faction; unit types have recruitment cost, upkeep, turns to recruit, and tier. | M | 4 |
| REQ-CAMP-042 | Regiments shall carry experience gained in battle across the campaign, modifying stats via data-defined tiers. | S | 4 |
| REQ-CAMP-043 | Regiments shall replenish losses over turns in friendly provinces. | S | 4 |

## 19. Artificial intelligence

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-AI-001 | Battle and campaign AI shall use a utility-AI framework: actions scored from data-defined considerations and weights. | M | 2 |
| REQ-AI-002 | Soldier behaviour shall be a small finite state machine (idle, move to slot, fight, rout, dead). | M | 1 |
| REQ-AI-003 | Battle AI shall deploy, form a battle line, engage, flank, use abilities, and retreat, respecting fog of war. | M | 2 |
| REQ-AI-004 | Campaign AI shall manage expansion, diplomacy, economy, recruitment, and army movement with a data-defined personality. | M | 4 |
| REQ-AI-005 | AI decisions shall be issued as Commands and shall be deterministic. | M | 2 |
| REQ-AI-006 | AI shall run within the sim tick budget by decision cadence (regiment AI every N ticks, staggered). | M | 2 |
| REQ-AI-007 | Consideration weights and AI personalities shall be moddable in JSON5. | S | 6 |

## 20. Rendering

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-RNDR-001 | The presentation shall be isometric 2.5D at a fixed pitch. Camera rotation shall snap to 4 directions (8 as Could). | M | 1 |
| REQ-RNDR-002 | Soldiers shall be rendered as instanced sprites from texture atlases with 8 facing directions per animation. | M | 1 |
| REQ-RNDR-003 | Rendering shall interpolate positions and facings between the last two simulation ticks. | M | 1 |
| REQ-RNDR-004 | Level of detail by zoom: Detailed Soldier, Reduced Soldier, Sprite Aggregation (one sprite per block of soldiers). | M | 3 |
| REQ-RNDR-005 | Camera: strategic zoom (whole battlefield), tactical zoom (individual soldiers), smooth pan and zoom, edge scrolling. | M | 1 |
| REQ-RNDR-006 | Terrain shall be rendered from the heightmap and zone data with tile textures and elevation shading. | M | 1 |
| REQ-RNDR-007 | The renderer shall run on its own thread or otherwise not block the simulation step. | S | 3 |
| REQ-RNDR-008 | Debug overlays: nav grid, formation slots, paths, morale, spatial grid, LOS. Toggleable at runtime in dev builds. | M | 1 |
| REQ-RNDR-009 | Projectiles, blood, and death animations shall be rendered with instancing and culled aggressively at far zoom. | S | 2 |

## 21. User interface

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-UI-001 | Battle UI: regiment cards for the player's army, command card for selected regiments, minimap with fog of war, battle clock and speed controls, morale and fatigue indicators, casualties summary. | M | 2 |
| REQ-UI-002 | Campaign UI: province panel, settlement and buildings panel, army and recruitment panel, diplomacy screen, research tree, faction overview, end-turn button and turn log. | M | 4 |
| REQ-UI-003 | Deployment UI: drag regiments into the deployment zone, group formation presets, start battle. | M | 2 |
| REQ-UI-004 | Battle result screen and campaign event popups. | M | 4 |
| REQ-UI-005 | All UI shall be data-driven enough that layout tweaks and string changes do not require engine changes. | S | 6 |
| REQ-UI-006 | The UI shall be usable at 1080p and 1440p with scalable text. | M | 2 |
| REQ-UI-007 | Main menu, custom battle setup (choose map, sides, rosters), settings, load/save screens. | M | 2 |

## 22. Input

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-INP-001 | Mouse and keyboard are the primary input devices. | M | 1 |
| REQ-INP-002 | Selection: click, shift-click, drag-select box, double-click selects unit type, control groups (Ctrl+1..9). | M | 1 |
| REQ-INP-003 | Orders: right-click move, right-drag to set formation width and facing (drag-formation), attack-move, halt, run toggle, formation template hotkeys, ability hotkeys, withdraw. | M | 1 |
| REQ-INP-004 | Camera: WASD/arrow pan, edge scroll, wheel zoom, Q/E snap rotate, middle-drag pan. | M | 1 |
| REQ-INP-005 | All bindings shall be rebindable via a data file and settings UI. | S | 2 |
| REQ-INP-006 | Every input shall be translated into a Command before reaching the simulation; the UI layer never mutates sim state. | M | 1 |

## 23. Audio

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-AUD-001 | Audio shall be driven by simulation events (impacts, volleys, charges, routs) through an event bus; the sim never calls audio directly. | M | 2 |
| REQ-AUD-002 | Battle ambience shall mix by zoom level (crowd roar at far zoom, individual clashes at near zoom) and by battle intensity. | S | 3 |
| REQ-AUD-003 | Unit response voice lines on selection and orders, data-defined per faction. | C | 6 |
| REQ-AUD-004 | Music with campaign and battle playlists and state-based transitions. | S | 4 |
| REQ-AUD-005 | Audio assets shall be moddable through the same content system. | S | 6 |

## 24. Localisation

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-LOC-001 | All user-visible strings shall be referenced by key and resolved from locale files; no literal UI strings in code. | M | 1 |
| REQ-LOC-002 | Mods shall be able to add or override strings and add new languages. | S | 6 |
| REQ-LOC-003 | English is the only shipped language for MVP. | M | 1 |
| REQ-LOC-004 | Text rendering shall support Latin, Cyrillic, and Greek scripts; right-to-left and CJK are out of scope for MVP. | S | 6 |

## 25. Modding

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-MOD-001 | Tier 1: everything in Content (units, factions, formations, technologies, buildings, abilities, maps, strings, AI weights, audio references) shall be definable and overridable in JSON5 without code. | M | 1 |
| REQ-MOD-002 | Tier 2: campaign events, missions, quests, triggers, and scenario logic shall be scriptable in sandboxed Lua 5.4. | M | 6 |
| REQ-MOD-003 | Lua shall not execute inside the battle tick for MVP. Battle behaviour is data plus Rust. | M | 6 |
| REQ-MOD-004 | Each mod shall have a manifest with id, version, dependencies, engine version range, and load order hints. | M | 1 |
| REQ-MOD-005 | Load order shall be resolved from dependencies; later mods override earlier by Content ID with explicit replace, merge, and list-operation semantics. | M | 1 |
| REQ-MOD-006 | Content IDs shall be namespaced by mod id (`modid:item_id`). | M | 1 |
| REQ-MOD-007 | Content shall be validated against schemas on load with actionable diagnostics (file, line, field, expected). | M | 1 |
| REQ-MOD-008 | Dev builds shall hot-reload JSON5 and Lua while the game runs. | S | 1 |
| REQ-MOD-009 | In-engine editors: map editor (Phase 3), unit editor and formation editor (Phase 6), writing mod files. | S | 3 / 6 |
| REQ-MOD-010 | Mods shall be distributable as a folder or zip; Workshop or mod.io integration is out of scope. | M / W | 6 / — |
| REQ-MOD-011 | Average content mods shall be possible without programming knowledge; the flagship game's own content is the reference example. | M | 6 |

## 26. Save system

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-SAVE-001 | Campaign saves, battle saves (mid-battle), autosaves (end of each turn, start of each battle), and quick save shall be supported. | M | 4 |
| REQ-SAVE-002 | A save shall be a binary snapshot with a JSON header (engine version, schema version, active mods and versions, content registry hash, timestamp, thumbnail summary). | M | 4 |
| REQ-SAVE-003 | Saves shall carry a schema version; loading an older version runs migration functions forward; loading a newer version fails with a clear message. | M | 4 |
| REQ-SAVE-004 | Loading a save whose mod list differs from the active set shall warn and, if a required mod is missing, refuse. | M | 4 |
| REQ-SAVE-005 | Battle replays (BattleSetup plus command stream) shall be recordable and playable. | S | 3 |
| REQ-SAVE-006 | Save and load of a battle shall not change its state hash sequence (REQ-SIM-006). | M | 2 |

## 27. Multiplayer readiness

Multiplayer is not part of MVP. The following are architectural requirements that must hold from Phase 0 so that Phase 7 does not require a rewrite.

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-NET-001 | The simulation shall accept Commands from multiple sources tagged by player id and shall execute them in a defined order per tick. | M | 0 |
| REQ-NET-002 | The simulation shall support a configurable input delay (Commands issued at tick T execute at T + d). | M | 0 |
| REQ-NET-003 | The state hash, snapshot, and headless step shall be sufficient for a peer to verify and resynchronise (REQ-SIM-005, REQ-SIM-006). | M | 0 |
| REQ-NET-004 | Target model: peer-to-peer lockstep with one host as relay and tiebreaker; 2 to 4 players per battle. | S | 7 |
| REQ-NET-005 | Head-to-head campaign for 2 players. | C | 7 |
| REQ-NET-006 | Desync handling: per-tick hash exchange; on mismatch the host snapshot is authoritative and is resent. | S | 7 |
| REQ-NET-007 | Transport shall be abstracted behind a trait; a specific transport (UDP, Steam relay) is a Phase 7 decision. | S | 7 |
| REQ-NET-008 | The simulation shall support transferring control of a player's regiments to another player or to the engine AI through a Command, so that a dropped peer's side keeps fighting. | M | 2 |

## 28. Tooling

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-TOOL-001 | A headless CLI shall run a scenario file for N ticks and print the state hash, for determinism tests and benchmarks. | M | 0 |
| REQ-TOOL-002 | A benchmark suite shall measure per-system tick cost at 2k, 10k, 20k soldiers. | M | 1 |
| REQ-TOOL-003 | In-game profiler overlay showing per-system tick time and frame time. | M | 1 |
| REQ-TOOL-004 | Map editor: paint terrain zones and heights, place rivers, roads, settlement pieces, deployment zones; save to map JSON5. | S | 3 |
| REQ-TOOL-005 | Unit and formation editors as egui panels editing registry entries and writing mod files. | S | 6 |
| REQ-TOOL-006 | Replay viewer with seek via periodic snapshots. | C | 3 |
| REQ-TOOL-007 | Desync report tool: given two replays or hash logs, report the first divergent tick and component. | S | 7 |

## 29. Testing and quality

| ID | Requirement | Priority | Phase |
|---|---|---|---|
| REQ-TEST-001 | The sim crates shall have unit tests for every formula in the Simulation Spec. | M | 1 |
| REQ-TEST-002 | CI shall run a determinism test: the same scenario twice, hashes compared every tick, plus a snapshot-restore-continue variant. | M | 0 |
| REQ-TEST-003 | CI shall run the benchmark suite and fail if any system exceeds its budget by more than 20 %. | S | 2 |
| REQ-TEST-004 | Scenario tests shall assert battle outcomes within tolerance bands (e.g. 100 hoplites beat 100 peltasts in melee in 80–95 % of seeds). | S | 2 |
| REQ-TEST-005 | Content validation shall run in CI over the flagship game data. | M | 1 |
| REQ-TEST-006 | Cross-machine determinism check (two Windows machines, compare hash logs) before Phase 7. | S | 7 |

---

## 30. Roadmap

Phases have exit criteria, not dates. A phase is complete when every exit criterion is met and every Must requirement tagged with the phase is satisfied.

### Phase 0 — Foundations

**Deliverables:** Cargo workspace with crate skeleton and dependency rules; `il_core` (ids, Scalar, RNG streams, state hash, tick); `il_sim_battle` headless crate stepping an empty world by Commands; snapshot and restore; headless CLI; CI with determinism test.

**Exit criteria:** A scenario of 1,000 idle soldiers steps 10,000 ticks twice with identical hashes; snapshot at tick 5,000 and continue yields identical hashes; the workspace builds with `cargo clippy -D warnings`.

### Phase 1 — Battlefield prototype

**Deliverables:** JSON5 loader, schemas, mod manifest, override rules, registries; unit type and formation template content for 3 antiquity units; wgpu instanced renderer with isometric camera and interpolation; terrain rendering from heightmap and zones; formation system (templates, slots, reform, resize, wheel); regiment A* on nav grid; soldier steering and collision; input to Command pipeline with drag-formation; egui debug overlays and profiler; localisation keys; benchmark suite.

**Exit criteria:** 2,000 soldiers in 10 regiments move and reform at 60 FPS with sim tick ≤ 10 ms; a mod folder overriding a unit's speed takes effect without code changes; determinism test passes with movement.

### Phase 2 — Combat systems

**Deliverables:** Melee, ranged with simulated projectiles and pooling, abilities (buff/debuff), morale, fatigue, routing with escape flow fields, generals and auras, fog of war, deployment and battle phases, withdraw and pursuit, BattleSetup/BattleResult, scenario files, battle UI, main menu and custom battle, utility battle AI, audio event bus, scenario tests.

**Exit criteria:** 10,000 soldiers fight to a conclusion at 60 FPS with sim tick ≤ 25 ms; the AI wins against a passive player; scenario tests pass; determinism test passes with all combat systems.

### Phase 3 — Scaling

**Deliverables:** HPA*; LOD rendering tiers; render thread separation; parallel sim systems under determinism rules; mixed regiments and group formations; rivers, fords, bridges; map editor; replays; per-system budgets enforced in CI.

**Exit criteria:** 20,000 soldiers at ≥ 30 FPS with sim tick ≤ 50 ms; 32,768 soldiers run without crash; a handcrafted map made in the editor loads in a custom battle; a replay reproduces a recorded battle's final hash.

### Phase 4 — Campaign layer

**Deliverables:** Province map and graph, turn engine, economy, diplomacy (war, peace, trade, alliance), research, recruitment, experience and replenishment, campaign AI, campaign UI, campaign↔battle transitions, auto-resolve (per OQ-3), saves with migration, autosaves, weather.

**Exit criteria:** A 30-faction campaign runs 100 turns with AI only in under 5 s per turn; a player campaign can be saved mid-battle, loaded, and continued with identical hashes; battles launched from the campaign apply results correctly. **This completes MVP.**

### Phase 5 — Fantasy and siege

**Deliverables:** Remaining ability effects (damage, heal, summon, fear, area, teleport), energy resource, heroes, walls and gates, siege equipment, siege campaign state, agents, vassalage and coalitions.

**Exit criteria:** A fantasy faction defined purely in a mod plays a full campaign; a siege battle with wall assault completes.

### Phase 6 — Modding and tooling

**Deliverables:** Lua 5.4 sandbox and API, event and mission scripting, unit and formation editors, mod packaging, modding documentation, localisation for mods, audio modding, UI data-driving.

**Exit criteria:** A tester with no programming background creates a new faction with two units and a scripted campaign event using only the documentation and editors.

### Phase 7 — Multiplayer and platforms

**Deliverables:** Lockstep networking (2–4 players), lobby, desync detection and recovery, desync report tool, Linux and macOS builds, fixed-point scalar if cross-platform hashes diverge, head-to-head campaign (Could).

**Exit criteria:** Two machines complete a 10,000-soldier battle with identical hash logs; a forced desync recovers via host snapshot.

## 31. Success criteria

The project is successful when all of the following hold:

| # | Criterion | Requirements |
|---|---|---|
| 1 | A campaign with multiple AI factions runs end to end. | REQ-CAMP-*, REQ-AI-004 |
| 2 | Campaign battles transition to real-time battles and back with no manual steps. | REQ-SIM-060..062 |
| 3 | 20,000 individual soldiers are simulated at 30 FPS or better. | REQ-PERF-003 |
| 4 | Historical and fantasy factions coexist in one game. | REQ-ABIL-*, REQ-VIS-023 |
| 5 | New units and factions can be created entirely through mods. | REQ-MOD-001, REQ-MOD-011 |
| 6 | The simulation is deterministic and the lockstep seams exist. | REQ-SIM-001..009, REQ-NET-001..003 |
| 7 | No game content lives in engine code. | REQ-VIS-004, REQ-VIS-020 |

## 32. Assumptions

| # | Assumption |
|---|---|
| A-1 | One developer works on the project at hobby pace; scope is cut by dropping Could requirements before Should. |
| A-2 | `bevy_ecs` remains usable standalone with a stable enough API across its releases. |
| A-3 | `f32` determinism holds across Windows x86-64 machines when the same binary is used, fast-math is off, and no target-feature-dependent code paths exist in the sim crates. |
| A-4 | 20,000 full-agent soldiers with collision can fit in 50 ms per tick on 8 cores with a uniform grid and parallel systems. |
| A-5 | Handcrafted maps are feasible for the campaign map size (one map per province terrain type and settlement tier, not one per province). |
| A-6 | Antiquity content does not need magic, so the fantasy effect layer can be deferred to Phase 5 without redesign. |

## 33. Risks

| # | Risk | Impact | Mitigation |
|---|---|---|---|
| R-1 | 20k full-agent soldiers at 20 Hz exceed the tick budget. | P3 missed | Per-system budgets from Phase 1; LOD in the simulation (far-from-combat regiments update steering every other tick) as a reserved fallback; hard cap enforced. |
| R-2 | Per-projectile simulation is too expensive for 3,000 archers. | P2 tick budget | Projectile cap with deterministic statistical fallback (REQ-CMBT-015); pooling. |
| R-3 | `f32` results diverge across machines. | Multiplayer blocked | Scalar trait from day one; cross-machine hash check before Phase 7; fixed-point implementation ready. |
| R-4 | Editors and siege exceed solo capacity. | Phase 3, 5, 6 slip | Both are Should or Could; map editor is minimal (paint and place). |
| R-5 | `bevy_ecs` schedule introduces nondeterminism through parallel system ordering. | Determinism bugs | Explicit system ordering; parallel systems only where output is order-independent; determinism test in CI. |
| R-6 | Lua is later wanted inside the battle tick. | Determinism and performance | Explicitly forbidden for MVP; if reconsidered, a Lua-free deterministic subset must be specified first. |
| R-7 | Content schema churn breaks mods and saves. | Modder friction | Schema versioning and migration from Phase 1; deprecation policy in the Modding SDK. |
| R-8 | Handcrafted map count is unmanageable. | Phase 4 content | Assumption A-5; procedural stitching of handcrafted chunks is the fallback. |

## 34. Open questions

| # | Question | Owner doc | Needed by |
|---|---|---|---|
| OQ-1 | Camera rotation: 4 snap directions (4 sprite facing sets suffice at cost of diagonal facings) or 8 (8 facing sets, more art)? | TDD renderer section | Phase 1 |
| OQ-2 | postcard or bincode for snapshots? | TDD save section | Phase 0 |
| OQ-3 | Auto-resolve: headless simulation at accelerated speed (consistent, slow for big battles) or a statistical model (fast, second balance surface)? | Simulation Spec §14 | Phase 4 |
| OQ-4 | Should far-from-combat regiments update at reduced tick rate (simulation LOD) if R-1 materialises? | Simulation Spec | Phase 3 |
| OQ-5 | Reinforcement arrival: timed groups only, or also driven by campaign distance? | Simulation Spec §12 | Phase 4 |
| OQ-6 | Does fog of war apply in the deployment phase (blind deployment) or is the enemy deployment visible? | Simulation Spec §12 | Phase 2 |
| OQ-7 | Which JSON5 crate: `json5` (serde, mature) or `serde_json5`? | TDD data section | Phase 1 |
| OQ-8 | Audio crate: `kira` (game-oriented) or `rodio`? | TDD audio section | Phase 2 |
