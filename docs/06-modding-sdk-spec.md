# Iron Legion Engine — Modding SDK Specification

| | |
|---|---|
| **Version** | 0.1 |
| **Status** | Draft for review |
| **Upstream** | [PRD v0.2](01-prd.md) · [SAD](02-sad.md) · [Glossary](00-glossary.md) |
| **Siblings** | [Simulation Spec](03-simulation-spec.md) · [TDD](04-tdd.md) · [Networking Spec](05-networking-spec.md) |
| **Schemas** | [schemas/](schemas/) |

This document is the contract between the engine and anyone who creates Content or scripts for it, including the flagship game itself. Terms in **bold** on first use are defined in the [Glossary](00-glossary.md) and are not redefined here.

---

## 1. Goals, tiers, personas

### 1.1 Goals

| Goal | Requirement |
|---|---|
| Everything in **Content** is definable and overridable in JSON5 without code. | REQ-MOD-001 |
| Campaign events, missions, quests, triggers, and scenario logic are scriptable in sandboxed Lua 5.4. | REQ-MOD-002, REQ-TECH-007 |
| Lua never executes inside the battle **Tick** for MVP. | REQ-MOD-003 |
| Every **Mod** has a **Manifest**; **Load order** is resolved from dependencies; later mods override earlier ones by **Content ID** with explicit semantics. | REQ-MOD-004, REQ-MOD-005, REQ-MOD-006 |
| Content is validated against schemas with actionable diagnostics. | REQ-MOD-007 |
| Dev builds **Hot reload** JSON5 and Lua. | REQ-MOD-008 |
| In-engine editors write mod files. | REQ-MOD-009, REQ-TOOL-004, REQ-TOOL-005 |
| Mods distribute as folder or zip. Workshop and mod.io are out of scope. | REQ-MOD-010 |
| Average content mods require no programming. The flagship **Game** is the reference example. | REQ-MOD-011 |
| The flagship Game is itself a Mod loaded through this same path. | REQ-VIS-021 |

### 1.2 Tiers

| Tier | What it can change | Tooling | Phase |
|---|---|---|---|
| **Tier 1** | Units, factions, formations, abilities, technologies, buildings, maps, AI weights (REQ-AI-007), audio references (REQ-AUD-005), UI layout data (REQ-UI-005), strings (REQ-LOC-002), engine rule tunables | Text editor, unit editor, formation editor, map editor | 1 (data), 3 (map editor), 6 (unit/formation editors) |
| **Tier 2** | Campaign events, missions, quests, triggers, scenario logic | Text editor, Lua | 6 |

There is no Tier 3. Battle behaviour is data plus engine Rust; a mod that needs new battle mechanics is an engine feature request.

### 1.3 Personas

| Persona | Wants to | Needs from the SDK |
|---|---|---|
| **Tweaker** (no code) | Rebalance a unit, recolour a faction, translate strings | A folder, a manifest, one JSON5 file per change, clear error messages |
| **Content author** (no code) | Add factions, units, formations, maps | Full Tier 1 reference (§4), editors (§6), a worked example to copy |
| **Scripter** (Lua) | Write campaign events and missions | Lua API reference (§5), event list, sandbox rules, error reporting |
| **Flagship developer** | Ship the antiquity game as a mod | Everything above plus schema versioning and migration policy (§11) |

---

## 2. Mod package layout and manifest

### 2.1 Folder layout

A Mod is a folder (or a zip containing exactly one top-level folder, §10) with a `mod.json5` at its root.

```
mymod/
  mod.json5                     manifest (required)
  content/                      content_root (default)
    units/*.json5
    factions/*.json5
    formations/*.json5
    abilities/*.json5
    technologies/*.json5
    buildings/*.json5
    maps/*.json5
    sprites/*.json5             sprite sheet frame tables (SpriteSet)
    ai/*.json5
    rules/                      engine tunables, one file per system
      morale.json5
      fatigue.json5
      combat.json5
      movement.json5
      battle_flow.json5
  locale/
    en.json5
    de.json5
  scripts/                      scripts_root (default), Tier 2 only
    main.lua
    missions/*.lua
  assets/                       assets_root (default)
    sprites/                    PNG sheets referenced by content/sprites/*.json5
    atlases/
    sounds/
    music/
```

Rules:

- File names inside `content/<kind>/` are free. The engine reads every `*.json5` under each kind folder recursively. A file contains either one content object or a JSON5 array of content objects of that kind.
- The kind of an object is determined by the folder it sits in, never by a field. A unit placed under `formations/` is a validation error.
- `content/rules/` files are objects keyed by tunable name, not arrays. The exact keys per file are owned by the Simulation Spec; the schemas under `schemas/` mirror them.
- Every path in the manifest is relative to the mod root and uses forward slashes.

### 2.2 Manifest fields

| Field | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string, `^[a-z0-9_]+$` | yes | — | The mod's namespace. Every Content ID this mod defines is `<id>:<item_id>`. Never changes across versions. |
| `name_key` | string | yes | — | Localisation key of the display name, resolved through this mod's own `locale/`. |
| `version` | semver string | yes | — | `MAJOR.MINOR.PATCH`. Recorded in saves (§8). |
| `engine_version` | semver range | yes | — | Engine versions this mod is written for, e.g. `">=0.4.0 <0.6.0"`. Loading outside the range warns; a range the engine cannot parse refuses. Range grammar (also for `dependencies[].version`): comparators separated by spaces, `*`, or one comparator; no space between an operator and its version (`>= 1.0.0` is an error). |
| `dependencies` | `[{id, version}]` | no | `[]` | Mods that must be present and loaded before this one. `version` is a semver range. |
| `load_after` | `[id]` | no | `[]` | Soft ordering: if these mods are present, load after them. Missing entries are ignored. |
| `load_before` | `[id]` | no | `[]` | Soft ordering: if these mods are present, load before them. |
| `content_root` | string | no | `"content"` | Folder holding the kind folders. |
| `scripts_root` | string | no | `"scripts"` | Folder holding Lua. `main.lua` inside it is the entry point. |
| `assets_root` | string | no | `"assets"` | Folder that asset references resolve against. |
| `locales` | `[string]` | no | `[]` | Language codes this mod provides files for under `locale/`. Informational; the loader also scans the folder. |

