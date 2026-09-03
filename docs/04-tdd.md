# Iron Legion Engine — Technical Design Document

| | |
|---|---|
| **Version** | 0.1 |
| **Status** | Draft for review |
| **Upstream** | [PRD v0.2](01-prd.md) · [SAD](02-sad.md) · [Simulation Spec](03-simulation-spec.md) · [Glossary](00-glossary.md) |
| **Siblings** | [Networking Spec](05-networking-spec.md) · [Modding SDK](06-modding-sdk-spec.md) |

## How to read this document

Each subsystem section has the same shape: responsibilities, public API (Rust signatures, abbreviated), ECS components and resources, systems and their stage, data schema pointers, per-tick budget at 20,000 soldiers (REQ-PERF-005), and tests. Signatures are the intended shape, not final code; names are binding, argument lists may grow.

Stage numbers refer to SAD §6.2. Rule IDs refer to the Simulation Spec. Field names match the Modding SDK schemas.

Budget table (sum must fit 50 ms at P3, 25 ms at P2):

| Stage | Budget at 20k (ms) | Section |
|---|---|---|
| 0 ApplyCommands | 0.2 | §4 |
| 1 AI | 2.0 | §8.5, §9 |
| 2 Formation | 2.0 | §7 |
| 3 RegimentMovement | 1.0 | §6 |
| 4 SoldierSteering | 8.0 | §6 |
| 5 Integrate | 0.5 | §6 |
| 6 SpatialGrid | 2.0 | §5 |
| 7 Collision | 8.0 | §6 |
| 8 Visibility | 1.0 | §8.4 |
| 9 Targeting | 4.0 | §8.1 |
| 10 Combat | 4.0 | §8.1 |
| 11 Projectiles | 3.0 | §8.2 |
| 12 Abilities | 0.5 | §8.3 |
| 13 Fatigue | 0.5 | §8.3 |
| 14 Morale | 1.0 | §8.3 |
| 15 Death | 1.0 | §8.1 |
| 16 BattleFlow | 0.3 | §4 |
| 17 Events + Hash | 3.0 | §2, §4 |
| **Total** | **42.0** | headroom 8 ms |

---

## 1. Workspace, crates, dependencies

### 1.1 Layout

As in SAD §5.1. Root `Cargo.toml`:

```toml
[workspace]
members = ["crates/*", "game/rules", "tests", "benches"]
resolver = "3"

[workspace.package]
edition = "2024"
rust-version = "1.95"          # MSRV (bevy_ecs 0.19 needs 1.95); toolchain pinned to 1.98.0 in rust-toolchain.toml; bumped only at phase boundaries

[workspace.lints.clippy]
float_arithmetic = "deny"       # allowed only inside il_core::scalar; `S` is a newtype so the lint bites (§2)
# (workspace clippy.toml bans Instant::now, SystemTime::now, std::fs and HashMap/HashSet in sim crates;
#  il_data, il_cli, il_app, tests and benches carry a local clippy.toml that keeps only the wall-clock bans;
#  f32::mul_add is banned everywhere in favour of Scalar::mul_add_rounded)

[profile.dev]
opt-level = 1                   # the 10,000-tick determinism test runs under cargo test
[profile.dev.package."*"]
opt-level = 3

[profile.release]
codegen-units = 1
lto = "thin"
# never: -C target-cpu=native, never fast-math (Rust has none by default; keep it that way)
```

Dependency rules are enforced by `tests/tests/dep_rules.rs`, which parses every `crates/il_*/Cargo.toml`; cargo-deny is not used (T0-002).

Feature flags:

| Flag | Crate | Effect |
|---|---|---|
| `dev` | il_app, il_data | hot reload, debug overlays, per-tick hashing |
| `trace` | all | `tracing` spans per system |
| `headless` | il_cli | no render/ui crates linked |
| `fixed` | il_core | `Scalar = Fixed32` (Phase 7) |

### 1.2 Dependencies (pinned per phase)

Phase 0 pins (T0-003) are the versions in the table; later phases pin their own crates when they arrive and update this table. Phase 1 pins (T1-050) are the newest mutually compatible set on 2026-09-03; `egui-wgpu` 0.36 requires `wgpu` ^30 and `egui-winit` 0.36 requires `winit` ^0.30.13.

| Crate | Version (initial) | Why | Used by |
|---|---|---|---|
| `bevy_ecs` | 0.19.1 (feature `multi_threaded`) | standalone ECS with schedules and parallel executor | il_sim_*, il_render (read) |
| `bevy_tasks` | 0.19.1 | `ComputeTaskPool` for `BattleWorld::set_threads` | il_sim_battle |
| `wgpu` | 30.0.1 | GPU API | il_render |
| `winit` | 0.30.13 | window and input events | il_app, il_ui (event types) |
| `egui`, `egui-wgpu`, `egui-winit` | 0.36.1 | UI (`egui-wgpu` paint pass lives in il_render) | il_ui, il_render |
| `serde`, `serde_derive` | 1 | serialisation | all |
| `json5` | 1.3 | scenario and frame-table parsing only; content goes through `il_data::json5`, a span-carrying parser written in T1-020 because per-field positions are needed for diagnostics and merge provenance (OQ-7 amended) | il_cli, il_render, il_app |
| `semver` | 1 | manifest versions and ranges | il_data |
| `serde_json` | 1 | save headers, schema validation input | il_data, il_save |
| `jsonschema` | 0.53 (`default-features = false`) | content validation, draft 2020-12 | il_data |
| `postcard` | 1.1 | snapshot encoding (OQ-2 resolved in Phase 0) | il_save, il_sim_* |
| `mlua` (`lua54`, `vendored`) | 0.10 | Lua | il_script |
| `glam` | 0.33.6 | render-side math only (never in sim) | il_render, il_ui |
| `png`, `bytemuck`, `pollster` | 0.18.1 / 1 / 0.4 | atlas files, GPU buffer casts, blocking on device creation | il_render, il_cli (`genart`) |
| `rayon` | 1 | not used directly; bevy_ecs task pool only | — |
| `xxhash-rust` (`xxh3`) | 0.8 | state hash | il_core |
| `tracing`, `tracing-subscriber` | 0.1 / 0.3 | spans | all |
| `criterion` | 0.8.2 | benchmarks (`benches/benches/*.rs`, `harness = false`; first bench in T1-031) | benches |
| `kira` | 0.9 | audio (OQ-8: chosen for game-oriented mixing) | il_audio |
| `notify` | 6 | hot reload file watcher (dev) | il_data |
| `thiserror`, `anyhow` | 2 / 1 | errors (anyhow only in binaries and their libs) | all |
| `clap` | 4 (`derive`) | command-line parsing | il_cli, il_app |
| `toml` | 0.8 | manifest parsing in the dependency-rule test | tests |

## 2. Core (`il_core`)

### 2.1 Responsibilities

Stable ids, `Scalar`, vector and angle math, deterministic hashing and RNG, tick and turn types, event base. Satisfies REQ-TECH-009, REQ-TECH-010, REQ-SIM-004, REQ-SIM-005.

### 2.2 Public API

