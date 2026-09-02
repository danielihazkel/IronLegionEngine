# Iron Legion Engine — Glossary

**Status:** v0.2 · Owned by: all documents · Every other document links here and never redefines a term.

Terms are grouped by area. A term in *italics* inside a definition is itself defined in this glossary.

---

## Product and organisation

| Term | Definition |
|---|---|
| **Engine** | The reusable, content-agnostic part of the codebase: simulation systems, data loaders, renderer, UI framework, editors, scripting host. Lives under `crates/il_*`. |
| **Game** (flagship game) | The first title built on the Engine: antiquity setting (Rome, Greece, Persia). Its content and game-specific rules live under `game/`. Engine and Game share one repository for now; the boundary is a folder and a dependency rule, not a separate product. |
| **Mod** | A package of *Content* and optional Lua scripts that adds to or overrides Game or other Mod content. The flagship Game is itself structured as a Mod so the loader has one code path. |
| **Phase** | A roadmap stage (Phase 0 through Phase 7) with deliverables and measurable exit criteria. Phases have no dates. |
| **MVP** | Everything up to and including Phase 4 (playable campaign with battles) on Windows. |
| **REQ ID** | A requirement identifier of the form `REQ-<AREA>-nnn` defined only in the PRD. Other documents cite REQ IDs to show traceability. |
| **SIM rule** | A numbered simulation rule `SIM-<AREA>-nnn` defined only in the Simulation Design Spec. |
| **ADR** | Architecture Decision Record. A short entry in the SAD decision log: context, decision, consequences. |

## Time

| Term | Definition |
|---|---|
| **Tick** | One fixed step of the battle simulation. Fixed at 20 Hz, so one tick is 50 ms of simulated time. Every battle rule is expressed per tick or in ticks. |
| **Frame** | One rendered image. Frames are decoupled from ticks; the renderer interpolates entity positions between the last two ticks. |
| **Turn** | One campaign-time unit. The campaign is turn-based: each *Faction* acts in sequence, then the turn ends and the world advances. |
| **Sim time** | Time measured in ticks (battle) or turns (campaign). Never wall-clock time. The simulation crates have no access to the system clock. |
| **Accumulator** | The app-loop mechanism that converts variable wall-clock frame time into an integer number of ticks to step. |

## Determinism and networking

| Term | Definition |
|---|---|
| **Determinism** | Property that identical initial state, identical *Command* stream, and identical seed produce bit-identical *State Hash* sequences on every run. |
| **Command** | The only way anything outside the simulation changes simulation state. A serialisable order (move, attack, change formation, pause, set speed, ability) stamped with the tick at which it takes effect. Player input, AI decisions, and network peers all produce Commands. |
| **Command stream** | The ordered list of Commands per tick. A battle is fully described by *BattleSetup* plus its Command stream. |
| **Seed** | The 64-bit value that initialises all random streams for a battle or campaign. |
| **RNG stream** | An independent deterministic random generator derived from the Seed plus a system identifier, so that adding a random call in one system never changes results in another. |
| **State Hash** | A 64-bit digest of the simulation state computed at the end of a tick over a defined, ordered set of components. Used to detect divergence in tests and in multiplayer. |
| **Snapshot** | A complete serialised copy of simulation state at a given tick. Used for saves, replay seeking, and desync recovery. |
| **Replay** | A *BattleSetup* plus Command stream, optionally with periodic Snapshots. Re-simulating a replay reproduces the battle exactly. |
| **Lockstep** | Multiplayer model where all peers run the full simulation and exchange only Commands. Each tick executes only once every peer's Commands for that tick have arrived. |
| **Input delay** | Number of ticks between a Command being issued and the tick it executes. Hides network latency in Lockstep. |
| **Host** | In peer-to-peer lockstep, the one peer that relays Commands, breaks ties, and whose Snapshot is authoritative on *Desync*. |
| **Desync** | Two peers producing different State Hashes for the same tick. |
| **Scalar** | The trait behind all simulation arithmetic. Implemented by `f32` today; designed so a fixed-point type can replace it without touching gameplay code. |

## Battle entities