The flagship game's manifest at `game/mod.json5` has `id: "rome"` (the culture namespaces `rome`, `greece`, `persia` are item id prefixes inside it, not separate mods; see §3.5). Its `engine_version` is always the exact engine version in the workspace.

### 2.3 Worked manifest

```json5
// mymod/mod.json5
{
  id: "mymod",
  name_key: "mymod.mod.name",
  version: "1.2.0",
  engine_version: ">=0.4.0 <0.6.0",

  // Hard requirement: we derive units from the flagship game's content.
  dependencies: [
    { id: "rome", version: ">=1.0.0" },
  ],

  // If "better_ai" is installed, our AI weights should win over theirs.
  load_after: ["better_ai"],

  // Defaults shown explicitly for clarity; these lines may be omitted.
  content_root: "content",
  scripts_root: "scripts",
  assets_root: "assets",
  locales: ["en", "de"],
}
```

---

## 3. Load order and override semantics

### 3.1 Discovery

The engine scans the mod directories in this order and collects every folder or zip containing `mod.json5`:

1. `game/` (the flagship game, always present, always enabled).
2. `<install>/mods/` (bundled extras).
3. `<user data>/IronLegion/mods/` (user mods).

The user enables and disables mods in the launcher UI. Enabled state and the resolved load order are stored in `<user data>/IronLegion/modlist.json5` and copied into every **Save** header (§8).

Two enabled mods with the same `id` is an error: `modlist: duplicate mod id "mymod" at <pathA> and <pathB>`.

### 3.2 Dependency resolution algorithm

Input: the set of enabled manifests. Output: a total order, or a list of errors.

1. **Validate ranges.** For every `dependencies[i]`, the dependency must be enabled and its `version` must satisfy the range. Otherwise error `mymod: missing dependency "rome" (>=1.0.0)` or `mymod: dependency "rome" is 0.9.0, requires >=1.0.0`. Resolution stops here if any hard dependency fails.
2. **Build the graph.** Nodes are mods. Add a directed edge `A → B` (A loads before B) for: each `B.dependencies` entry naming A; each `B.load_after` entry naming A if A is enabled; each `A.load_before` entry naming B if B is enabled. `game` (`id: "rome"`) gets an implicit edge to every other mod.
3. **Detect cycles.** Run Tarjan's strongly connected components. Any component larger than one node is an error listing the cycle: `load order cycle: mymod -> better_ai -> mymod (via mymod.load_after, better_ai.load_before)`. Soft edges (`load_after`, `load_before`) that participate in a cycle are dropped one at a time, in manifest order, and the check repeats; if the cycle persists with only hard edges, resolution fails.
4. **Topological sort with a stable tie-break.** Kahn's algorithm; when several nodes are ready, pick the one whose `id` sorts first. This makes the order reproducible regardless of filesystem order.
5. **Record.** The resolved order is written to `modlist.json5` and logged at startup.

### 3.3 Application order

Content is applied mod by mod in load order. Within one mod, files are read in sorted path order and objects within a file in array order. Within a mod, defining the same Content ID twice is an error (`mymod/content/units/a.json5:12:3 id: duplicate "mymod:thracian_peltast" (first defined in content/units/b.json5:4:3)`). Across mods, the later mod's object is an **Override** of the earlier one.

### 3.4 Override directives

Any content object may carry these directive properties. They are stripped before schema validation of the merged result.

| Directive | Where | Meaning |
|---|---|---|
| `"$override": "merge"` | object root | Default. Deep-merge this object into the existing item with the same Content ID (§3.4.1). If no such item exists, the object is a new definition and must be complete. |
| `"$override": "replace"` | object root | Discard the existing item entirely and use this object as the complete definition. |
| `"$delete": true` | object root | Remove the item with this Content ID from the registry. No other fields are allowed alongside except `id`. Content that references the deleted ID later fails validation, naming the deleting mod. |
| `{"$append": [...]}` | any list field | Append these elements to the existing list (duplicates preserved). |
| `{"$remove": [...]}` | any list field | Remove every element equal to any listed element. Elements are compared by value; for lists of objects with an `id` field, by `id`. |
| `{"$replace": [...]}` | any list field | Replace the whole list. Same as writing the list directly. |

#### 3.4.1 Deep-merge rules

Given existing item `E` and override object `O` with `$override: merge`:

1. For each field in `O`:
   - If the value is a list-operation object (`$append`, `$remove`, `$replace`), apply it to `E`'s list. Missing list in `E` is treated as empty. Operations are applied in the order `$replace`, `$remove`, `$append` if several appear in one object.
   - Else if both `E[field]` and `O[field]` are objects, recurse.
   - Else `E[field] = O[field]`. A plain list in `O` replaces the list in `E` (lists are values, not merged element-wise, unless a list operation is used).
2. Fields absent from `O` are untouched.
3. `null` in `O` resets the field to its schema default (or removes it if optional with no default).
4. The merged result is validated against the kind's schema. Errors name the mod that performed the last write to the offending field.

#### 3.4.2 Examples

Rebalance one number (Tweaker):

```json5
// mymod/content/units/velites_tweak.json5
{
  id: "rome:velites",          // same ContentId as the flagship unit
  // $override defaults to "merge"
  ranged: { accuracy: 0.55 },  // only this field changes
}
```

Add an ability to an existing unit without knowing its current list:

```json5
{
  id: "rome:hastati",
  abilities: { $append: ["mymod:war_cry"] },
}
```

Remove a formation from a unit and forbid another mod's ability:

```json5
{
  id: "greece:hoplite",
  formations: { $remove: ["rome:loose"] },
  abilities: { $remove: ["better_ai:auto_charge"] },
}
```

Replace a unit wholesale:

```json5
{
  $override: "replace",
  id: "persia:immortal",
  name_key: "mymod.unit.immortal.name",
  category: "infantry",
  // ... every required field must be present
}
```

Delete content:

```json5
{ id: "rome:testudo", $delete: true }
```

### 3.5 Content ID namespacing

- A Content ID is `modid:item_id`, both `^[a-z0-9_]+$`. The `modid` part must equal the defining mod's `id` when the object is a *new* definition. An `id` with another mod's namespace is only legal as an override of an item that mod actually defines; otherwise: `mymod/content/units/x.json5:3:7 id: "rome:legionary_v2" is not defined by mod "rome"; new content must use the "mymod:" namespace`.
- The flagship game (`id: "rome"`) is the one exception: it may define ids under the namespaces `rome`, `greece`, and `persia`, declared in its manifest via `namespaces: ["rome", "greece", "persia"]`. This optional manifest field is only honoured for the mod at `game/`.
- References (`abilities: [...]`, `units: [...]`, `formations: [...]`) are always full Content IDs. There is no implicit current-mod prefix, so a file copied between mods keeps meaning the same thing.
- Content IDs are case-sensitive and stable forever. Renaming an item is a delete plus a new definition, and breaks saves (§8).

### 3.6 Validation and diagnostics

JSON5 files are parsed to plain JSON values first, then the merged result per Content ID is validated against the kind's JSON Schema (draft 2020-12) in `schemas/`. Line and column information is preserved from the JSON5 parse so errors point at the source, not the merged object. Validation collects every error across all mods before failing (SAD §9.2).

Error format, one per line:

```
<mod-relative file>:<line>:<col> <field path>: <message> (expected <constraint>)
```

Examples:

```
mymod/content/units/peltast.json5:14:5 ranged.accuracy: value 1.4 out of range (expected 0.0..=1.0)
mymod/content/units/peltast.json5:3:3 id: unknown reference in abilities[1] "mymod:war_cri" (expected an existing ability ContentId; nearest: "mymod:war_cry")
mymod/content/formations/deep.json5:9:3 role_zones[0].ranks_to: 12 exceeds max_ranks 8 (expected <= max_ranks)
mymod/content/factions/thrace.json5:1:1 <root>: missing required field "colour_primary" (expected #rrggbb string)
rome/content/units/hastati.json5:2:3 armour: after merge by "mymod" (content/units/tweak.json5:5:3): value -2 out of range (expected 0..=100)
```

The last form appears when a merge produces an invalid result; both the original location and the overriding mod's location are named.

Warnings (do not stop loading):

- Unknown field under an object: `... unknown field "amour" (did you mean "armour"?)`. Unknown fields are rejected by `additionalProperties: false`, so this is an error, not a warning, except for fields the schema marks deprecated (§11).
- Manifest `engine_version` range does not include the running engine.
- A mod overrides an item that no enabled mod defines (the override is ignored).

---

## 4. Tier 1 content reference

Every content kind below lists its fields. Types: `id` (Content ID string), `key` (localisation key), `f` (number, decimal), `i` (integer), `b` (boolean), `[T]` (list). Units: world units (wu) are the battle coordinate unit; the flagship game uses 1 wu = 1 metre. Ticks are 50 ms. Field names are the same identifiers used in Simulation Spec formulas and TDD structs.

### 4.1 Unit types — `content/units/`

Schema: [`schemas/unit-type.schema.json`](schemas/unit-type.schema.json). Satisfies REQ-CMBT-002, REQ-CMBT-011, REQ-CAMP-041.

| Field | Type | Unit | Default | Meaning |
|---|---|---|---|---|
| `id` | id | | required | Content ID |
| `name_key` | key | | required | Display name key |
| `category` | enum | | required | `infantry`, `cavalry`, `ranged`, `skirmisher`, `general`, `siege`. Drives role zones, anti-cavalry, AI considerations. |
| `soldier_radius` | f | wu | 0.4 | Collision circle radius |
| `mass` | f | | 1.0 | Push weight in collision and charge resolution |
| `hp` | i | | required | Hit points per soldier |
| `speed_walk` | f | wu/s | required | Formation walking speed |
| `speed_run` | f | wu/s | required | Charge and rout speed |
| `speed_march` | f | wu/s | `speed_walk` | Column/march speed on roads |
| `attack` | f | | required | Melee attack skill |
| `defence` | f | | required | Melee defence skill |
| `armour` | f | | 0 | Flat damage reduction |
| `damage` | f | | required | Melee damage before armour |
| `attack_interval_ticks` | i | ticks | 20 | Attack cycle length |
| `reach` | f | wu | 1.0 | Melee reach from circle edge |
| `charge_bonus` | f | | 0 | Added to attack on first contact after a charge |
| `anti_cavalry_bonus` | f | | 0 | Added to attack and defence versus `cavalry` when braced |
| `second_rank_attack` | b | | false | Second rank may attack (spears, pikes) |
| `frontal_arc_deg` | f | degrees | 120 | Arc inside which attacks are frontal |
| `ranged` | object | | none | Present only for units that shoot; see below |
| `ranged.range` | f | wu | required | Maximum range |
| `ranged.min_range` | f | wu | 0 | Minimum range (indirect fire) |
| `ranged.accuracy` | f | 0..1 | required | Base probability of landing on the aimed point's soldier |
| `ranged.projectile_speed` | f | wu/s | required | Launch speed |
| `ranged.reload_ticks` | i | ticks | required | Ticks between volleys |
| `ranged.ammo` | i | | required | Volleys per soldier per battle |
| `ranged.damage` | f | | required | Projectile damage before armour |
| `ranged.armour_penetration` | f | | 0 | Subtracted from target armour |
| `ranged.arc` | enum | | `direct` | `direct` (flat, blocked by friends) or `indirect` (lobbed, fires over friends) |
| `morale_base` | f | 0..100 | 60 | Starting regiment morale contribution |
| `fatigue_rate_mult` | f | | 1.0 | Multiplier on fatigue accumulation |
| `los_radius` | f | wu | 80 | Line-of-sight radius on flat open ground |
| `abilities` | [id] | | `[]` | Ability Content IDs |
| `formations` | [id] | | required | Formation templates this unit may use; first is default |
| `sprite_set` | string | | required | Path under `assets_root` to the 8-facing atlas set |
| `sounds` | object | | `{}` | `select`, `move`, `attack`, `charge`, `die` → paths under `assets_root` |
| `cost` | i | gold | required | Recruitment cost |
| `upkeep` | i | gold/turn | required | Per-turn upkeep |
| `recruit_turns` | i | turns | 1 | Turns to recruit |
| `tier` | i | 1..5 | 1 | Recruitment tier (building requirement) |
| `experience_tiers` | [object] | | `[]` | `{xp, attack, defence, morale}` per tier; additive bonuses |