```rust
// ids.rs — stable, monotonic within a battle/campaign, never reused
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SoldierId(pub u32);
pub struct RegimentId(pub u32);
pub struct ArmyId(pub u16);
pub struct FactionId(pub u8);
pub struct PlayerId(pub u8);         // 0..=7 humans/AIs; 255 = engine (AI-internal per Networking Spec)
pub struct ProjectileId(pub u32);
pub struct ProvinceId(pub u16);

pub struct IdAllocator<T> { next: u32, _m: PhantomData<T> }   // serialised in snapshots

// time.rs
pub struct Tick(pub u32);            // 20 Hz; wraps never (2^32 ticks = 6.8 years)
pub struct Turn(pub u32);
pub const TICK_SECONDS: f32 = 0.05;  // the only f32 constant allowed outside scalar.rs (app-side accumulator only)
pub const TICKS_PER_SECOND: u32 = 20;

// scalar.rs
pub trait Scalar:
    Copy + PartialOrd + Add<Output=Self> + Sub<Output=Self> + Mul<Output=Self> + Div<Output=Self>
    + Neg<Output=Self> + Default + Serialize + DeserializeOwned + Hashable + 'static
{
    const ZERO: Self; const ONE: Self; const HALF: Self; const PI: Self; const TAU: Self;
    fn from_i32(v: i32) -> Self;
    fn from_f32_data(v: f32) -> Self;      // data loading only; never in tick code
    fn to_f32_render(self) -> f32;         // render only
    fn sqrt(self) -> Self;
    fn sin(self) -> Self; fn cos(self) -> Self; fn atan2(y: Self, x: Self) -> Self;
    fn abs(self) -> Self; fn min(self, o: Self) -> Self; fn max(self, o: Self) -> Self;
    fn clamp(self, lo: Self, hi: Self) -> Self;
    fn floor_i32(self) -> i32;
    fn mul_add_rounded(self, a: Self, b: Self) -> Self;  // a*b+self as two roundings; named so it cannot be shadowed by the fused inherent f32::mul_add (banned by clippy)
}
impl Scalar for f32 { /* sin/cos/atan2/sqrt via std; documented as platform-deterministic on one OS */ }
pub struct F32(f32);      // transparent newtype; delegates to the f32 impl; serde as a plain number; Hashable by bits
impl Scalar for F32 {}
pub struct Fixed32(i32);  // Phase 7; 16.16, table sin/cos, integer sqrt

pub type S = F32; // a newtype, not an alias: clippy's float_arithmetic sees through aliases, so `S = f32` would
                  // fire on every sim expression; the newtype keeps the lint on and makes a stray f32 a type error.
                  // Constants are written S::from_i32(n), S::HALF, S::ONE; content values enter via from_f32_data.
                  // Fixed32 replaces it behind feature `fixed`.

// vec.rs
#[derive(Copy, Clone, Default, Serialize, Deserialize)]
pub struct Vec2<T: Scalar> { pub x: T, pub y: T }
impl<T: Scalar> Vec2<T> {
    pub fn length(self) -> T; pub fn length_sq(self) -> T;
    pub fn normalized_or_zero(self) -> Self;
    pub fn clamp_length(self, max: T) -> Self;
    pub fn rotate(self, angle: T) -> Self;
    pub fn dot(self, o: Self) -> T; pub fn perp(self) -> Self;
}
pub type V2 = Vec2<S>;
pub struct Angle<T: Scalar>(T);  // radians, normalised to (-PI, PI]
impl<T: Scalar> Angle<T> { pub fn delta(self, to: Self) -> T; pub fn turn_toward(self, to: Self, max: T) -> Self; pub fn to_facing8(self) -> u8; }

// hash.rs
pub struct StateHasher(xxh3::Xxh3);
pub trait Hashable { fn hash_state(&self, h: &mut StateHasher); }
// Phase 0 decision (T0-012): no proc-macro crate. Structs use `impl_hashable_struct!(Ty { a, b })`, field-less enums
// `impl_hashable_fieldless_enum!(Ty)` (discriminant as u8); enums with payloads implement the trait by hand with a
// discriminant byte first. Option is tag byte + payload, slices are u32-length-prefixed, usize hashes as 64 bits.
// Revisit a derive macro when Phase 2 adds many components.
impl Hashable for f32 { fn hash_state(&self, h) { h.write_u32(self.to_bits()) } }
#[derive(Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateHash(pub u64);

// rng.rs — PCG32 with 64-bit state, plus stateless hash draws (SIM-DET-002)
pub struct RngStream { state: u64, inc: u64 }
impl RngStream {
    pub fn from_seed(seed: u64, stream_id: StreamId) -> Self;
    pub fn next_u32(&mut self) -> u32;
    pub fn unit<T: Scalar>(&mut self) -> T;                         // [0,1)
}
#[derive(Copy, Clone)] pub enum StreamId { CombatMelee, CombatRanged, Morale, AiRegiment, AiArmy, Abilities, Deployment, Weather, Campaign }
pub fn hash_draw<T: Scalar>(seed: u64, tick: Tick, entity: u32, index: u32) -> T; // xxh3 of tuple -> [0,1)

// events.rs
pub trait Event: Serialize + Clone {}
pub struct EventQueue<E: Event> { items: Vec<(Tick, E)> }  // ordered by insertion within a tick; systems push in stable order
```

### 2.3 Tests

- `Scalar` law tests (associativity is *not* assumed; tests check `mul_add` equals `a*b+c` bit-exactly on f32).
- `hash_draw` distribution test (chi-square over 1e6 draws) and stability test (golden values checked into the repo so a hash function change is caught).
- `RngStream` golden sequence test.
- `Angle::to_facing8` boundaries.
- Golden hash of a fixed struct (`hash.rs`) and golden empty-world hashes (`il_sim_battle`) so any hasher or layout change is caught.

Budget: hashing 32k soldiers × ~40 bytes ≈ 1.3 MB per tick through xxh3 ≈ 0.5 ms; full Stage 17 budget 3 ms includes event flush.

## 3. Data (`il_data`)

### 3.1 Responsibilities

Mod discovery, manifest parsing, load order, JSON5 parsing, schema validation, override/merge, registries, handles, localisation, content registry hash, hot reload. Satisfies REQ-VIS-004, REQ-MOD-001, 004..008, REQ-LOC-001, REQ-TEST-005.

### 3.2 Public API

```rust
pub struct ContentId(Arc<str>);              // "modid:item_id", interned
pub struct Handle<T> { index: u32, _m: PhantomData<T> }   // Copy, Hashable
pub struct Registry<T: ContentKind> { items: Vec<T>, by_id: HashMap<ContentId, u32>, ids: Vec<ContentId> }
impl<T: ContentKind> Registry<T> {
    pub fn get(&self, h: Handle<T>) -> &T;                  // infallible; handles are validated at load
    pub fn lookup(&self, id: &ContentId) -> Option<Handle<T>>;
    pub fn id_of(&self, h: Handle<T>) -> &ContentId;
    pub fn iter(&self) -> impl Iterator<Item=(Handle<T>, &T)>;   // ascending index = deterministic
}
pub trait ContentKind: DeserializeOwned + 'static { const DIR: &'static str; const TAG: KindTag; fn id(&self) -> &ContentId; fn resolve(&mut self, reg: &Registries) -> Result<(), ResolveError>; }  // TAG selects the embedded schema (T1-021); il_data::validate::validate_value maps every schema error back to the key's line and column

pub struct Registries {
    pub units: Registry<UnitType>, pub factions: Registry<Faction>, pub formations: Registry<FormationTemplate>,
    pub group_formations: Registry<GroupFormationTemplate>, pub abilities: Registry<Ability>,
    pub technologies: Registry<Technology>, pub buildings: Registry<Building>, pub maps: Registry<MapDef>,
    pub zones: Registry<ZoneType>, pub ai_profiles: Registry<AiProfile>, pub ai_actions: Registry<AiActionSet>,
    pub rules: Rules,                         // morale, fatigue, combat, movement, formation, general, visibility, battle_flow, ai, campaign, diplomacy
    pub locale: Locale, pub sprite_sets: Registry<SpriteSet>, pub sound_sets: Registry<SoundSet>,
    pub content_registry_hash: u64,                    // xxh3 over the typed sim-relevant fields (`ContentKind::hash_content`), kinds in a fixed order, items in ContentId order, references as ContentIds; independent of file order, whitespace, key order and registry layout (Networking Spec §4.2). As built in T1-023 the struct holds units, formations, group_formations, factions, zones, maps, sprite_sets, rules {movement, formation}, input (bindings), mods, mod_list_hash; the other registries arrive with their phases. References are resolved in two passes (ids registered first, then `resolve`), so file order never matters; Faction.ai_profile and tech_tree stay ContentIds until their kinds exist.
}

pub struct ModSet { pub mods: Vec<LoadedMod> }                // in resolved load order
pub struct LoadedMod { pub manifest: Manifest, pub root: PathBuf }
pub fn discover(roots: &[PathBuf]) -> Result<Vec<ManifestWithPath>, DataError>;
pub fn resolve_load_order(found: &[ManifestWithPath], enabled: &[String]) -> Result<ModSet, LoadOrderError>;
pub fn load(set: &ModSet) -> Result<Registries, Diagnostics>;  // collects ALL diagnostics before failing

pub struct Diagnostic { pub file: PathBuf, pub line: u32, pub col: u32, pub field: String, pub message: String, pub expected: Option<String> }
pub struct Diagnostics(pub Vec<Diagnostic>);

pub struct Locale { tables: BTreeMap<String /*lang*/, BTreeMap<String, String>>, current: String, show_keys: AtomicBool, missing: Mutex<BTreeSet<String>> }   // as built (T1-024): fallback is always "en"; misses are recorded once and logged with tracing::warn
impl Locale { pub fn get<'a>(&'a self, key: &'a str) -> &'a str;  /* current → en → the key itself */ pub fn fmt(&self, key: &str, args: &[(&str, &dyn Display)]) -> String; pub fn set_language(&mut self, lang: &str) -> bool; pub fn set_show_keys(&self, on: bool); pub fn missing_keys(&self) -> Vec<String>; }

#[cfg(feature = "hot-reload")]   // il_app enables it through its `dev` feature
pub struct HotReload { watcher: notify::RecommendedWatcher, rx: Receiver<notify::Result<notify::Event>>, set: ModSet, current: Arc<Registries>, dirty: Vec<PathBuf>, quiet_polls: u32, events: Vec<ReloadEvent> }
impl HotReload {
    pub fn new(set: ModSet, current: Arc<Registries>) -> notify::Result<Self>;   // watches every mod's content and locale folders
    pub fn poll(&mut self) -> Option<Arc<Registries>>;   // per frame; after ~100 ms of quiet re-runs the whole pipeline laid out like `current` (old ids keep their index, deleted ids stay as removed slots, new ids append); the app calls BattleWorld::replace_registries between ticks
    pub fn rebuild_now(&mut self) -> Option<Arc<Registries>>;
    pub fn take_events(&mut self) -> Vec<ReloadEvent>;   // Swapped { files } | Structural { added, removed } | Failed(Diagnostics) (old registries kept) | ManifestIgnored(path)
}  // replaces item in place by ContentId; index stable
```

### 3.3 Load pipeline