| Term | Definition |
|---|---|
| **Faction** | A political entity that owns *Provinces*, *Armies*, and diplomatic relations. In battle, a side. |
| **Army** | A campaign entity: a collection of *Regiments* under a *General* at one *Province*. In battle, all Regiments of one side that arrived together. |
| **Regiment** | The unit of command. A group of *Soldiers* sharing a *Formation*, a *Morale* value, orders, and a *Unit Type* (or several for mixed regiments). Players and AI give orders to Regiments, never to Soldiers. |
| **Soldier** | An individual simulated agent: position, velocity, collision circle, health, fatigue, slot assignment, target. Soldiers execute orders and hold formation; they do not decide. |
| **General** | A special Soldier with a *Leadership Aura*. May die; death causes a morale shock. One per Army. |
| **Unit Type** | A data definition (JSON5) describing a kind of soldier: stats, equipment, sprite set, abilities, cost. Example: `rome:hastati`. |
| **Projectile** | A simulated entity with a ballistic arc, spawned by ranged attacks, that hits whatever soldier circle it lands on. |
| **Entity cap** | Hard engine limit of 32,768 Soldier entities per battle. |

## Formations and movement

| Term | Definition |
|---|---|
| **Formation** | The spatial arrangement of a Regiment's Soldiers: a set of *Slots* generated by a *Formation Template* given soldier count, spacing, and *Facing*. |
| **Formation Template** | A data definition naming a *Layout Function* and its parameters (ranks, files, spacing, role zones). Built-in templates: Line, Column, Square, Wedge, Phalanx, Loose. |
| **Layout Function** | Engine code that turns (soldier count, template parameters) into Slot offsets relative to the Regiment anchor. |
| **Slot** | A target position and facing inside a Formation, assigned to exactly one Soldier or empty. |
| **Anchor** | The Regiment's reference point (centre of front rank) and Facing from which Slot offsets are computed. |
| **Facing** | The direction a Regiment or Soldier points, in radians in world space. Rendering quantises Facing to 8 sprite directions. |
| **Role zone** | A region of a Formation reserved for one Unit Type in a mixed Regiment (e.g. front ranks spearmen, rear ranks archers). |
| **Group formation** | An Army-level template arranging several Regiments (battle line, echelon, refused flank). |
| **Reform** | Re-assigning Soldiers to Slots after a change in count, template, or facing, minimising total travel cost. |
| **Morphing** | Changing Formation Template while keeping the Regiment functional (line to square). |
| **Formation integrity** | A 0–1 measure of how close Soldiers are to their Slots. Drives combat and morale modifiers. |
| **Nav grid** | The coarse walkability grid derived from a battle map, used for Regiment pathfinding. |
| **HPA\*** | Hierarchical Pathfinding A\*. The Nav grid is divided into clusters connected by gates; Regiments path on the abstract gate graph and refine inside clusters. |
| **Flow field** | A per-cell direction map toward a goal. Used for routing Soldiers (escape field toward own map edge). |
| **Steering** | Per-Soldier local movement: seek slot, separate from neighbours, avoid obstacles. Soldiers never run A\*. |
| **Spatial grid** | Uniform grid over the battlefield bucketing Soldiers and Projectiles for neighbour queries. |

## Combat, morale, fatigue

| Term | Definition |
|---|---|
| **Engagement** | The state of a Soldier having an enemy within melee reach. A Regiment is engaged when any of its Soldiers is. |
| **Attack cycle** | The per-Soldier timer, in ticks, between melee attacks. |
| **Hit roll** | The deterministic random comparison of attacker skill versus defender defence that decides whether an attack lands. |
| **Armour** | Flat damage reduction applied after a hit lands. |
| **Charge** | A Regiment moving at run speed into an enemy; grants a temporary attack bonus on first contact. |
| **Flank / rear attack** | Attack arriving from outside the defender's frontal arc; multiplies damage and morale penalty. |
| **Morale** | A Regiment-level value in [0, 100] modified each tick by *Morale factors*; mapped to a *Morale state*. |
| **Morale state** | One of Steady, Unsettled, Shaken, Broken, Routing, with hysteresis between thresholds. |
| **Rout** | Morale state in which the Regiment ignores orders and its Soldiers flee along the escape Flow field. May *Rally*. |
| **Shattered** | A Regiment that routed and cannot rally; leaves the battle. |
| **Rally** | Return from Routing to Shaken when morale recovers and no enemy is near. |
| **Leadership Aura** | Radius around a General inside which allied Regiments receive morale and combat bonuses. |
| **Fatigue** | A Soldier-level value accumulated by activity, mapped to Fresh, Active, Tired, Exhausted. Regiment fatigue is the mean. |
| **Ability** | A data-defined effect a Regiment or General can trigger: buff, debuff, damage, heal, summon, fear, area, teleport. Antiquity uses buffs and debuffs only; the rest are the Phase 5 fantasy layer. |
| **Status effect** | A time-limited modifier applied to a Regiment or Soldier by an Ability or terrain. |