Worked example: a Thracian peltast derived from the flagship Velites by merge.

```json5
// mymod/content/units/thracian_peltast.json5
{
  // New definition in our namespace...
  id: "mymod:thracian_peltast",
  // ...but we do not want to retype 30 fields, so we inherit from rome:velites.
  // "$from" copies the referenced item as the base, then merges the rest.
  $from: "rome:velites",

  name_key: "mymod.unit.thracian_peltast.name",
  category: "skirmisher",

  // Thracians: faster, harder-hitting javelins, weaker armour.
  speed_run: 5.2,
  armour: 0,
  ranged: {
    accuracy: 0.5,
    damage: 14,
    ammo: 8,
  },

  abilities: { $append: ["mymod:rhomphaia_frenzy"] },
  formations: { $replace: ["rome:loose", "rome:line"] },

  sprite_set: "sprites/units/thracian_peltast",
  sounds: { select: "sounds/voice/thracian_select.ogg" },

  cost: 380,
  upkeep: 45,
  tier: 1,
}
```

Merged result (what the engine validates and loads; `tests/mods/sdk_example/` holds this exact fixture and `tests/tests/sdk_example.rs` checks it):

```json5
{
  id: "mymod:thracian_peltast",
  name_key: "mymod.unit.thracian_peltast.name",
  category: "skirmisher",
  soldier_radius: 0.4, mass: 70, hp: 80,                // inherited from rome:velites
  speed_walk: 1.8, speed_run: 5.2, speed_march: 1.8,    // speed_run overridden
  attack: 25, defence: 20, armour: 0, damage: 25,       // armour overridden
  attack_interval_ticks: 32, reach: 0.5, charge_bonus: 0.1, anti_cavalry_bonus: 0,
  second_rank_attack: false, shield: false, frontal_arc_deg: 120,
  ranged: { range: 40, min_range: 5, accuracy: 0.5, projectile_speed: 20, reload_ticks: 80,
            ammo: 8, damage: 14, armour_penetration: 0.3, arc: "direct" },   // nested merge: only damage changed
  morale_base: 50, fatigue_rate_mult: 1.0, los_radius: 250,
  abilities: ["mymod:rhomphaia_frenzy"],                // $append onto the inherited empty list
  formations: ["rome:loose", "rome:line"],              // $replace dropped rome:column
  sprite_set: "sprites/units/thracian_peltast",
  sounds: { select: "sounds/voice/thracian_select.ogg", move: "sounds/voice/velites_move.ogg" },  // sibling key kept
  cost: 380, upkeep: 45, recruit_turns: 1, tier: 1, experience_tiers: [],
}
```

`$from` is the fourth directive: `"$from": "<ContentId>"` copies an existing item of the same kind (after all earlier mods have applied) as the base for a *new* Content ID, then applies the rest of the object with merge rules. It is resolved before schema validation. A `$from` chain deeper than 8 or a cycle is an error.

### 4.2 Formation templates — `content/formations/`

Schema: [`schemas/formation-template.schema.json`](schemas/formation-template.schema.json). Satisfies REQ-FORM-001, REQ-FORM-002, REQ-FORM-007, REQ-FORM-008.

| Field | Type | Unit | Default | Meaning |
|---|---|---|---|---|
| `id` | id | | required | |
| `name_key` | key | | required | |
| `layout` | enum | | required | `line`, `column`, `square`, `wedge`, `phalanx`, `loose`, `custom`. Names the engine **Layout Function**. |
| `default_ranks` | i | | required | Ranks when the player has not adjusted depth |
| `min_ranks` | i | | 1 | |
| `max_ranks` | i | | 16 | |
| `spacing_file` | f | wu | 1.0 | Distance between adjacent soldiers in a rank |
| `spacing_rank` | f | wu | 1.2 | Distance between ranks |
| `role_zones` | [object] | | `[]` | `{unit_category, ranks_from, ranks_to}` (inclusive, 1-based from the front). Only meaningful in mixed regiments. |
| `morph_ticks` | i | ticks | 60 | Transition time when morphing into this template |
| `integrity_bonus_attack` | f | | 0 | Attack bonus at integrity 1.0, scaled linearly |
| `integrity_bonus_defence` | f | | 0 | Defence bonus at integrity 1.0 |
| `speed_mult` | f | | 1.0 | Movement speed multiplier in this template |
| `custom_slots` | [object] | spacing units | required if `layout == custom` | `{x, y}` offsets from the **Anchor**; x is along the front (right positive), y toward the rear. Multiplied by `spacing_file` / `spacing_rank`. Soldiers beyond the slot count are appended in extra ranks. |

Worked example:

```json5
// mymod/content/formations/deep_phalanx.json5
{
  id: "mymod:deep_phalanx",
  name_key: "mymod.formation.deep_phalanx.name",
  layout: "phalanx",
  default_ranks: 16,
  min_ranks: 8,
  max_ranks: 32,
  spacing_file: 0.9,
  spacing_rank: 1.0,
  morph_ticks: 100,
  integrity_bonus_attack: 2,
  integrity_bonus_defence: 6,
  speed_mult: 0.7,
}
```

### 4.3 Factions — `content/factions/`

Schema: [`schemas/faction.schema.json`](schemas/faction.schema.json). Satisfies REQ-CAMP-010, REQ-CAMP-032, REQ-AI-004.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | id | required | |
| `name_key` | key | required | |
| `culture` | string | required | Free tag grouping factions (`hellenic`, `latin`, `thracian`); used by buildings and technologies as a filter |
| `colour_primary` | `#rrggbb` | required | Map and sprite tint |
| `colour_secondary` | `#rrggbb` | required | |
| `units` | [id] | required | Unit types this faction may recruit (subject to buildings and tier) |
| `starting_provinces` | [string] | `[]` | Province ids on the campaign map owned at start |
| `ai_profile` | id | required | Content ID of an AI profile under `content/ai/` |
| `diplomacy_personality` | object | see schema | `aggression`, `loyalty`, `greed`, `expansionism` in 0..1 |
| `tech_tree` | id | required | Content ID of a technology tree definition |

Worked example:

```json5
// mymod/content/factions/thrace.json5
{
  id: "mymod:thrace",
  name_key: "mymod.faction.thrace.name",
  culture: "thracian",
  colour_primary: "#7a1f1f",
  colour_secondary: "#e0c060",
  units: [
    "mymod:thracian_peltast",
    "mymod:rhomphaia_warrior",
    "greece:hoplite",        // reuse flagship content
    "rome:light_cavalry",
  ],
  starting_provinces: ["thracia_interior", "thracia_coast"],
  ai_profile: "mymod:tribal_raider",
  diplomacy_personality: { aggression: 0.8, loyalty: 0.3, greed: 0.6, expansionism: 0.5 },
  tech_tree: "greece:hellenic_tree",
}
```

### 4.4 Abilities — `content/abilities/`

Satisfies REQ-ABIL-001..003. MVP effect types are `buff` and `debuff`; the others validate but are rejected at load unless the engine build enables the Phase 5 fantasy layer.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id`, `name_key` | | required | |
| `description_key` | key | required | Tooltip |
| `cooldown_ticks` | i | required | |
| `duration_ticks` | i | 0 | 0 = instant |
| `energy_cost` | f | 0 | Phase 5 resource; must be 0 for antiquity |
| `targeting` | enum | required | `self`, `regiment`, `area`, `point` |
| `radius` | f | 0 | For `area` |
| `effects` | [object] | required | Each `{type, stat, amount, stacking}`; `type` ∈ buff, debuff, damage, heal, summon, fear, area, teleport |
| `effects[].stacking` | enum | `refresh` | `refresh`, `stack`, `highest` |
| `icon` | string | required | Path under `assets_root` |

### 4.5 Technologies — `content/technologies/`

Satisfies REQ-CAMP-040.

| Field | Type | Meaning |
|---|---|---|
| `id`, `name_key`, `description_key` | | |
| `tree` | id | Tree this node belongs to |
| `category` | string | Data-defined (`military`, `economic`, `political`; `magic` in Phase 5) |
| `prerequisites` | [id] | |
| `cost_turns` | i | |
| `effects` | [object] | `{kind: unlock_unit \| unlock_building \| modifier, target, value}` |
| `cultures` | [string] | Cultures that may research it; empty = all |

### 4.6 Buildings — `content/buildings/`

Satisfies REQ-CAMP-022.

| Field | Type | Meaning |
|---|---|---|
| `id`, `name_key`, `description_key` | | |
| `cost`, `build_turns`, `maintenance` | i | |
| `prerequisites` | [id] | Buildings and technologies |
| `chain` | string | Upgrade chain name; one building per chain per settlement |
| `effects` | [object] | `{kind: recruit_tier \| tax_mult \| production \| growth \| garrison, value}` |
| `cultures` | [string] | |

### 4.7 Maps — `content/maps/`

Battle maps are large; the schema is summarised in §6.1 because the map editor is their normal author. Reserved siege fields exist from Phase 1 (REQ-SIM-045).

### 4.8 AI profiles — `content/ai/`

Satisfies REQ-AI-007. An AI profile is an object of consideration weights per decision, keyed by the consideration names the Simulation Spec defines (`SIM-AI-*`). Unknown consideration names are errors; omitted ones take the engine default. Personality fields duplicate `diplomacy_personality` for campaign AI when a faction lacks its own.

### 4.9 Engine rule tunables — `content/rules/`

Each file is one object of named tunables for one system. The Simulation Spec owns the names and defaults; this SDK only fixes the mechanism: a later mod's `rules/morale.json5` merges over earlier ones field by field. Example:

```json5
// mymod/content/rules/morale.json5
{
  // Routing allies frighten neighbours more strongly in our mod.
  routing_ally_penalty: 6.0,
  routing_ally_radius: 45.0,
}
```

### 4.10 Locale — `locale/<lang>.json5`

See §7.

---

## 5. Tier 2 Lua

### 5.1 Sandbox setup

The engine embeds Lua 5.4 through `mlua` (REQ-TECH-007). Each enabled mod with a `scripts_root/main.lua` gets its own Lua state. Nothing is shared between mods except through campaign state and events.

The **Sandbox** removes or replaces:

| Standard | Status | Replacement |
|---|---|---|
| `io`, `os`, `debug`, `package.loadlib`, `load` with binary chunks, `dofile`, `loadfile` | removed | none |
| `require` | replaced | Loads only `.lua` files under the mod's own `scripts_root`; module name maps to a relative path with `.` as separator |
| `print` | replaced | `il.log.info` |
| `math.random`, `math.randomseed` | removed | `il.rng.next()`, `il.rng.range(lo, hi)` |
| `os.time`, `os.clock`, `os.date` | removed | `il.campaign.turn()`; there is no wall-clock |
| `string`, `table`, `math` (rest), `utf8`, `coroutine` | kept | |
| Memory | limited | 64 MiB per state; exceeding it raises an error and disables the mod's scripts for the session |
| Instructions | limited | A per-event budget of 50 million instructions; exceeding it aborts the handler with an error |

### 5.2 Lifecycle

```
engine start
  └─ mods resolved, content loaded