1. `discover`: read every `mod.json5` under the configured roots (`game/`, `mods/`, user mods dir).
2. `resolve_load_order`: Kahn topological sort over `dependencies`, `load_after`, `load_before`; ties by mod id ascending; cycle → error listing the cycle.
3. For each mod in order, for each `ContentKind::DIR`, parse every `*.json5` with `il_data::json5::parse_json5` into a `SpannedValue` (every key and value keeps `file:line:col`; `to_json()` gives the plain `serde_json::Value`). Per-file checks are limited to the object shape, the `id`, directive syntax and duplicate ids within the mod. Objects then merge into the kind's accumulating map keyed by ContentId (`il_data::merge`): `$from` copies an existing item of the same kind as the base (forward references inside a mod are applied first; depth <= 8; cycles are errors), then `$override`, `$delete` and list directives apply per Modding SDK §3.4.1; directives never survive into the map. A merged leaf keeps the key span of the mod that first wrote the field and takes the value span of the last writer. Validation runs on the **merged result only** (SDK §3.4.1 rule 4, decided in Phase 1): errors point at the original key and, when another mod wrote the value, add `after merge by "<mod>" (<file>:<line>:<col>)`. Merge fragments and `$delete` objects therefore never fail the `required` list.
4. Deserialise merged values into typed structs; call `resolve` to turn ContentIds into handles (two-pass: all ids registered first, then references resolved, so order between files does not matter).
5. Compute `content_registry_hash`.
6. Rules files: exactly one merged object per rules kind; missing fields fall back to engine defaults *only* for Could-tier fields; Must-tier fields missing are diagnostics.

Budget: load of the flagship game < 1 s; not per tick.

### 3.4 Tests

- Golden diagnostics for malformed files (file:line:col in the message).
- Load-order tests: diamond dependencies, cycles, `load_before` vs dependency conflict.
- Override tests: replace, deep merge, `$append`/`$remove`/`$replace`, `$delete`.
- `content_registry_hash` stability across file order and whitespace.

## 4. Battle simulation core (`il_sim_battle`)

### 4.1 Responsibilities

Owns the battle `World`, the stage schedule, Command application, Events, snapshot, hash, `BattleSetup` and `BattleResult`. Satisfies REQ-SIM-001..009, 020..036, 060..063, REQ-NET-001..003.

### 4.2 Public API

```rust
pub mod interface {
    pub struct BattleSetup { pub map_id: ContentId, pub seed: u64, pub weather: Weather, pub time_of_day: u8,
        pub time_limit_ticks: u32, pub reveal_deployment: bool, pub sides: Vec<SideSetup>, pub victory: VictoryRules }
    pub struct SideSetup { pub faction: ContentId, pub player: PlayerId, pub deployment_zone: u8,
        pub general: GeneralSetup, pub regiments: Vec<RegimentSetup>, pub reinforcements: Vec<ReinforcementGroup> }
    pub struct RegimentSetup { pub id: u32 /* campaign regiment id, echoed in result */, pub unit_type: ContentId,
        pub count: u16, pub experience: u8, pub fatigue: f32 /* data-side f32, converted */, pub formation: Option<ContentId>,
        pub position: Option<[f32; 2]>, pub facing_deg: Option<f32> /* TEMPORARY Phase 0 anchor (SAD T-7); removed in T2-070 */ }
    // `map_id` is required (T1-030); `BattleWorld::new` fails with `SetupError::UnknownMap`, `MissingDeploymentZone` or `PositionOutOfMap`.
    pub struct BattleResult { pub winner: Option<u8>, pub duration_ticks: u32, pub sides: Vec<SideResult>, pub summary: BattleSummary }
    pub struct SideResult { pub regiments: Vec<RegimentResult>, pub general_fate: GeneralFate, pub loot: i64 }
    pub struct RegimentResult { pub id: u32, pub initial: u16, pub survivors: u16, pub fled: u16, pub killed: u16, pub experience_gain: u16, pub ammo_left: u16 }
    pub enum GeneralFate { Alive, Wounded, Dead, Captured }
}

pub struct BattleWorld { world: bevy_ecs::World, schedule: Schedule, tick: Tick, phase: BattlePhase }
impl BattleWorld {
    pub fn new(setup: &BattleSetup, regs: Arc<Registries>) -> Result<Self, SetupError>;   // validates SIM-FLOW-019; the world keeps the Arc
    pub fn step(&mut self, commands: &[Command]) -> StepOutput;   // exactly one tick: simulates tick() + 1; commands must be stamped with that tick
    pub fn tick(&self) -> Tick;                                    // completed ticks; the app gathers commands for tick() + 1 (§15)
    pub fn phase(&self) -> BattlePhase;
    pub fn snapshot(&self) -> Snapshot;                            // postcard bytes of all Hashable+Serialize components and resources
    pub fn restore(snapshot: &Snapshot, regs: Arc<Registries>) -> Result<Self, RestoreError>;  // rebuilds derived data (paths, flow fields, grid)
    pub fn hash(&self) -> StateHash;                               // same value as StepOutput.hash of the last step (or of the initial state)
    pub fn result(&self) -> Option<BattleResult>;                  // Some once phase == Ended
    pub fn view(&self) -> BattleView<'_>;                          // read-only accessors for render/ui/ai (T1-052): tick, phase, regs, sides, soldiers()/soldiers_unordered()/soldier(id) -> SoldierRow, regiments()/regiment(id) -> RegimentRow; cached QueryStates refreshed by step/new/restore/recompute_hash
    pub fn set_threads(&mut self, n: usize);                       // n <= 1: SingleThreadedExecutor; else MultiThreadedExecutor on the process-global
                                                                   // ComputeTaskPool (sized by the first such call). Determinism test runs 1 and 8.
    pub fn ecs_mut(&mut self) -> &mut World;                       // tests and tools only; call recompute_hash() afterwards
}
pub struct StepOutput { pub hash: StateHash, pub events: Vec<BattleEvent>, pub rejected: Vec<(Command, RejectReason)> }

#[derive(Clone, Serialize, Deserialize, Hashable)]
pub struct Command { pub tick: Tick, pub player: PlayerId, pub seq: u16, pub kind: CommandKind }
pub enum CommandKind {
    Move { regiments: Vec<RegimentId>, target: V2, facing: Option<Angle<S>>, speed: SpeedMode },
    AttackRegiment { regiments: Vec<RegimentId>, target: RegimentId },
    AttackMove { regiments: Vec<RegimentId>, target: V2 },
    Halt { regiments: Vec<RegimentId> },
    // Content references in commands are ContentIds, not handles: a command stream must be self-describing in replays
    // and on the wire, and handles are not serialised. Stage 0 resolves them against the registries.
    SetFormation { regiments: Vec<RegimentId>, template: ContentId, ranks: Option<u8> },
    SetFacing { regiments: Vec<RegimentId>, facing: Angle<S> },
    SetSpeedMode { regiments: Vec<RegimentId>, mode: SpeedMode },
    GroupFormation { regiments: Vec<RegimentId>, template: ContentId, anchor: V2, facing: Angle<S>, width: S },
    FireMode { regiments: Vec<RegimentId>, mode: FireMode },
    UseAbility { regiment: RegimentId, ability: ContentId, target: AbilityTarget },
    Withdraw { regiments: Vec<RegimentId> },
    Deploy { regiment: RegimentId, position: V2, facing: Angle<S>, template: Option<ContentId> },
    ConfirmDeployment,
    Pause, SetSpeed { mult_x100: u16 },
    Surrender,
    TransferControl { from: PlayerId, to: PlayerId },   // Networking Spec §9: drop-to-AI
}
pub enum BattleEvent {
    SoldierDied { id: SoldierId, regiment: RegimentId, killer: Option<SoldierId>, pos: V2 },
    VolleyFired { regiment: RegimentId, count: u16 }, ProjectileLanded { pos: V2, hit: bool },
    Charge { regiment: RegimentId, target: RegimentId }, Engaged { regiment: RegimentId },
    MoraleState { regiment: RegimentId, from: MoraleState, to: MoraleState },
    Rallied { regiment: RegimentId }, Shattered { regiment: RegimentId }, GeneralDied { army: ArmyId },
    AbilityUsed { regiment: RegimentId, ability: Handle<Ability> }, PhaseChanged { from: BattlePhase, to: BattlePhase },
    CommandRejected { command_seq: u16, player: PlayerId, reason: RejectReason }, ReinforcementsArrived { side: u8 },
    Ended { result: Box<BattleResult> },
}
pub enum BattlePhase { Deployment, Battle, Pursuit, Ended }
pub enum SpeedMode { Walk, Run, March }
pub enum FireMode { FireAtWill, Hold, Target(RegimentId) }
```

### 4.3 Components (soldier-level, SoA via bevy_ecs tables)

| Component | Fields | Hashed | Interpolated |
|---|---|---|---|
| `Soldier` | `id: SoldierId, regiment: RegimentId, unit: Handle<UnitType>, category: UnitCategory` | id only | — |
| `Pos` | `p: V2` | yes | yes (`PrevPos` written at Stage 17) |
| `Vel` | `v: V2` | yes | — |
| `Facing` | `theta: Angle<S>` | yes | yes (`PrevFacing`) |
| `Body` | `r: S, m: S` | no (derived) | — |
| `Health` | `hp: S` | yes | — |
| `FatigueC` | `f: S` | yes | — |
| `SlotRef` | `slot: Option<u16>` | yes | — |
| `Fsm` | `state: SoldierState, since: Tick` | yes | — |
| `MeleeState` | `target: Option<SoldierId>, cooldown: u16, attackers: u8` | yes | — |
| `RangedState` | `ammo: u16, cooldown: u16` | yes | — |
| `Rank` | `rank: u8, file: u8` | no | — |
| `GeneralTag` | marker + `rank: u8` | — | — |
| `Dead` | marker, removed at Stage 15 | — | — |