## Battle flow and visibility

| Term | Definition |
|---|---|
| **BattleSetup** | The complete input to a battle: map id, seed, weather, per-side rosters, Generals, deployment zones, reinforcement groups. Produced by the campaign or a scenario file. |
| **BattleResult** | The complete output of a battle: per-Regiment casualties and experience, General fate, winner, loot, duration. Consumed by the campaign. |
| **Deployment phase** | Pre-battle phase where each side places Regiments inside its deployment zone. Ends when all players confirm. |
| **Battle phase** | Real-time (ticked) fighting with pause and speed control. |
| **Withdraw** | A player order to leave the field in good order via the own map edge. |
| **Pursuit phase** | Sub-phase after one side has entirely routed or withdrawn; remaining pursuers inflict casualties per pursuit rules until the timer ends. |
| **Line of sight** | Whether a Regiment can see a point given distance, elevation, and occluding terrain. |
| **Fog of war** | Per-Faction visibility: enemy Regiments are shown only when inside an allied Regiment's line of sight. |

## Campaign

| Term | Definition |
|---|---|
| **Province** | A polygonal region of the campaign map with one owner Faction, terrain type, resources, and a *Settlement*. Nodes of the *Province graph*. |
| **Province graph** | Adjacency graph of Provinces; campaign pathfinding runs on it. |
| **Settlement** | The Province's town: buildings, garrison, recruitment. |
| **Agent** | A campaign character other than a General (diplomat, spy). Post-MVP. |
| **Interception** | An Army moving into a Province containing a hostile Army triggers a battle. |
| **Auto-resolve** | Resolving a battle without playing it. Model to be decided (see PRD open questions). |
| **Technology** | A data-defined research item with prerequisites and effects. |
| **Recruitment pool** | Unit Types available at a Settlement given its buildings and Faction. |

## Data, saves, modding

| Term | Definition |
|---|---|
| **Content** | All data-defined game definitions: Unit Types, Factions, Formation Templates, Technologies, Buildings, Abilities, Maps, Localisation strings. Stored as JSON5. |
| **JSON5** | JSON with comments, trailing commas, unquoted keys. The canonical Content format. |
| **Content ID** | A namespaced string `modid:item_id` uniquely identifying one Content item. |
| **Registry** | The in-memory typed store of all loaded Content of one kind, after mod overrides are applied. |
| **Handle** | A cheap typed index into a Registry, used by the ECS instead of strings. |
| **Manifest** | `mod.json5` at a Mod's root: id, version, dependencies, engine version range, load order hints. |
| **Load order** | The resolved sequence in which Mods apply. Later Mods override earlier. |
| **Override** | A Content item in a later Mod with the same Content ID as an earlier one. Semantics: replace, deep-merge, or list operations, chosen explicitly in the data. |
| **Save** | A Snapshot plus JSON header (engine version, schema version, active Mod list). Campaign saves and battle saves share the format. |
| **Schema version** | Integer bumped whenever a saved struct changes; migration functions convert older versions forward. |
| **Hot reload** | Dev-only reloading of JSON5 and Lua while the game runs. |
| **Tier 1 modding** | Content-only mods (JSON5). No programming. |
| **Tier 2 modding** | Lua scripting of campaign events, missions, triggers, scenario logic. |
| **Sandbox** | The restricted Lua environment: no `io`, `os`, `require` of native code, no wall-clock, no unseeded random. |

## Rendering and UI

| Term | Definition |
|---|---|
| **Isometric** | Fixed-angle 2.5D presentation. World is 2D; the camera projects it at a fixed pitch with 4 or 8 snap rotations. |
| **Sprite facing set** | The 8 directional sprite variants of a Unit Type per animation. |
| **Instancing** | Drawing all Soldiers of one atlas in a single GPU draw call from an instance buffer. |
| **LOD** | Level of detail. Rendering tiers: Detailed Soldier (near), Reduced Soldier (mid), Sprite Aggregation (far: one sprite per block of soldiers). |
| **Interpolation** | Blending each Soldier's position between the previous and current tick by the frame's fractional tick progress. |
| **Command card** | The UI panel of orders and abilities for the selected Regiments. |
| **Regiment card** | The UI element showing one Regiment's type, strength, morale, fatigue. |
| **Drag-formation** | The RTS gesture: drag to define the front line width and facing of selected Regiments. |