campaign start or load
  └─ for each mod in load order: create Lua state, run main.lua (registers handlers only)
  └─ event "campaign_loaded" (once)
each turn
  └─ turn_start → player phase → AI phase → resolution events → turn_end
campaign end / return to menu
  └─ Lua states destroyed
```

`main.lua` must only register handlers and define functions. Mutating campaign state at load time is an error (`il.campaign.command` raises `not in an event handler`).

Script state that must survive a save lives in `il.state`, a table that the engine serialises into the Save as JSON (strings, numbers, booleans, nested tables with string or integer keys; functions and userdata are rejected with an error naming the key path). Everything else is rebuilt from `main.lua` on load.

### 5.3 API reference

Namespace `il` is the only global the engine adds.

**Events**

| Function | Meaning |
|---|---|
| `il.events.on(name, fn)` | Register `fn` for event `name`. Handlers run in load order of mods, registration order within a mod. |
| `il.events.off(name, fn)` | Unregister. |

| Event | Arguments | When |
|---|---|---|
| `campaign_loaded` | `{turn, is_new}` | After all mods' `main.lua` ran |
| `turn_start` | `{turn, faction_id}` | At the start of each faction's phase |
| `turn_end` | `{turn}` | After resolution |
| `battle_start` | `setup` (read-only BattleSetup view) | Before the app switches to the battle |
| `battle_end` | `result` (read-only BattleResult view) | After the campaign applied the result |
| `province_captured` | `{province_id, old_owner, new_owner}` | |
| `faction_destroyed` | `{faction_id}` | |
| `treaty_signed` | `{a, b, kind}` | `kind` ∈ peace, trade, alliance |
| `war_declared` | `{aggressor, defender}` | |
| `tech_researched` | `{faction_id, tech_id}` | |
| `building_completed` | `{province_id, building_id}` | |
| `army_created` | `{army_id, faction_id, province_id}` | |
| `general_died` | `{general_id, faction_id, battle_id}` | |

**Campaign read API** (all return plain tables, snapshots of the current state)

| Function | Returns |
|---|---|
| `il.campaign.turn()` | integer |
| `il.campaign.factions()` | list of `{id, name, treasury, provinces: [id], at_war_with: [id]}` |
| `il.campaign.faction(id)` | one of the above or `nil` |
| `il.campaign.provinces()` / `il.campaign.province(id)` | `{id, owner, terrain, buildings: [id], settlement: {tier, garrison: [army_id]}}` |
| `il.campaign.armies()` / `il.campaign.army(id)` | `{id, faction, province, general, regiments: [{unit, count, xp}]}` |
| `il.campaign.player_faction()` | id |
| `il.campaign.relation(a, b)` | `{attitude, state}` |

**Campaign Commands** — the only way to change state. Each returns `true` or `false, reason`.

| Command | Effect |
|---|---|
| `il.campaign.command("grant_gold", {faction, amount})` | Add (or subtract) treasury |
| `il.campaign.command("spawn_army", {faction, province, regiments = {{unit, count}}})` | |
| `il.campaign.command("transfer_province", {province, to})` | |
| `il.campaign.command("set_relation", {a, b, state})` | `state` ∈ war, peace, alliance |
| `il.campaign.command("grant_tech", {faction, tech})` | |
| `il.campaign.command("add_building", {province, building})` | Completed immediately |
| `il.campaign.command("modifier", {faction, key, value, turns})` | Temporary faction-wide modifier from the engine's modifier vocabulary |

Commands issued from Lua enter the campaign command stream stamped with the current turn and the issuing mod id, so they are replayed and hashed like any other **Command**.

**Missions**

| Function | Meaning |
|---|---|
| `il.mission.create({id, title_key, description_key, faction, objective, reward, turns})` | `objective` ∈ `{kind = "capture_province", province}`, `{kind = "destroy_faction", faction}`, `{kind = "own_provinces", count}`, `{kind = "research", tech}`, `{kind = "custom"}` |
| `il.mission.complete(id)` / `il.mission.fail(id)` | For `custom` objectives |
| `il.mission.active(faction)` | list |

**UI, RNG, log**

| Function | Meaning |
|---|---|
| `il.ui.notify(key, params)` | Localised notification (`params` substituted into `{name}` placeholders) |
| `il.ui.popup(key, params, choices)` | Blocking popup during the player phase; `choices` is a list of `{key, fn}` |
| `il.rng.next()` | float in [0,1) from a stream seeded by campaign seed + turn + mod id |
| `il.rng.range(lo, hi)` | integer in [lo, hi] |
| `il.log.info(msg)`, `il.log.warn(msg)`, `il.log.error(msg)` | To the engine log, prefixed with the mod id |
| `il.loc(key, params)` | Resolve a string in the current locale |

### 5.4 Examples

Grant gold each turn to a faction holding a specific province:

```lua
-- mymod/scripts/main.lua
il.events.on("turn_start", function(e)
  local prov = il.campaign.province("thracia_coast")
  if prov and prov.owner == e.faction_id then
    il.campaign.command("grant_gold", { faction = e.faction_id, amount = 150 })
    if e.faction_id == il.campaign.player_faction() then
      il.ui.notify("mymod.event.coastal_trade", { amount = 150 })
    end
  end
end)
```

A mission with an objective and reward:

```lua
il.events.on("campaign_loaded", function(e)
  if e.is_new then
    il.mission.create({
      id = "mymod.unite_thrace",
      title_key = "mymod.mission.unite_thrace.title",
      description_key = "mymod.mission.unite_thrace.desc",
      faction = "mymod:thrace",
      objective = { kind = "own_provinces", count = 6 },
      reward = { gold = 2000, tech = "greece:phalanx_drill" },
      turns = 40,
    })
  end
end)
```

A scripted consequence of a battle: a crushing defeat triggers a revolt army.

```lua
il.events.on("battle_end", function(result)
  if result.loser == "mymod:thrace" and result.loser_casualty_ratio > 0.7 then
    il.state.thrace_defeats = (il.state.thrace_defeats or 0) + 1
    if il.state.thrace_defeats >= 2 and il.rng.next() < 0.5 then
      il.campaign.command("spawn_army", {
        faction = "rebels",
        province = "thracia_interior",
        regiments = { { unit = "mymod:rhomphaia_warrior", count = 120 } },
      })
      il.ui.notify("mymod.event.thracian_revolt", {})
    end
  end
end)
```

### 5.5 Error reporting

- Syntax errors in `main.lua` disable that mod's scripts and show a startup dialog: `mymod/scripts/main.lua:12: '=' expected near 'then'`.
- Runtime errors inside a handler are caught per handler; the handler is skipped, the error is logged with a Lua traceback, and a non-blocking UI warning names the mod. Other handlers for the same event still run.
- Three runtime errors from the same handler in one campaign disable that handler until reload.
- Command rejections (`false, reason`) are not errors; scripts should check them.

### 5.6 Determinism rules

Scripts run inside the campaign simulation's turn resolution and therefore affect the campaign **State Hash** (REQ-SIM-002). To keep the campaign deterministic:

- Only `il.rng` for randomness. The stream is derived from the campaign **Seed**, the turn, and the mod id, so results are reproducible and independent of other mods.
- Iteration over Lua tables with string keys is not ordered; the API returns lists (arrays) wherever order matters, and scripts must not use `pairs` over API results to drive Commands. `ipairs` is safe.
- No access to time, files, network, or environment.
- Handler registration order is fixed by load order, so two mods reacting to the same event always run in the same order.

### 5.7 Forbidden

- Executing during the battle **Tick** (REQ-MOD-003). There is no battle API in MVP; `battle_start` and `battle_end` fire outside the battle.
- Loading native libraries or other mods' scripts.
- Storing functions or userdata in `il.state`.
- Calling `il.campaign.command` outside an event handler.

---

## 6. Editors

Editors are engine features (`il_editor`, SAD §5) that read the loaded registries and write JSON5 into a target mod folder chosen by the user. They never write into `game/` unless the user explicitly selects it.

### 6.1 Map editor (Phase 3, REQ-TOOL-004)

Tools:

| Tool | Writes |
|---|---|
| Terrain zone brush | `zones` polygons with a `type` from `open`, `forest`, `marsh`, `rock`, `road` (REQ-SIM-041) |
| Height brush (raise, lower, smooth, flatten) | `heightmap` |
| River tool (polyline with width) plus ford and bridge markers | `rivers[]`, `crossings[]` (REQ-SIM-042) |
| Road tool | zones of type `road` |
| Settlement pieces (Phase 5 content, placeable but inert before then) | `structures[]` |
| Deployment zones (one polygon per side, plus reinforcement edges) | `deployment[]`, `reinforcement_edges[]` |
| Metadata panel | `id`, `name_key`, `size`, `campaign_terrain_tags`, `weather_allowed` |

Map JSON5 summary (full schema in TDD §6):

```json5
{
  id: "mymod:thracian_hills",
  name_key: "mymod.map.thracian_hills.name",
  size: { w: 2000, h: 1600 },                    // world units
  campaign_terrain_tags: ["hills", "forest"],    // campaign picks maps by tag
  weather_allowed: ["clear", "rain", "fog"],
  heightmap: { cell: 4, path: "maps/thracian_hills.hgt" }, // 16-bit raw under assets_root
  zones: [ { type: "forest", polygon: [[100,100],[400,120],[380,300]] } ],
  rivers: [ { width: 12, points: [[0,800],[900,760],[2000,900]] } ],
  crossings: [ { kind: "ford", at: [900,760], width: 30 }, { kind: "bridge", at: [1500,850], width: 8 } ],
  deployment: [ { side: 0, polygon: [...] }, { side: 1, polygon: [...] } ],
  reinforcement_edges: [ { side: 0, edge: "west" } ],

  // Reserved from Phase 1 (REQ-SIM-045). Must be present, may be empty.
  structures: [],       // { kind: "wall" | "gate" | "tower", polyline | at, hp, faction_side }
  siege_points: [],     // { kind: "ladder" | "ram" | "tower", at, facing }
}
```

### 6.2 Unit editor and formation editor (Phase 6, REQ-TOOL-005)

- egui panels over the `UnitType` and `FormationTemplate` registries. Fields are edited with the same ranges the schemas enforce, so the editor cannot produce an invalid file.
- Saving writes a single content object per file to `<target mod>/content/units/<item_id>.json5` or `.../formations/<item_id>.json5`, with `$override: "merge"` and only the changed fields when editing another mod's item, or a complete object when creating a new id.
- The formation editor previews slot layout for a chosen soldier count and lets the user drag `custom_slots`.
- Both editors hot-reload their output (§9) so a change is visible in a running custom battle after the next tick.

---

## 7. Localisation in mods

Satisfies REQ-LOC-001..004.

- Every user-visible string is a key (REQ-LOC-001). Keys are `modid.kind.item.field`, for example `mymod.unit.thracian_peltast.name`, `mymod.mission.unite_thrace.title`, `mymod.event.coastal_trade`. Engine keys use the `il.` prefix (`il.battle.deploy.confirm`).
- `locale/<lang>.json5` is a flat object of key → string. Nested objects are flattened with `.`:

```json5
// mymod/locale/en.json5
{
  mymod: {
    mod: { name: "Thracian Tribes" },
    unit: { thracian_peltast: { name: "Thracian Peltast" } },
    formation: { deep_phalanx: { name: "Deep Phalanx" } },
    faction: { thrace: { name: "Thrace" } },
    event: { coastal_trade: "Coastal trade brings {amount} gold." },
  },
}
```

- Placeholders are `{name}` and are substituted from the `params` table. Plural rules and gender are out of scope; write neutral strings.
- Fallback chain for a key in locale `L`: last-loaded mod's `L` → earlier mods' `L` → same chain for `en` → the key itself, rendered literally and logged once as a warning.
- Mods may override any key, including engine `il.*` keys, by providing it in their `locale/` (REQ-LOC-002).
- A mod adds a language by shipping `locale/<lang>.json5`; the language appears in settings once at least one enabled mod provides it. MVP fonts cover Latin, Cyrillic, and Greek (REQ-LOC-004).

---

## 8. Save compatibility

Satisfies REQ-SAVE-004.

The **Save** header records:

```json5
{
  engine_version: "0.5.2",
  schema_version: 7,
  mods: [
    { id: "rome",  version: "1.0.0" },
    { id: "mymod", version: "1.2.0" },
  ],
  // load order is the array order
}
```

On load the engine compares the header to the enabled set:

| Situation | Behaviour |
|---|---|
| Identical ids, versions, order | Load silently. |
| Same ids, a mod's version differs | Warn (`mymod 1.2.0 → 1.3.0`), load. |
| Extra enabled mod not in the save | Warn, load. New content simply exists; new starting provinces are ignored on an existing campaign. |
| Mod in the save is missing and was not a hard dependency of another present mod | Warn, load. Content IDs the save references that no longer exist are handled per kind: units become the engine placeholder unit `il:missing_unit` and are flagged in the UI; buildings and technologies are dropped; missing factions abort the load. |
| Mod in the save is missing and is a dependency of a present mod | Refuse: `cannot load: save requires "mymod" 1.2.0 (dependency of "mymod_addon")`. |
| Different load order | Warn, load with the current order. Merged values may differ; this is the modder's responsibility. |
| `schema_version` newer than the engine | Refuse (REQ-SAVE-003). |

Handles in snapshots are stored as Content IDs, not indices, so registry reordering between mod sets does not corrupt a save (SAD §7).

Expectations for modders:

- Never rename or delete a Content ID in a released mod version if you want saves to survive. Deprecate instead: keep the old id, mark it with `deprecated: "use mymod:new_id"` (a schema-level field every kind accepts), and hide it from recruitment.
- Changing numeric fields is always save-safe.
- Changing a `formations` list may leave saved regiments in a template they can no longer use; the engine reverts them to the first allowed template at battle load and logs it.

---

## 9. Hot reload

Dev builds only (REQ-MOD-008; SAD §9.4).

- The loader watches every enabled mod folder. On a change to a `content/**/*.json5` or `locale/*.json5` file it re-parses that file, recomputes the affected Content IDs through the full merge chain, re-validates, and swaps the **Registry** entries in place. **Handles** stay valid.
- Numeric and enum changes apply from the next tick. Structural changes (new Content IDs, changed `formations` lists, new sprite sets) apply at the next battle or campaign load; the log says which.
- A validation error during hot reload keeps the previous value, shows the diagnostic in the in-game console, and does not stop the game.
- Lua: a change to any file under `scripts_root` destroys and recreates that mod's Lua state at the next turn boundary, re-running `main.lua`. `il.state` is preserved.
- Hot reload never applies to `mod.json5`; manifest changes require a restart.
- Release builds ignore file changes entirely.

---

## 10. Distribution and packaging

Satisfies REQ-MOD-010.

- A mod ships as a folder or as a zip. A zip must contain exactly one top-level folder whose name is the mod `id`, with `mod.json5` directly inside it:

```
mymod-1.2.0.zip
  mymod/
    mod.json5
    content/...
    locale/...
    scripts/...
    assets/...
```

- Zips are read in place (no extraction). Hot reload does not work for zipped mods.
- The recommended zip file name is `<id>-<version>.zip`. The engine ignores the file name and reads the manifest.
- Assets are referenced by path relative to `assets_root`; absolute paths and `..` segments are rejected at load.
- Maximum sizes: a single JSON5 file 16 MiB; a mod 4 GiB; anything larger fails with a clear message.
- Steam Workshop and mod.io are out of scope. A future distribution layer would only add mod folders to the scan list in §3.1 and must not change anything in this document.

---

## 11. SDK versioning and deprecation policy

- **Schema version.** Every schema under `schemas/` carries `"x-schema-version": N` and the engine's `schema_version` (also written to saves, §8) increments whenever any content or save struct changes shape. Content files do not declare a schema version; the engine's version applies.
- **Compatibility.** Within one engine MINOR series (e.g. `0.5.x`), content that validated once keeps validating. Fields are only added, never removed or retyped.
- **Deprecation.** A field to be removed is first marked `"deprecated": "<message>"` in the schema. Using it produces a warning at load naming the file, line, and replacement, for at least two engine MINOR versions. After that it becomes an error. The flagship game's content is kept warning-free so it serves as the reference.
- **Renames of Content IDs** in the flagship game follow the same rule with a `deprecated` alias item kept for two MINOR versions.
- **Lua API.** Functions and event fields follow the same two-MINOR-version deprecation, with `il.log.warn` emitted on first use per session.
- **Engine version ranges.** Mods should declare `engine_version` as `>=X.Y.0 <X.(Y+2).0` to match the guarantee above.
- **Changelog.** Every engine release ships `docs/modding-changelog.md` listing added fields, deprecations, and removals by schema version.