Regiment-level components live on regiment entities (≈ 200): `Regiment { id, army, faction, units: SmallVec<[Handle<UnitType>;2]>, soldiers: Vec<SoldierId> (ascending) }`, `Anchor { pos: V2, facing: Angle<S> }`, `FormationState { template, ranks, files, slots: Vec<Slot>, integrity: S, morph_until: Tick }`, `Order { kind: OrderKind, target: V2, facing: Option<Angle<S>>, speed: SpeedMode, since: Tick }`, `Path { waypoints: Vec<Waypoint { p: V2, corridor: S }>, next: u16, requested: bool }` (stored and snapshotted, T1-032), `Morale { m: S, state: MoraleState, rout_count: u8, deaths_5s: RingBuffer<u16, 100>, engaged_since: Option<Tick>, initial: u16 }`, `Fire { mode, target: Option<RegimentId>, retarget_at: Tick }`, `Energy { e: S }`, `Statuses { list: SmallVec<[StatusEffect; 4]> }`, `Cooldowns { list: SmallVec<[(Handle<Ability>, u16); 4]> }`, `Experience(u8)`, `Visible { by_faction: u8 /* bitmask */ }`.

Projectiles: `Projectile { id, shooter_regiment, side, launch_tick, land_tick, start: V2, end: V2, apex: S, kind, damage, pen }`, `Pos`, plus a `ProjectilePool` resource of free entities (REQ-PERF-008).

### 4.4 Resources

`Clock { tick }`, `Phase`, `Sides: Vec<SideState>` (deployment confirmed, defeated, edge, flow field handle), `MapRes(Arc<LoadedMap>)` (heightmap, zone raster, river flags, deployment polygons, inert walls and gates; built by `new`/`restore` from `map_id`, `BattleWorld::empty` holds a flat placeholder `engine:flat`), `SpatialGrid`, `NavGrid`, `HpaGraph`, `FlowFields`, `CommandQueue`, `Rng { streams: [RngStream; 9] }`, `Ids { soldiers, regiments, projectiles: IdAllocator }`, `Events: EventQueue<BattleEvent>`, `Rules: Arc<Rules>`, `Regs: Arc<Registries>`, `Weather`, `Timer`, `PendingDamage: Vec<(Tick, SoldierId, S, Angle<S>)>` (SIM-PROJ-008), `ThreadCount`.

### 4.5 Schedule

One `Schedule` per stage, run in `Stage::ALL` order by `step` (T1-060; stages were already totally ordered, so nothing is lost and each stage can be timed through `StageObserver`), with one `SystemSet` per stage inside it. Within a set, systems are added with explicit `.after()` where order matters; otherwise bevy_ecs may parallelise. Systems that must be single-threaded for determinism are marked `.run_if(always)` and take `&mut World` exclusively (the apply steps).

Stage 0 `apply_commands`: sort incoming by `(player, seq)`, validate per SIM-CMD-003/004, mutate `Order`, `FormationState`, `Fire`, `Sides`; push `CommandRejected` events.
Stage 16 `battle_flow`: SIM-FLOW-011..017.
Stage 17 `flush_events_and_hash`: copy `Pos→PrevPos`, `Facing→PrevFacing`; hash per SIM-DET-004 in ascending id (iterate a sorted `Vec<Entity>` maintained by the `Ids` resource); drain events.

### 4.6 Snapshot

`Snapshot { version: u32, tick, phase, setup: BattleSetup, ids, rng, sides, regiments: Vec<RegimentSnap>, soldiers: Vec<SoldierSnap>, projectiles: Vec<ProjectileSnap>, pending_damage, timer }` encoded with postcard (`SNAPSHOT_VERSION = 2` since T1-030, when `map_id` became required; Phase 0 snapshots are not migrated). Phase 0 layout: `RegimentSnap { id, side, setup_id, unit_type: ContentId, anchor_pos, anchor_facing, morale, morale_state, order, ammo }`, `SoldierSnap { id, regiment, p, v, facing, hp, fatigue, slot, fsm_state, fsm_since }`, `IdsSnap { soldiers_next, regiments_next, projectiles_next }`; `PrevPos`/`PrevFacing` and `Body` are rebuilt on restore. `restore` rebuilds: `SpatialGrid` (from positions), `NavGrid`/`HpaGraph`/`FlowFields` (from map plus gate states), `Path` (re-requested), `Rank` (from slots). Snapshot of 32k soldiers ≈ 32k × 36 B ≈ 1.2 MB.

### 4.7 Tests

- `step` with no commands on an empty world advances tick and hash changes only by tick.
- Command validation matrix (ownership, phase, routing).
- Snapshot round trip: hash(restore(snapshot(w))) == hash(w) and continues identically for 1,000 ticks.
- Threads 1 vs 8 hash equality on the full scenario set.

## 5. Spatial grid (`il_sim_battle::spatial`)

```rust
// As built (T1-031): generic over the stable id so the same type indexes soldiers and regiment anchors.
pub struct Entry<Id> { pub id: Id, pub entity: Entity, pub pos: V2 }
pub struct SpatialGrid<Id> { cell: S, cols: u32, rows: u32, heads: Vec<u32> /* per cell, first index */, next: Vec<u32> /* per entry */, entries: Vec<Entry<Id>> /* ascending id */ }
impl<Id: Copy + Ord> SpatialGrid<Id> {
    pub fn new(width: S, height: S, cell: S) -> Self;                 // cols/rows = ceil(extent / cell); a non-positive cell means one cell
    pub fn ensure(&mut self, width: S, height: S, cell: S) -> bool;   // re-dimensions when the map or the rules changed (hot reload)
    pub fn rebuild(&mut self, iter: impl IntoIterator<Item = Entry<Id>>);   // sorted by id, inserted back to front so every cell chain ascends → deterministic bucket order
    pub fn cell_entries(&self, cx: u32, cy: u32) -> impl Iterator<Item = usize>;   // indices into entries(), ascending id
    pub fn query_circle(&self, c: V2, r: S, out: &mut Vec<Entry<Id>>);         // ascending id (sorted after collection); query_circle_indices for the index form
    pub fn for_each_pair(&self, f: impl FnMut(usize, usize));       // i<j within same and neighbouring cells, each pair once (self, E, NE, N, NW), rows ascending
    pub fn for_each_pair_in_row(&self, cy: u32, f: impl FnMut(usize, usize));   // the pairs of one row, so rows can run in parallel into per-row buffers
    pub fn cell_of(&self, p: V2) -> (u32, u32);                      // clamped to the grid
}
pub fn rebuild_spatial_grids(/* Stage 6 system */);                 // SpatialGridRes (soldiers, movement.spatial_cell) and AnchorGridRes (anchors, movement.anchor_cell); also run by rebuild_derived
```

- Cell size `spatial.cell` = 4 m (about 10 soldier diameters); at 2 km × 2 km that is 250k cells, 1 MB of heads. Rebuilt every tick (Stage 6) rather than incrementally: 32k inserts ≈ 0.3 ms; a full rebuild is simpler to keep deterministic (ADR-013).
- Pair iteration for collision uses the half-neighbourhood pattern (self, E, NE, N, NW) so each pair is visited once; pairs are collected per cell into a buffer, sorted by `(i, j)` id, then processed. Parallel over cell rows with results in per-soldier push buffers, applied in id order (SAD §8).
- A second grid instance with `cell = 16 m` indexes regiment anchors for AI and visibility queries.

Tests: query results equal brute force on random layouts; pair enumeration is a permutation-invariant set.

## 6. Movement and pathfinding (`il_sim_battle::movement`, `::nav`)

### 6.1 Nav grid and paths

```rust
pub struct NavGrid { cell: S, cols: u32, rows: u32, cost: Vec<u16> /* 0 = impassable; else cost×100 */, passable_run_x: Vec<u8>, passable_run_y: Vec<u8> }
impl NavGrid { pub fn from_map(map: &LoadedMap, rules: &MovementRules) -> Self; pub fn update_gate(&mut self, gate: GateId, state: GateState) -> DirtyRect; }
pub struct HpaGraph { cluster: u32, gates: Vec<GateNode>, edges: Vec<(u32, u32, u32)>, cluster_of: Vec<u16> }
impl HpaGraph { pub fn build(nav: &NavGrid, cluster: u32) -> Self; pub fn repair(&mut self, nav: &NavGrid, dirty: DirtyRect); }
pub trait Pathfinder { fn find(&mut self, nav: &NavGrid, from: V2, to: V2, out: &mut Vec<V2>) -> PathResult; }
pub struct AStar { open: BinaryHeap<(Reverse<u32>, u32)>, g: Vec<u32>, came: Vec<u32>, closed_epoch: Vec<u32>, epoch: u32 }
pub struct Hpa { abstract_astar: AStar, refine: AStar, graph: HpaGraph }
pub fn string_pull(nav: &NavGrid, path: &mut Vec<V2>);
pub struct PathRequests { queue: BTreeSet<RegimentId> }   // served ascending, `movement.paths_per_tick` per tick (SIM-MOVE-005)
```

- A\* uses integer costs (octile × 100) so the heap order is deterministic regardless of `Scalar`. Ties in the heap are broken by node index.
- `Pathfinder` is a resource swapped by phase: `AStar` in Phase 1, `Hpa` from Phase 3 (REQ-PATH-002).
- As built (T1-032): `NavGrid::from_map(&LoadedMap, &Registries, &MovementRules)` marks a nav cell impassable when any zone cell whose centre lies in it is `passable: false` or a river cell without a `crossing` zone, and costs it the largest `move_cost × 100` of those zone cells (slope is not in the cost; `from_costs` builds test grids). Diagonal steps cost `ceil(cost × 141 / 100)` and never cut an impassable corner. `Pathfinder::find(nav, from, to, out) -> PathResult::{Found, NoPath, StartBlocked, GoalBlocked}`: blocked endpoints snap to the nearest passable cell within 8 rings (ties by smaller `(cy, cx)`), `out[0] == from`, the last point is `to` (or the snapped cell centre); `string_pull` is greedy farthest-visible over `segment_clear`, a supercover DDA that also tests both side cells at an exact corner crossing. `corridor_width_at(p)` = `min(passable_run_x, passable_run_y) × cell`; each `Waypoint { p, corridor }` stores it so the corridor morph (SIM-MOVE-004) compares against the regiment's current width at follow time instead of a baked flag. `serve_path_requests` (Stage 3, exclusive) pops up to `paths_per_tick` ids from `PathRequests` (a `BTreeSet<RegimentId>`, rebuilt on restore from `Path.requested`), writes `Path { waypoints, next: 1, requested: false }`, and on failure resets the order to Idle with a `PathNotFound` event. `dijkstra_cost` is the optimality oracle.

### 6.2 Systems

| System | Stage | Parallel | Rule IDs |
|---|---|---|---|
| `serve_path_requests` | 3 | no | SIM-MOVE-002/005 |
| `regiment_follow_path` (anchor move, wheel, cohesion, corridor column morph) | 3 | per regiment (independent) | SIM-MOVE-010..013, SIM-MOVE-004 |
| `soldier_steer` (seek/flow, separation via grid, avoidance) → writes `Vel` | 4 | par_iter over soldiers (reads previous tick grid) | SIM-MOVE-020..025, SIM-FLOW-002 |
| `integrate` | 5 | par_iter | `p += v × dt`; SIM-MOVE-042 clamp |
| `collision_resolve` | 7 | pair buffers per cell row → id-order apply, ×`collision_iterations` | SIM-MOVE-040..043 |
| `compute_flow_fields` | on demand (start, nav change) | no | SIM-FLOW-001/003 |

Note on Stage 4 reading the grid: steering at tick *t* uses the grid built at Stage 6 of tick *t−1* (positions of the previous tick). This is deterministic and avoids a second rebuild. Collision at Stage 7 uses the grid rebuilt at Stage 6 of the same tick.

Terrain sampling (as built, T1-030): `LoadedMap::from_def(&MapDef, zone_cell) -> Result<LoadedMap, MapError>`; `height_at(p) -> S` bilinear on a `Vec<S>` of `height_cols × height_rows` samples at the map's `height_cell` (positions outside the map read the nearest edge); `zone_at(p) -> Option<Handle<ZoneType>>` from a rasterised `Vec<u8>` at `movement.zone_cell` (2 m) indexing the map's `zone_handles` table (`[0]` = `base_zone`; `None` only on the flat placeholder map); `river_at(p) -> bool` from a capsule raster of the rivers; `in_bounds`, `clamp`, `zone_cell_of`, `deployment_polygon(side)`. Zone polygons are rasterised by scanline at cell centres (even-odd, half-open on the right), later polygons overriding earlier ones. The heightmap sidecar is read by the `il_data` pipeline (`HeightmapRef::samples`, from the `assets_root` of the mod that last wrote `heightmap.path`; a missing or mis-sized file is a load diagnostic), so the sim never touches the filesystem.

Budget: steering 8 ms at 20k (400 ns per soldier with 8 neighbours); collision 8 ms.

Tests: A\* optimality vs Dijkstra on random grids; HPA\* path cost within 10 % of A\*; string pulling never crosses impassable cells; collision conserves momentum-weighted centre for equal masses; steering unit tests for arrive damping.

## 7. Formations (`il_sim_battle::formation`)

```rust
pub struct FormationTemplate { pub id: ContentId, pub name_key: String, pub layout: Layout, pub default_ranks: u8, pub min_ranks: u8, pub max_ranks: u8,
    pub spacing_file: S, pub spacing_rank: S, pub role_zones: Vec<RoleZone>, pub morph_ticks: u16,
    pub integrity_bonus_attack: S, pub integrity_bonus_defence: S, pub speed_mult: S, pub custom_slots: Vec<V2>, pub min_files: u8, pub loose_mult: S, pub default_files_column: u8 }
pub enum Layout { Line, Column, Square, Wedge, Phalanx, Loose, Custom }
pub struct Slot { pub offset: V2, pub facing_offset: Angle<S>, pub rank: u8, pub file: u16 /* u16 since T1-040: a 2,000-man single rank */, pub category: Option<UnitCategory> }
pub trait LayoutFn { fn layout(&self, t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>); }
pub fn layout_for(layout: Layout) -> &'static dyn LayoutFn;   // SIM-FORM-003..009
// As built (T1-040): layout_slots(t, n, ranks, radius, out) dispatches on t.layout; effective_ranks(t, n, requested) clamps to [min_ranks, max_ranks] and to n;
// files_for(n, ranks) = ceil(n / ranks); spacing(t, radius) = (spacing_file, spacing_rank) × 2 radius; ranks_used / files_used read a table back.
// Column widens beyond default_files_column only if it would exceed 255 ranks; Wedge ignores `ranks`; Square uses `ranks` as the depth of each side.
pub fn assign_slots(soldiers: &[(SoldierId, V2, UnitCategory)], slots: &[Slot], anchor: &Anchor, grid: &SpatialGrid, rules: &FormationRules, prev: &[Option<u16>], out: &mut Vec<Option<u16>>);  // SIM-FORM-022
pub fn integrity(soldiers: &[V2], assigned: &[Option<u16>], slots_world: &[V2], radius: S) -> S;  // SIM-FORM-030
pub struct GroupFormationTemplate { pub id: ContentId, pub kind: GroupKind, pub gap: S, pub skirmishers_forward: bool, pub cavalry_flanks: bool, pub lines: u8 }
pub fn arrange_group(t: &GroupFormationTemplate, regiments: &[RegimentInfo], anchor: V2, facing: Angle<S>, width: S, rules: &FormationRules) -> Vec<(RegimentId, V2, Angle<S>, u8 /*ranks*/)>;  // SIM-FORM-040..042
```

Systems: `formation_layout` (Stage 2, per regiment with `needs_reform` flag; parallel over regiments, each writing only its own `FormationState`), `formation_integrity` (Stage 2, every `integrity_period_ticks`).

Assignment cost: greedy with grid candidates is O(n × k); swap passes O(n × files). Budget 2 ms for all reforming regiments; benchmark `assign_slots` at n = 500 must be < 0.5 ms (SIM-FORM-023).

Tests: layout functions produce `n` slots, centred front rank, no duplicates; assignment keeps slots within `keep_slot_radius`; group arrangement width within tolerance; golden slot tables for each layout at n ∈ {1, 7, 60, 160, 500}.

## 8. Combat, morale, fatigue, abilities, visibility, AI (`il_sim_battle::combat`, `::morale`, `::abilities`, `::visibility`, `::ai`)

### 8.1 Melee and death

```rust
pub struct CombatRules { pub base_hit: S, pub hit_scale: S, pub min_hit: S, pub max_hit: S, pub min_damage: S, pub engage_radius: S, pub retarget_period_ticks: u16, pub reach_slack: S,
    pub charge_window_ticks: u16, pub charge_dmg_share: S, pub charge_distance: S, pub charge_mass_mult: S, pub brace_integrity: S,
    pub flank_dmg_mult: S, pub rear_dmg_mult: S, pub flank_def_mult: S, pub rear_def_mult: S, pub height_defence: S, pub height_range: S, pub height_ref: S,
    pub second_rank_reach_bonus: S, pub exp_step: S, pub pursuit_hit_mult: S, pub pursue_repath_ticks: u16, pub corpse_ticks: u16,
    pub projectile_cap: u32, pub projectile_radius: S, pub scatter_scale: S, pub direct_apex: S, pub gravity: S, pub shield_mult: S, pub stat_hit_base: S, pub friendly_block_dist: S, pub volley: bool, pub ranged_retarget_ticks: u16 }

pub fn hit_probability(a: S, d: S, r: &CombatRules) -> S;                    // SIM-CMBT-011
pub fn melee_damage(dmg: S, armour: S, pen: S, mults: S, r: &CombatRules) -> S; // SIM-CMBT-013
pub fn attack_arc(defender_facing: Angle<S>, to_attacker: V2, frontal_arc: S) -> Arc;  // SIM-CMBT-014
pub struct AttackOutcome { attacker: SoldierId, target: SoldierId, hit: bool, damage: S, arc: Arc }
```

Systems: `melee_target` (Stage 9, staggered, par_iter reading grid, writes own `MeleeState`; `attackers` counts are recomputed in a single-threaded pass after), `melee_attack` (Stage 10, par_iter producing `AttackOutcome` into a per-thread buffer, then merged and sorted by attacker id, then applied to `Health`), `resolve_deaths` (Stage 15: soldiers with `hp ≤ 0` sorted by id → `Dead`, regiment soldier lists updated, `deaths_5s` ring, kill credit, events, `needs_reform`).

Budget: targeting 4 ms, combat 4 ms (only engaged soldiers do work; typical 3–6k engaged at P3).

Tests: `hit_probability` monotonic and clamped; arc classification golden cases; charge negation by braced anti-cavalry; scenario bands (Simulation Spec §15.3).

### 8.2 Ranged and projectiles

Systems: `ranged_target` (Stage 9, per regiment), `ranged_fire` (Stage 10: per soldier cooldown, aim prediction, scatter via `hash_draw`, spawn from `ProjectilePool` or statistical path when capped, SIM-PROJ-003..004/008), `projectile_advance` (Stage 11, par_iter: position along precomputed arc by `t = (tick − launch)/(land − launch)`), `projectile_land` (Stage 11: those with `land_tick == tick`, sorted by id; grid query; damage into `PendingDamage`; then `apply_pending_damage` applies entries whose tick has come, in `(tick, target id)` order).

Aim prediction: `aim = target.p + target.v × flight_ticks × dt`, flight ticks solved from `projectile_speed` and distance for direct, from the 45° ballistic formula for indirect, rounded up to integer ticks.

Budget: 3 ms at 8k live projectiles.

Tests: flight time golden values; landing hit selection equals nearest; statistical path expected casualties within 10 % of simulated over 50 seeds (T-3).

### 8.3 Morale, fatigue, abilities

```rust
pub struct MoraleRules { pub t_unsettled: S, pub t_shaken: S, pub t_broken: S, pub t_routing: S, pub hysteresis: S, pub rally_margin: S, pub rally_safe_radius: S,
    pub max_routs: u8, pub shatter_strength: S, pub general_death_shock: S, pub rout_shock: S, pub rout_shock_radius: S, pub disengage_penalty: S, pub charged_penalty: S,
    pub casualty_rate_ref: S, pub casualty_total_ref: S, pub fatigue_start: S, pub ally_radius: S, pub allies_ref: S, pub routing_ref: S, pub outnumber_ref: S,
    pub engage_fatigue_ticks: u32, pub safe_radius: S, pub exp_bonus: S, pub w: MoraleWeights, pub state_mults: [StateMults; 5] }
pub struct MoraleWeights { pub casualty_rate: S, pub casualty_total: S, pub fatigue: S, pub general_aura: S, pub allies_near: S, pub allies_routing: S, pub high_ground: S, pub fear: S, pub flanked: S, pub outnumbered: S, pub integrity: S, pub engaged_duration: S, pub winning: S, pub recovery: S }
pub fn morale_factors(ctx: &RegimentContext) -> [S; 14];        // x_f per SIM-MOR-010..024, order = MoraleWeights field order
pub fn morale_state(m: S, current: MoraleState, r: &MoraleRules) -> MoraleState;  // SIM-MOR-003 hysteresis

pub struct FatigueRules { pub rate_idle: S, pub rate_walk: S, pub rate_march: S, pub rate_run: S, pub rate_fighting: S, pub rate_routing: S, pub armour_rate: S,
    pub thresholds: [S; 3], pub speed_loss: S, pub attack_loss: S, pub defence_loss: S, pub interval_gain: S }
pub fn fatigue_mults(f: S, r: &FatigueRules) -> FatigueMults;   // SIM-FAT-004

pub struct Ability { pub id: ContentId, pub name_key: String, pub targeting: Targeting, pub radius: S, pub range: S, pub cooldown_ticks: u16, pub duration_ticks: u16, pub energy_cost: S,
    pub effects: Vec<Effect>, pub stacking: Stacking, pub requires_not_engaged: bool, pub requires_not_moving: bool }
pub enum Effect { Buff { stat: Stat, mult: S, add: S }, Debuff { stat: Stat, mult: S, add: S }, Damage { amount: S, armour_penetration: S, per_tick: bool },
    Heal { amount: S, per_tick: bool }, Summon { unit_type: Handle<UnitType>, count: u16, formation: Handle<FormationTemplate> }, Fear, Area { effects: Vec<Effect>, radius: S, duration_ticks: u16 }, Teleport { max_distance: S } }
pub struct StatusEffect { pub source: Handle<Ability>, pub remaining: u16, pub stacks: u8 }
pub fn status_mults(statuses: &[StatusEffect], regs: &Registries) -> StatMults;  // SIM-ABIL-005
```

Systems: `ability_tick` (Stage 12: cooldowns, energy regen, status expiry, per-tick effects, in regiment id order), `fatigue_tick` (Stage 13, par_iter), `regiment_fatigue_mean` (Stage 13, every 10 ticks), `morale_tick` (Stage 14: per regiment sequential in id order; uses the anchor grid for allies/enemies; applies one-time shocks queued by combat/death systems in a `MoraleShocks` resource; transitions and rout/rally/shatter per SIM-MOR-030..033).

Budget: morale 1 ms (200 regiments × grid queries), fatigue 0.5 ms, abilities 0.5 ms.

Tests: factor functions golden; hysteresis state machine table; rally requires safe radius; shatter conditions; stacking rules.

### 8.4 Visibility

`Visibility` resource: per faction a bitmask over regiment index plus last-seen positions. System `visibility_update` (Stage 8, one faction per tick round-robin per `visibility.period_ticks`; per regiment pair within `los_radius` sample the heightmap along the segment every `los_sample`). Budget 1 ms. `BattleView::visible_regiments(faction)` is the only accessor UI and AI use.

### 8.5 Battle AI (`il_ai` + `il_sim_battle::ai`)

```rust
// il_ai
pub struct Consideration { pub input: InputId, pub curve: Curve }
pub enum Curve { Linear { m: S, b: S }, Quadratic { k: S }, Logistic { k: S, mid: S }, Step { threshold: S } }
pub struct ActionDef { pub id: ContentId, pub base: S, pub considerations: Vec<Consideration>, pub noise: S }
pub struct AiActionSet { pub id: ContentId, pub scope: AiScope /*Army|Regiment*/, pub actions: Vec<ActionDef> }
pub trait InputProvider { fn input(&self, id: InputId) -> S; }  // implemented by RegimentContext / ArmyContext
pub fn select<'a>(set: &'a AiActionSet, inputs: &dyn InputProvider, rng: Option<(&mut RngStream, S)>) -> (&'a ActionDef, S);  // SIM-AI-001
pub struct AiProfile { pub id: ContentId, pub aggression: S, pub reserve_fraction: S, pub general_aggression: S, pub army_strength_target: S, pub composition: Vec<(UnitCategory, S)>, pub min_garrison: u8, pub action_sets: Vec<ContentId> }
```

`InputId` is a closed enum in `il_sim_battle::ai::inputs` (distance_to_nearest_enemy, strength_ratio, own_morale, is_flank_exposed, cavalry_approaching, enemy_ranged_in_range, friendly_in_line_of_fire, ...); data refers to them by name. Army plan (SIM-AI-010..014) is a resource per AI side; regiment actions (SIM-AI-021) map to Commands pushed into `CommandQueue` for `tick + 1` with `PlayerId(255)`-style internal tagging per Networking Spec §2.

Budget: 2 ms (staggered: ~10 regiments and ≤ 1 army per tick).

Tests: curve evaluation golden; selection is deterministic and tie-breaks by order; scenario: AI army beats a passive player army.

## 9. Campaign simulation (`il_sim_campaign`)

```rust
pub struct CampaignWorld { world: World, schedule: Schedule, turn: Turn, phase: TurnPhase }
impl CampaignWorld {
    pub fn new(start: &CampaignStart, regs: &Registries) -> Result<Self, SetupError>;
    pub fn apply(&mut self, commands: &[CampaignCommand]) -> CampaignOutput;      // during PlayerPhase / AIPhase
    pub fn end_turn(&mut self) -> CampaignOutput;                                   // runs AIPhase for all AI, Resolution, TurnEnd; may emit BattleRequested and stop
    pub fn resume_after_battle(&mut self, id: BattleId, result: BattleResult) -> CampaignOutput;
    pub fn snapshot(&self) -> Snapshot; pub fn restore(...); pub fn hash(&self) -> StateHash;
    pub fn view(&self) -> CampaignView<'_>;
}
pub enum CampaignCommand { MoveArmy { army: ArmyId, path: Vec<ProvinceId> }, Recruit { settlement: ProvinceId, unit: Handle<UnitType> }, Build { settlement: ProvinceId, building: Handle<Building> },
    Research { tech: Handle<Technology> }, Diplomacy { target: FactionId, action: DiplomacyAction, terms: Terms }, SetTax { province: ProvinceId, level: u8 },
    MergeArmies { into: ArmyId, from: ArmyId }, SplitArmy { army: ArmyId, regiments: Vec<u32> }, DisbandRegiment { army: ArmyId, regiment: u32 }, EndTurn,
    ApplyBattleResult { battle: BattleId, result: Box<BattleResult> }, AutoResolve { battle: BattleId } }
pub enum CampaignEvent { TurnStarted, TurnEnded, BattleRequested { id: BattleId, setup: Box<BattleSetup> }, ProvinceCaptured { province, by }, FactionDestroyed(FactionId),
    TreatySigned { a, b, kind }, WarDeclared { a, b }, TechResearched { faction, tech }, BuildingCompleted { province, building }, ArmyCreated(ArmyId), GeneralDied { faction, army }, RebellionSpawned(ProvinceId) }
```

Entities: `Faction { id, treasury: i64, research: Option<(Handle<Technology>, u16)>, known_techs: BitSet, personality: Handle<AiProfile>, player: Option<PlayerId> }`, `Province { id, owner, terrain: Handle<ZoneType>, resources, population: u32, tax_level: u8, public_order: i16, buildings: Vec<Handle<Building>>, construction: Option<(Handle<Building>, u16)>, recruiting: Vec<(Handle<UnitType>, u16)>, neighbours: Vec<Edge> }`, `Army { id, faction, province, general: Option<General>, regiments: Vec<CampaignRegiment>, movement_left: u16, path: Vec<ProvinceId>, state: ArmyState }`, `CampaignRegiment { id: u32, unit: Handle<UnitType>, count: u16, experience: u8, fatigue: S }`, `Relations { matrix: Vec<Relation> /* n×n */, attitude: Vec<S> }`.

Systems in `end_turn` (sequential, id order): `ai_phase` (SIM-CAMP-050/051 per faction), `move_armies` (SIM-CAMP-011, interception → `BattleRequested`, pause), `economy` (SIM-CAMP-020..023), `research`, `recruitment`, `replenish`, `diplomacy_update` (SIM-CAMP-031), `events` (Lua hooks Phase 6), `hash`, autosave trigger event.

Campaign pathfinding: Dijkstra on `Province.neighbours` with edge cost; graph ≤ 500 nodes.

Auto-resolve (SIM-CAMP-045): `il_app` constructs a `BattleWorld`, replaces both players by AI, steps until `Ended` or `autoresolve_max_ticks`, and feeds `ApplyBattleResult`. In `il_cli` the same path runs headless.

Budget: end_turn < 5 s with 30 factions (REQ-PERF-009); dominated by auto-resolves, which are bounded.

Tests: economy arithmetic golden; interception creates exactly one battle with reinforcements; `BattleResult` application table (survivors, general fates); campaign determinism over 100 AI turns with hash per turn.

## 10. Renderer (`il_render`)

### 10.1 Design

- **Projection.** World (x, y, h) → screen: isometric with fixed pitch. `screen = P × R(k × 90°) × (x, y)` plus `−h × pitch_scale` on screen y, where `k ∈ 0..4` is the snap rotation (OQ-1 resolved as 4 snaps for MVP; 8 as Could). Sprite facing index = `(facing8 − 2k) mod 8`, so 8 facing sets suffice for all snaps. As built (T1-052): `Camera { center: Vec2 (world), zoom (px/m, 2..96), rotation: u8 (0..=3, quarter turns clockwise), pitch (0.5), elevation (0.8) }`; world → view applies `R(−k·90°)`; `world_to_screen`, `screen_to_world`, `pan_screen`, `zoom_at` (keeps the point under the cursor fixed), `rotate`, `visible_bounds` (culling AABB).
- **Depth.** Painter's order by projected y (back to front), with instance sort per frame on the CPU (32k sort ≈ 1 ms) or by depth in a depth buffer using projected y as z; the latter is chosen (no CPU sort, alpha edges handled by alpha-to-coverage).
- **Instancing.** One draw per (atlas, LOD tier). Instance layout 32 bytes (as built in T1-051; wgpu has no scalar `f16` vertex format): `pos: [f32; 2]` (projected screen pixels), `depth: f32`, `frame_facing: u32` (atlas column in bits 0..16, facing row in bits 16..24), `tint: [u8; 4]`, `scale: f32`, `flags: u32` (bit 0 selected, bit 1 hovered), `reserved: u32`. 32k instances = 1 MB per frame, written with `queue.write_buffer` into a ring of 3 buffers. The colour target is 4× MSAA with alpha-to-coverage, resolved to the surface. Sprite sheets are `SpriteSet` content files (`content/sprites/*.json5`: atlas path, frame size, facings as rows, columns as frames, ground origin, named animations) over a PNG under `assets/`; `il_cli genart` generates the placeholder sheets.
- **Interpolation.** `p = lerp(prev, cur, alpha)`; facing snaps when the angle crosses a facing8 boundary (no angular lerp for sprites).
- **LOD.** `zoom < z1`: Detailed (full atlas frame, animation); `z1..z2`: Reduced (single frame per state, no animation); `> z2`: Aggregation — one quad per regiment rank block coloured by faction and shaded by density, computed from `FormationState` (REQ-RNDR-004).
- **Terrain.** As built (T1-053): `il_render::terrain::TerrainMesh::build(&LoadedMap, &Registries)` makes one vertex per height sample (`pos`, `height`, `shade` from the finite-difference normal under a fixed north-west light; 16 bytes) and two triangles per `height_cell` cell, plus an `R8Uint` zone-index raster at `zone_cell` (rows padded to 256 bytes; river cells without a `crossing` zone take slot 255 = water) and a 256-entry linear palette from `ZoneType.colour`. `terrain.wgsl` projects vertices with a 64-byte camera uniform that mirrors `Camera::world_to_screen`, writes depth 1.0 with no depth write so every sprite draws over it, and colours fragments from the palette times the shade with 2 m contour lines. Rivers and roads therefore come from the raster rather than separate strips; walls and gates as sprite strips arrive in Phase 5. `Renderer::set_terrain(&TerrainMesh)` uploads once per battle; `Renderer::render(&FrameScene { clear, camera, sprites, lines }, ui)` draws terrain, sprites and lines in one MSAA pass. Sprites take `height` from `LoadedMap::height_at` in `build_snapshot`. A line-list pipeline (`lines.rs`, `LineScene { vertices: Vec<LineVertex { pos, colour }> }`, screen-space, alpha-blended, no depth) draws the deployment outlines (`deployment_outlines`, ground-following, side tint) and serves the debug overlays (T1-054).
- **Debug overlays.** Line list pipeline fed from `BattleView` (nav grid, slots, paths, LOS radii, morale bars) toggled by `DebugFlags`.
- **Threading.** Phase 1: render on the main thread after the sim step from a `RenderSnapshot` (positions ×2, facings ×2, regiment blocks, projectiles, camera). Phase 3: the snapshot is sent over a channel to a render thread (REQ-RNDR-007); the snapshot type is designed now so only the plumbing changes (T-5).

```rust
pub struct Renderer { device, queue, surface, sprite_pipe, terrain_pipe, line_pipe, atlases: Vec<Atlas>, instance_ring: [Buffer; 3], camera: Camera }
pub struct Camera { pub center: V2f, pub zoom: f32, pub rotation: u8 /* 0..4 */, pub pitch_scale: f32 }
pub struct RenderSnapshot { pub tick: Tick, pub alpha: f32, pub soldiers: Vec<SoldierInst>, pub projectiles: Vec<ProjInst>, pub regiments: Vec<RegimentBlock>, pub fog: FogMask, pub debug: DebugLines }
impl Renderer { pub fn render(&mut self, colour: ClearColour, scene: &SpriteScene, ui: Option<&EguiPaint>) -> Result<(), RenderError>; }  // as built: the sprite pass (MSAA, resolved to the surface) then the egui-wgpu paint pass over it; `EguiPaint` borrows il_ui's tessellated `UiOutput`
pub fn build_snapshot(view: &BattleView, input: &SnapshotInput { alpha, camera, screen, selected }, out: &mut RenderSnapshot);  // as built (T1-052): clears and refills `out` (no per-frame allocation), lerps positions, snaps facing8, culls to camera bounds; `lod`, `flags` and `faction` join the input as their features land (T1-054, Phase 2/3)
pub fn scene_from_snapshot(snap: &RenderSnapshot, screen: Vec2, time: f32, categories: &[CategoryAtlas], out: &mut SpriteScene);  // projection, depth from projected ground y, facing remap, animation column, side tint
```

Budget: 32k instances at 60 FPS: snapshot build ≈ 1.5 ms, GPU ≈ 2 ms on the target GPU.

Tests: projection round trip; facing index under rotation; LOD tier selection; headless `wgpu` test with a software adapter renders one frame without panic (CI, best effort).

## 11. UI and input (`il_ui`)

- **Input mapping.** `Bindings` loaded from `content/input/bindings.json5` (REQ-INP-005): `{ action: "select_all", keys: ["Ctrl+A"] }`. `InputState` accumulates winit events per frame; `Gestures` produce `UiIntent`s: `Select(box|click)`, `OrderMove { target, facing, width }` from right-drag (drag vector defines facing perpendicular and width), `AttackMove`, `Halt`, `SetFormation`, `Ability`, `CameraPan/Zoom/Rotate`, `Pause`, `Speed`.
- **Selection model.** `Selection { regiments: BTreeSet<RegimentId>, groups: [BTreeSet<RegimentId>; 10] }`, only own faction, only visible.
- **Command emission.** `UiIntent → Command` with `tick = now + input_delay`, `seq` from a per-player counter. Drag-formation → `GroupFormation` if > 1 regiment else `Move { facing }` and `SetFormation { ranks }` derived per SIM-FORM-042.
- **Panels (egui).** Battle: regiment cards (top), command card (bottom), minimap with fog (bottom-right, rendered from `FogMask` and regiment blocks into an egui texture), clock and speed (top-right), casualties. Deployment: regiment tray, zone outline, confirm. Campaign: province, settlement, army, diplomacy, research, faction, turn log, end-turn. Menus: main, custom battle (map, sides, roster builder from registries), settings (bindings, audio, video), load/save (headers from `il_save`).
- **Localisation.** All labels via `Locale::get`; a debug toggle shows keys.

Budget: egui ≈ 1 ms per frame; minimap texture regenerated every 10 frames.

Tests: gesture geometry (drag vector → facing, width); binding parse; selection rules; snapshot tests of intent → command conversion.

## 12. Audio (`il_audio`)

- `AudioEngine` wraps kira. `SoundSet` content maps `BattleEvent` kinds and unit types to sample lists; `EventRouter` consumes `StepOutput.events` and the camera state each frame.
- Zoom mixing (REQ-AUD-002): per-frame, events are bucketed by distance to the camera centre; near zoom plays individual `SoldierDied`/`Charge` samples (rate-limited to `audio.max_voices`), far zoom drives a continuous "battle roar" loop whose gain = `sat(engaged_soldiers / audio.roar_ref)`.
- Music: `MusicState { campaign, battle_calm, battle_intense, victory, defeat }` with crossfades.

Budget: < 0.5 ms per frame. Tests: router rate limiting; gain curves.

## 13. Scripting (`il_script`) — Phase 6

```rust
pub struct ScriptHost { lua: Lua, handlers: BTreeMap<EventName, Vec<RegistryKey>>, pending: Vec<CampaignCommand> }
impl ScriptHost {
    pub fn new(regs: &Registries, set: &ModSet) -> Result<Self, ScriptError>;   // builds sandbox: strips io/os/package/require/debug; installs `il` table
    pub fn dispatch(&mut self, ev: &CampaignEvent, view: &CampaignView) -> Vec<CampaignCommand>;  // handlers may only queue Commands
}
```

- Sandbox: `il.rng` seeded from `(campaign_seed, turn)`; `os.time`, `os.clock`, `math.random` removed; instruction budget per handler via `set_hook` (`script.max_instructions` = 1e6) then error.
- Scripts run in the `events` step of `end_turn` in mod load order then handler registration order; their output is Commands applied in that same order, so the campaign hash covers script effects.
- Battle: no Lua (REQ-MOD-003). `battle_start`/`battle_end` hooks run in the campaign around the battle.

Tests: sandbox escape attempts fail; instruction budget triggers; handler order determinism.

## 14. Save and replay (`il_save`)

```rust
pub struct SaveHeader { pub engine_version: String, pub schema_version: u32, pub kind: SaveKind /*Campaign|Battle|Replay*/, pub mods: Vec<(String, String)>, pub content_registry_hash: u64, pub created: String, pub turn: Option<u32>, pub tick: Option<u32>, pub summary: String }
pub struct SaveFile { pub header: SaveHeader, pub body: Vec<u8> /* postcard, zstd-compressed */ }
pub fn write(path: &Path, header: &SaveHeader, body: &[u8]) -> io::Result<()>;   // "ILSV" magic, u32 header length, JSON header, body
pub fn read_header(path: &Path) -> io::Result<SaveHeader>;
pub fn read(path: &Path) -> io::Result<SaveFile>;
pub trait Migrate { fn migrate(from: u32, body: Vec<u8>) -> Result<Vec<u8>, MigrateError>; }   // chain of version steps, one function per bump
pub struct Replay { pub setup: BattleSetup, pub commands: Vec<Command>, pub checkpoints: Vec<(Tick, Vec<u8>)>, pub hashes: Vec<(Tick, StateHash)> }
```

- Campaign save = header + campaign snapshot + `script_state: Option<String>` (the Lua `il.state` table as JSON, Modding SDK §5) + (if mid-battle) battle snapshot and the pending `BattleId`.
- On load: registries rebuilt from the header's mod list (REQ-SAVE-004: missing required mod → refuse; different versions → warn); handles inside snapshots are stored as ContentIds and re-resolved during restore. A unit ContentId that no longer exists resolves to the engine placeholder `il:missing_unit` and is flagged in the UI; missing buildings and technologies are dropped; a missing faction aborts the load (Modding SDK §8).
- Replay recording is on by default in battles (`Replay` appended per tick, checkpoint every 1,200 ticks); `il_cli replay --verify` re-simulates and compares hashes.

Tests: header round trip; migration chain; mod mismatch policies; replay verify on the scenario set.

## 15. App shell (`il_app`)

```rust
enum AppState { MainMenu, Campaign(CampaignSession), Battle(BattleSession), Editor(EditorSession) }
struct BattleSession { world: BattleWorld, accumulator: f64, speed: f32, paused: bool, input_delay: u16, local_player: PlayerId, pending: Vec<Command>, replay: Replay, net: Option<LockstepSession> }
```

Frame: poll winit → `il_ui` intents → commands stamped `tick + input_delay` into `pending` → `accumulator += dt × speed` (capped at `app.max_catchup_ticks` = 4 ticks) → while `accumulator ≥ TICK`: gather commands for `world.tick()+1` (local pending, AI internal, network) → `step` → route events to audio/UI/replay → `accumulator −= TICK` → build `RenderSnapshot(alpha)` → render → egui. Campaign state runs `apply` on intents and `end_turn` on End Turn; `BattleRequested` switches state; `Ended` returns the result via `resume_after_battle` or the auto-resolve path.

Tests: accumulator never runs more than the cap; pause records a `Pause` command; state transitions.

## 16. Editors (`il_editor`)

- **Map editor (Phase 3).** Operates on `MapDef` (`id`, `name_key`, `size`, `campaign_terrain_tags`, `weather_allowed`, heightmap `Vec<f32>` at `height_cell` stored as a 16-bit raw sidecar, `base_zone`, `zones` polygons (fords and bridges are polygons of a `crossing: true` zone type laid over a river), `rivers`, roads, `deployment` polygons, `reinforcement_edges`, and the reserved `structures` and `siege_points` lists; Modding SDK §6.1 shows the JSON5). Tools: raise/lower/smooth height brush, zone paint brush, polyline tool, polygon tool, piece placement; live nav grid preview; save to `content/maps/<id>.json5` plus the `.hgt` sidecar for the heightmap (JSON5 stores the reference and cell size; `il_cli genmap` writes the Phase 1 test map the same way).
- **Unit and formation editors (Phase 6).** egui property grids over `Registry<UnitType>` and `Registry<FormationTemplate>` entries with schema-driven widgets (from the JSON Schema `description`/ranges); preview panel renders a formation at chosen `n`; save writes the item into the selected mod folder with a `$override: "merge"` diff if it derives from another mod's item.

## 17. Testing and CI

| Test | Location | Runs | Requirement |
|---|---|---|---|
| Unit tests per formula | each crate | every push | REQ-TEST-001 |
| Determinism: each scenario twice, 1 thread and 8 threads, snapshot/restore at mid-point | `tests/tests/determinism.rs`, in-process on `BattleWorld` (`set_threads(1)` and `set_threads(8)`), plus an in-process `il_cli::run` twice comparison; CI also diffs two `il_cli run --hash-every 1000` logs | every push | REQ-TEST-002 |
| Content validation of `game/` | `tests/content.rs` | every push | REQ-TEST-005 |
| Scenario outcome bands (Simulation Spec §15.3), 50 seeds | `tests/scenarios.rs` | nightly | REQ-TEST-004 |
| Benchmarks per system at 2k/10k/20k, budgets from the table at the top; fail at +20 % | `benches/` via criterion, compared against a checked-in baseline | nightly | REQ-TEST-003, REQ-PERF-005 |
| Replay verify | `il_cli replay --verify` on recorded replays in `tests/replays/` | nightly | REQ-SAVE-005 |
| Cross-machine hash compare | manual runbook, `il_cli run --hash-log` on two machines and `il_cli desync-report` | before Phase 7 | REQ-TEST-006 |

`il_cli` subcommands: `run <scenario.json5> --ticks N [--hash-every K] [--threads T] [--snapshot-at T]`, `bench`, `replay <file> --verify`, `validate <mods...>`, `desync-report <log_a> <log_b>`, `autoresolve <setup.json5>`, `genart [--mod-root]` (placeholder sprite sheets, T1-051), `genmap [--mod-root] [--id] [--seed]` (the deterministic Phase 1 test map and its heightmap, T1-030).

## 18. Coding conventions and determinism checklist

Conventions: `rustfmt` default; `clippy -D warnings`; no `unsafe` outside `il_render` (`unsafe_code = "forbid"` workspace-wide until then); public items documented with the rule ID they implement (`/// SIM-CMBT-011`); errors via `thiserror`; every tunable read from `Rules`, never a literal. `S` is a newtype, so sim constants are `S::from_i32(n)`, `S::HALF`, `S::ONE`; content and scenario values enter through `from_f32_data`; `f32::mul_add` is a clippy disallowed method (use `Scalar::mul_add_rounded`).

Determinism checklist for review of any sim change:

1. No `HashMap`/`HashSet` iteration; no `Instant`; no `thread_rng`; no `f32` literals outside `il_core::scalar` and data conversion.
2. Any per-entity random draw uses `hash_draw` with `(tick, id, index)`, never a sequential stream inside a parallel system.
3. Any parallel system writes only its own entity's components or a per-entity buffer applied later in id order.
4. Reductions have a fixed order.
5. New components that affect future state are added to the hash (SIM-DET-004) and to the snapshot.
6. New derived data is rebuilt in `restore`, not stored.
7. New Commands validate ownership and phase and produce `CommandRejected` on failure.
8. Stage placement follows SAD §6.2; moving a system across stages is an ADR.
9. The determinism test and the affected scenario bands pass locally at 1 and 8 threads.
