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

Budget table (sum must fit 50 ms at P3, 25 ms at P2). The measured columns are Phase 1 means from `il_cli bench --ticks 600` (release, 8 threads, the move/reform script, T1-083) on the target machine in `docs/evidence/phase1/machine.md`; stages 8 to 16 hold only a placeholder system, so their numbers are schedule overhead.

| Stage | Budget at 20k (ms) | Measured 2k | Measured 10k | Measured 20k | Section |
|---|---|---|---|---|---|
| 0 ApplyCommands | 0.2 | 0.00 | 0.00 | 0.00 | §4 |
| 1 AI | 2.0 | 0.03 | 0.05 | 0.07 | §8.5, §9 |
| 2 Formation | 2.0 | 0.17 | 0.35 | 0.58 | §7 |
| 3 RegimentMovement | 1.0 | 0.12 | 0.25 | 0.47 | §6 |
| 4 SoldierSteering | 8.0 | 1.03 | 3.56 | 7.74 | §6 |
| 5 Integrate | 0.5 | 0.10 | 0.19 | 0.33 | §6 |
| 6 SpatialGrid | 2.0 | 0.14 | 0.25 | 0.49 | §5 |
| 7 Collision | 8.0 | 1.94 | 5.61 | 11.16 | §6 |
| 8 Visibility | 1.0 | 0.07 | 0.10 | 0.11 | §8.4 |
| 9 Targeting | 4.0 | 0.06 | 0.06 | 0.08 | §8.1 |
| 10 Combat | 4.0 | 0.04 | 0.05 | 0.06 | §8.1 |
| 11 Projectiles | 3.0 | 0.04 | 0.04 | 0.06 | §8.2 |
| 12 Abilities | 0.5 | 0.03 | 0.04 | 0.05 | §8.3 |
| 13 Fatigue | 0.5 | 0.03 | 0.03 | 0.05 | §8.3 |
| 14 Morale | 1.0 | 0.03 | 0.03 | 0.04 | §8.3 |
| 15 Death | 1.0 | 0.03 | 0.03 | 0.04 | §8.1 |
| 16 BattleFlow | 0.3 | 0.03 | 0.03 | 0.03 | §4 |
| 17 Events + Hash | 3.0 | 0.12 | 0.49 | 1.07 | §2, §4 |
| **Total** | **42.0** | **4.03** | **11.17** | **22.43** | headroom 8 ms; p95 tick 6.4 / 16.7 / 32.9 ms |

At 20k the two soldier-level stages already sit at their Phase 3 budgets (Collision 11.2 ms against 8, Steering 7.7 against 8) while every other stage is far under; Phase 2 combat work must not grow them, and SAD §12 carries the item.

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

Feature flags (as built at the end of Phase 1):

| Flag | Crate | Effect |
|---|---|---|
| `dev` (default on) | il_app | hot reload (enables `il_data/hot-reload`), debug overlays, the profiler and event panels; CI also builds `--no-default-features` |
| `hot-reload` | il_data | the `notify` watcher and `HotReload` (T1-025) |

The `trace`, `headless` and `fixed` flags of the Phase 0 plan were never needed: il_cli links no render crate, profiling goes through `StageObserver` (SAD §9.3) and `Scalar` has one representation.

### 1.2 Dependencies (pinned per phase)

Phase 0 pins (T0-003) are the versions in the table; later phases pin their own crates when they arrive and update this table. Phase 1 pins (T1-050) are the newest mutually compatible set on 2026-09-03; `egui-wgpu` 0.36 requires `wgpu` ^30 and `egui-winit` 0.36 requires `winit` ^0.30.13.

| Crate | Version (initial) | Why | Used by |
|---|---|---|---|
| `bevy_ecs` | 0.19.1 (feature `multi_threaded`) | standalone ECS with schedules and parallel executor | il_sim_battle, benches |
| `bevy_tasks` | 0.19.1 | `ComputeTaskPool` for `BattleWorld::set_threads` | il_sim_battle |
| `wgpu` | 30.0.1 | GPU API | il_render |
| `winit` | 0.30.13 | window and input events | il_app, il_ui (event types; a direct dependency so `cargo test -p il_ui` unifies winit's features like il_app) |
| `egui`, `egui-wgpu`, `egui-winit` | 0.36.1 | UI (`egui-wgpu` paint pass lives in il_render) | il_render (`egui`, `egui-wgpu` with feature `winit`), il_ui (`egui`, `egui-winit` without default features), il_app (`egui`) |
| `serde` (feature `derive`) | 1 | serialisation | il_core, il_data, il_sim_battle, il_cli |
| `json5` | 1.3 | test fixtures only since T1-081; scenarios and content go through `il_data::json5`, a span-carrying parser written in T1-020 because per-field positions are needed for diagnostics and merge provenance (OQ-7 amended) | il_sim_battle, il_render, il_ui (dev-dependencies) |
| `semver` | 1 | manifest versions and ranges | il_data |
| `serde_json` | 1 | save headers, schema validation input | il_data, il_sim_battle, il_cli, tests (il_save when it arrives) |
| `jsonschema` | 0.53 (`default-features = false`) | content validation, draft 2020-12 | il_data |
| `postcard` | 1.1 (feature `use-std`) | snapshot encoding (OQ-2 resolved in Phase 0) | il_sim_battle (il_save when it arrives) |
| `mlua` (`lua54`, `vendored`) | 0.10 | Lua | il_script (Phase 6; not in the workspace yet) |
| `glam` | 0.33.6 | render-side math only (never in sim) | il_render, il_ui, il_app |
| `png`, `bytemuck`, `pollster` | 0.18.1 / 1 / 0.4 | atlas files, GPU buffer casts, blocking on device creation | il_render; il_cli uses `png` only (`genart`) |
| `xxhash-rust` (`xxh3`) | 0.8 | state hash | il_core |
| `tracing` | 0.1 | one `warn!` in `Locale` for a missing key; no subscriber is installed yet and il_sim_battle declares it without using it (SAD §12 T-11) | il_data, il_sim_battle |
| `criterion` | 0.8.2 | benchmarks (`benches/benches/*.rs`, `harness = false`; first bench in T1-031) | benches (dev-dependency; `il_cli` is a dev-dependency too, for the generated bench setups) |
| `kira` | 0.9 | audio (OQ-8: chosen for game-oriented mixing) | il_audio (Phase 2; not in the workspace yet) |
| `notify` | 8.2 (optional, behind `hot-reload`) | hot reload file watcher (dev) | il_data |
| `thiserror`, `anyhow` | 2 / 1 | errors (anyhow only in binaries and their libs) | all |
| `clap` | 4 (`derive`) | command-line parsing | il_cli, il_app |
| `toml` | 0.8 | manifest parsing in the dependency-rule test | tests (dev-dependency) |

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
// As built (T1-020..T1-025). Kinds for later phases (abilities, technologies, buildings, AI profiles, sound sets) join `Registries` with their phases.
pub struct ContentId(Arc<str>);              // "modid:item_id" matching ^[a-z0-9_]+:[a-z0-9_]+$; `ContentId::new(&str) -> Result<Self, InvalidContentId>`; one Arc per value, no intern table
pub struct Handle<T> { index: u32, _marker: PhantomData<fn() -> T> }   // Copy, Hashable; `index()`
pub struct Registry<T> { items: Vec<T>, ids: Vec<ContentId>, by_id: HashMap<ContentId, u32> /* lookup only, never iterated */, removed: BTreeSet<u32> /* hot-reload tombstones */ }
impl<T> Registry<T> {
    pub fn get(&self, h: Handle<T>) -> &T;                  // infallible; handles are validated at load
    pub fn lookup(&self, id: &ContentId) -> Option<Handle<T>>;   // None for removed slots (`lookup_any` sees them)
    pub fn id_of(&self, h: Handle<T>) -> &ContentId;
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)>;   // ascending index, skipping removed slots = deterministic
    pub fn ids(&self) -> impl Iterator<Item = &ContentId>;  // plus all_ids, slots, len, is_empty, contains, is_removed, removed_ids, ids_added_after
}
impl<T: ContentKind> Registry<T> { pub fn insert(&mut self, item: T) -> Result<Handle<T>, DuplicateId>; }
pub trait ContentKind: DeserializeOwned + Clone + Send + Sync + 'static {
    const DIR: &'static str; const TAG: KindTag;            // TAG selects the embedded schema (T1-021)
    fn id(&self) -> &ContentId;
    fn resolve(&mut self, lookup: &Lookup, errors: &mut Vec<ResolveError>) {}   // ContentIds → handles; each unknown reference is one positioned error with a suggestion
    fn hash_content(&self, h: &mut StateHasher) {}          // the sim-relevant fields; references hash as ContentIds
}

pub struct Registries {
    pub units: Registry<UnitType>, pub formations: Registry<FormationTemplate>, pub group_formations: Registry<GroupFormationTemplate>,
    pub factions: Registry<Faction>, pub zones: Registry<ZoneType>, pub maps: Registry<MapDef>, pub sprite_sets: Registry<SpriteSet>,
    pub rules: Rules,                         // `Rules { movement, formation, combat, morale, fatigue, general, visibility, battle_flow }`, one struct per `content/rules/*.json5`, every field required (§3.3 step 6); `il_sim_battle::Rules` re-exports it
    pub input: InputBindings, pub locale: Locale, pub mods: Vec<ModInfo>,
    pub mod_list_hash: u64,                   // xxh3 over (id, version) pairs in load order
    pub content_registry_hash: u64,           // `compute_content_hash()`: xxh3 over `hash_content` of every item, kinds in a fixed order, items in ContentId order; independent of file order, whitespace, key order and registry layout (Networking Spec §4.2)
}
// Typed kinds exported with it: UnitType (Ranged, ExperienceTier, UnitCategory, UnitSounds, ProjectileArc), FormationTemplate (Layout, RoleZone),
// GroupFormationTemplate (GroupKind), Faction (DiplomacyPersonality), ZoneType, MapDef (MapSize, HeightmapRef, ZonePolygon, River, DeploymentZone,
// ReinforcementEdge, MapEdge), SpriteSet (Anim), InputBindings (Binding), MovementRules, FormationRules, CombatRules, MoraleRules (MoraleWeights, StateMults, StateMultsTable), FatigueRules, GeneralRules, VisibilityRules, BattleFlowRules (TimeoutWinner), de::Rgb; `merge::{KindAccumulator, MergedItem, Tombstone}`, `Sources`/`SourceFile` and `Lookup` are the pipeline's working types.

pub struct ModSet { pub mods: Vec<LoadedMod>, pub warnings: Vec<String> }   // resolved load order; `index_of`, `mod_list_hash`
pub struct LoadedMod { pub manifest: Manifest, pub root: PathBuf, pub is_game: bool }   // `namespaces()`
pub struct ManifestWithPath { pub manifest: Manifest, pub root: PathBuf, pub is_game: bool }
pub fn read_manifest(root: &Path, is_game: bool) -> Result<ManifestWithPath, Diagnostics>;   // validated against mod-manifest.schema.json
pub fn discover(roots: &[PathBuf]) -> Result<Vec<ManifestWithPath>, Diagnostics>;
pub fn resolve_load_order(found: &[ManifestWithPath], enabled: &[String]) -> Result<ModSet, Vec<LoadOrderError>>;   // `Edge`, `EdgeKind` name the graph edges in errors
pub fn discover_set(roots: &[PathBuf]) -> Result<ModSet, Diagnostics>;
pub fn load(set: &ModSet) -> Result<Registries, Diagnostics>;        // collects ALL diagnostics before failing
pub fn load_roots(roots: &[PathBuf]) -> Result<Registries, Diagnostics>;   // discover_set + load; what il_cli, il_app and the tests call
pub struct LoadReport { pub registries: Option<Registries>, pub diagnostics: Diagnostics }
pub fn load_report(set: &ModSet) -> LoadReport;                     // warnings alongside a successful load (il_cli validate)
pub fn load_report_with_prev(set: &ModSet, prev: Option<&Registries>) -> LoadReport;   // index-stable relayout for hot reload
pub fn validate_value(..) / validate_merged(..);                     // schema validation of a merged value; every error maps to the key's line and column (T1-021)

pub struct Diagnostic { pub severity: Severity /* Error | Warning */, pub file: PathBuf, pub line: u32, pub col: u32, pub field: String, pub message: String, pub expected: Option<String> }   // builders file_level/at/field/expected/warning; `is_error`
pub struct Diagnostics(pub Vec<Diagnostic>);   // has_errors, errors(), warnings(), into_result; Display lists every one; implements Error

pub mod json5 {   // the engine's own JSON5 parser (T1-020): every key and value keeps its position, for diagnostics and merge provenance
    pub fn parse_json5(src: &str, file: FileId) -> Result<SpannedValue, ParseError>;   // full JSON5; duplicate keys, Infinity and NaN are errors
    pub struct FileId(pub u32); pub struct Span { pub file: FileId, pub line: u32, pub col: u32 }   // 1-based
    pub struct SpannedValue { pub span: Span, pub kind: ValueKind }   // Null | Bool | Num(Int(i64) | Float(f64)) | Str | Array | Object(Vec<(Key, SpannedValue)>) in source order
    // as_object/as_array/as_str/as_bool, get/get_mut/remove, key_span, at_path(&[PathSeg]), to_json() -> serde_json::Value, span_display
}

pub struct Locale { tables: BTreeMap<String /*lang*/, BTreeMap<String, String>>, current: String, show_keys: AtomicBool, missing: Mutex<BTreeSet<String>> }   // as built (T1-024): fallback is always FALLBACK_LANGUAGE = "en"; misses are recorded once and logged with tracing::warn
impl Locale { pub fn get<'a>(&'a self, key: &'a str) -> &'a str;  /* current → en → the key itself */ pub fn fmt(&self, key: &str, args: &[(&str, &dyn Display)]) -> String; pub fn has(&self, key: &str) -> bool; pub fn set_language(&mut self, lang: &str) -> bool; pub fn language(&self) -> &str; pub fn languages(&self) -> Vec<&str>; pub fn set_show_keys(&self, on: bool); pub fn show_keys(&self) -> bool; pub fn missing_keys(&self) -> Vec<String>; pub fn insert(&mut self, lang: &str, key: &str, text: &str); }

#[cfg(feature = "hot-reload")]   // il_app enables it through its `dev` feature
pub const QUIET_POLLS: u32 = 6;      // polls without a change (≈ 100 ms at 60 Hz) before a rebuild; il_data reads no clock
pub enum ReloadEvent { Swapped { files: Vec<PathBuf> }, Structural { added: Vec<(KindTag, ContentId)>, removed: Vec<(KindTag, ContentId)> }, Failed(Diagnostics), ManifestIgnored(PathBuf) }
pub struct HotReload { _watcher: notify::RecommendedWatcher, rx: Receiver<notify::Result<notify::Event>>, set: ModSet, current: Arc<Registries>, dirty: Vec<PathBuf>, quiet_polls: u32, events: Vec<ReloadEvent> }
impl HotReload {
    pub fn new(set: ModSet, current: Arc<Registries>) -> notify::Result<Self>;   // watches every mod's content and locale folders
    pub fn poll(&mut self) -> Option<Arc<Registries>>;   // per frame; after QUIET_POLLS quiet polls re-runs the whole pipeline laid out like `current` (old ids keep their index, deleted ids stay as removed slots, new ids append); the app calls BattleWorld::replace_registries between ticks
    pub fn rebuild_now(&mut self) -> Option<Arc<Registries>>;
    pub fn take_events(&mut self) -> Vec<ReloadEvent>;   // Failed keeps the old registries; ManifestIgnored because manifests are read at startup only
    pub fn current(&self) -> &Arc<Registries>; pub fn mod_set(&self) -> &ModSet;
}
```

### 3.3 Load pipeline

1. `discover`: read every `mod.json5` under the given roots (the game root first; `--mod` on il_cli and il_app adds folders); each manifest is validated against `mod-manifest.schema.json`.
2. `resolve_load_order`: Kahn topological sort over `dependencies`, `load_after`, `load_before`; ties by mod id ascending; cycle → error listing the cycle.
3. For each mod in order, for each `ContentKind::DIR`, parse every `*.json5` with `il_data::json5::parse_json5` into a `SpannedValue` (every key and value keeps `file:line:col`; `to_json()` gives the plain `serde_json::Value`). Per-file checks are limited to the object shape, the `id`, directive syntax and duplicate ids within the mod. Objects then merge into the kind's accumulating map keyed by ContentId (`il_data::merge`): `$from` copies an existing item of the same kind as the base (forward references inside a mod are applied first; depth <= 8; cycles are errors), then `$override`, `$delete` and list directives apply per Modding SDK §3.4.1; directives never survive into the map. A merged leaf keeps the key span of the mod that first wrote the field and takes the value span of the last writer. Validation runs on the **merged result only** (SDK §3.4.1 rule 4, decided in Phase 1): errors point at the original key and, when another mod wrote the value, add `after merge by "<mod>" (<file>:<line>:<col>)`. Merge fragments and `$delete` objects therefore never fail the `required` list.
4. Deserialise merged values into typed structs; call `resolve` to turn ContentIds into handles (two-pass: all ids registered first, then references resolved, so order between files does not matter). Ids that failed validation are registered as invalid, so a reference to one is not reported a second time.
5. Singleton kinds (`input/bindings.json5`, one merged object per rules file) and the locale tables (`locale/<lang>.json5`, deep-merged per language) go through the same merge; the heightmap sidecar of every map (`.hgt`, 16-bit little-endian samples at `height_cell`) is read from the `assets_root` of the mod that last wrote `heightmap.path`, so the sim never touches the filesystem. Then `content_registry_hash` and `mod_list_hash`.
6. Rules files: exactly one merged object per rules kind; every field is required and a missing file or field is an error (Phase 1 decision: no engine numeric defaults, Simulation Spec §15.1 lists them all). Loading continues with zeroed rules so every diagnostic is reported in one run.

Budget: load of the flagship game < 1 s; not per tick.

### 3.4 Tests

- Golden diagnostics for malformed files (file:line:col in the message); the broken fixture mod under `tests/fixtures/` yields exactly its expected positioned errors (`tests/tests/content.rs`).
- Load-order tests: diamond dependencies with the id tie-break, hard cycles named in the error, soft cycles dropping the first soft edge, `load_before` contradicting a dependency, the game always first, missing dependencies and version mismatches, duplicate and unknown enabled ids, disabled mods not constraining the order, `mod_list_hash` depending on order and version.
- Override tests: replace, deep merge, `$append`/`$remove`/`$replace`, `$delete`, `null` removing a key, plain lists replacing, `$from` with forward references, depth limit and cycles, the namespace rule, directive syntax errors; a second mod merging into a game unit (`tests/tests/mod_override.rs`) and the SDK worked example (`tests/tests/sdk_example.rs`).
- Registries: the game root populates every Phase 1 registry; handles resolve regardless of file order; unknown references are positioned with a suggestion; every rules field is required; one tweaked rule changes the content hash.
- `content_registry_hash` stability across file layout, whitespace, key order and number spelling.
- Locale: a miss returns the key and is recorded once; the fallback chain; `fmt` placeholders; `show_keys`.
- Schemas: every embedded schema compiles; manifests validate.
- Hot reload (`tests/tests/hot_reload_sim.rs`): an edited number is swapped into a running `BattleWorld` between ticks with the index layout preserved; a structural change is reported; a failing edit keeps the old registries.

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
    pub const SOLDIER_CAP: u32 = 32_768;                  // `BattleSetup::soldier_total()` (initial plus reinforcements) above it is `SetupError::OverCap` (SIM-CORE-006)
    pub enum Weather { Clear, Rain, Fog }   pub struct VictoryRules { pub timeout_winner: Option<u8> }
    pub struct GeneralSetup { pub unit_type: ContentId, pub rank: u8, pub name_key: String }
    pub struct ReinforcementGroup { pub arrival_tick: u32, pub edge: u8, pub regiments: Vec<RegimentSetup> }
    pub struct Scenario { #[serde(flatten)] pub setup: BattleSetup, pub commands: Vec<Command> }   // a scenario file (T1-081); `script() -> ScriptedCommands`
    pub struct ScriptedCommands { .. }   // sorted by (tick, player, seq); `take_for(tick) -> Vec<Command>` hands over everything stamped `tick` or earlier (stale ones too, so the sim rejects them visibly), `remaining`, `is_empty`
    pub struct BattleResult { pub winner: Option<u8>, pub duration_ticks: u32, pub sides: Vec<SideResult>, pub summary: BattleSummary { total_killed, total_fled } }
    pub struct SideResult { pub regiments: Vec<RegimentResult>, pub general_fate: GeneralFate, pub loot: i64 }
    pub struct RegimentResult { pub id: u32, pub initial: u16, pub survivors: u16, pub fled: u16, pub killed: u16, pub experience_gain: u16, pub ammo_left: u16 }
    pub enum GeneralFate { Alive, Wounded, Dead, Captured }
}

pub struct BattleWorld { world: bevy_ecs::World, view_queries: ViewQueries /* cached QueryStates for view() */, schedules: Vec<Schedule> /* one per Stage, §4.5 */, tick: Tick, phase: BattlePhase }
impl BattleWorld {
    pub fn new(setup: &BattleSetup, regs: Arc<Registries>) -> Result<Self, SetupError>;   // validates SIM-FLOW-019; the world keeps the Arc
    pub fn step(&mut self, commands: &[Command]) -> StepOutput;   // exactly one tick: simulates tick() + 1; commands must be stamped with that tick
    pub fn tick(&self) -> Tick;                                    // completed ticks; the app gathers commands for tick() + 1 (§15)
    pub fn phase(&self) -> BattlePhase;
    pub fn empty(seed: u64, regs: Arc<Registries>, phase: BattlePhase) -> Self;   // no map, no soldiers (tests, tools)
    pub fn snapshot(&self) -> Snapshot;                            // an owned copy of all Hashable+Serialize components and resources; `Snapshot::to_bytes() -> Vec<u8>` / `from_bytes(&[u8]) -> Result<Snapshot, RestoreError>` are the postcard encoding (§4.6)
    pub fn restore(snapshot: &Snapshot, regs: Arc<Registries>) -> Result<Self, RestoreError>;  // rebuilds derived data (paths, flow fields, grid)
    pub fn hash(&self) -> StateHash;                               // same value as StepOutput.hash of the last step (or of the initial state)
    // `result(&self) -> Option<BattleResult>` arrives with the battle flow in Phase 2; Phase 1 never reaches `Ended`
    pub fn step_observed(&mut self, commands: &[Command], observer: &mut dyn StageObserver) -> StepOutput;   // `step` with `NoopObserver`; begin/end around every stage (§4.5, SAD §9.3)
    pub fn view(&self) -> BattleView<'_>;                          // read-only accessors for render/ui/ai (T1-052): tick(), phase(), regs(), rules(), sides(), map(), nav_grid(), spatial_grid(), anchor_grid(), soldier_count(), regiment_count(), soldiers()/soldiers_unordered()/soldier(id) -> SoldierRow { id, regiment, unit, category, pos, prev_pos, facing, prev_facing, state, hp, slot }, regiments()/regiment(id) -> RegimentRow { id, side, unit, anchor_pos, anchor_facing, order, morale, morale_state, soldier_count, integrity, formation, ranks, files }, formation_state(id), path(id), slots_world(&row); cached QueryStates refreshed by step/new/restore/recompute_hash
    pub fn map(&self) -> &Arc<LoadedMap>; pub fn nav_grid(&self) -> &NavGrid; pub fn setup(&self) -> Option<&BattleSetup>; pub fn registries(&self) -> &Arc<Registries>;
    pub fn soldier_ids(&self) / regiment_ids(&self) -> impl Iterator; pub fn soldier_count(&self) / regiment_count(&self) -> usize;
    pub fn replace_registries(&mut self, regs: Arc<Registries>);   // hot reload (T1-025): asserts the old id list is a prefix of the new one per kind; values copied at spawn (`Body`) do not update
    pub fn threads(&self) -> usize; pub fn recompute_hash(&mut self) -> StateHash; pub fn ecs(&self) -> &World; pub fn debug_translate_all(&mut self, delta: V2, facing: Option<Angle<S>>);   // tools and tests
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
pub enum AbilityTarget { SelfTarget, Point(V2), Regiment(RegimentId) }
pub enum RejectReason { StaleTick { command_tick: Tick, current: Tick }, UnknownRegiment(RegimentId), NotOwner(RegimentId), Routing(RegimentId), WrongPhase, UnknownContent(ContentId), FormationNotAllowed { regiment: RegimentId, template: ContentId }, InvalidTarget(RegimentId) /* AttackRegiment at an own-side or empty regiment, T2-020 */, NotImplemented }
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
| `MeleeState` (T2-020) | `target: Option<SoldierId>, cooldown: u16` | yes | — |
| `Attackers` (T2-020) | `n: u8`, soldiers targeting this one; recounted after targeting and on restore | no (derived) | — |
| `RangedState` (Phase 2) | `ammo: u16, cooldown: u16` | yes | — |
| `Rank` | `rank: u8, file: u16` | no | — |
| `GeneralTag` (Phase 2) | marker + `rank: u8` | — | — |
| `Dead` (Phase 2) | marker, removed at Stage 15 | — | — |

Regiment-level components live on regiment entities (≈ 200). As built (Phase 1): `Regiment { id, side: u8, setup_id: u32, unit: Handle<UnitType>, soldiers: Vec<SoldierId> (ascending), ammo: u16 }` (one unit type per regiment until Phase 3, plan S15), `Anchor { pos: V2, facing: Angle<S> }`, `FormationState { template, ranks: u8, files: u16, slots: Vec<Slot> (derived), assignment: Vec<Option<u16>>, integrity: S, morph_until: Tick, needs_reform: bool, prior_template: Option<Handle<FormationTemplate>> (corridor morph), laid_out_facing: Angle<S>, dirty: bool }` (`FormationState::new(template, ranks, slots, facing)`), `Order { kind: OrderKind, target: V2, facing: Option<Angle<S>>, speed: SpeedMode, since: Tick }` (`OrderKind::moves()`), `Path { waypoints: Vec<Waypoint { p: V2, corridor: S }>, next: u16, requested: bool }` (stored, hashed and snapshotted, T1-032; `is_active()`, `current()`), `Morale { m: S, state: MoraleState, deaths_5s: [u16; DEATHS_RING = 100], initial: u16 }` (the ring and `initial` since T2-020, written by death; `rout_count` and `engaged_since` arrive with T2-041), `Combat { engaged: bool, last_fighting: Tick, charge_until: Tick, experience: u8, kills: u32 }` (T2-020; hashed and snapshotted), `Order` additionally carries `target_regiment: Option<RegimentId>` (T2-020). `SoldierState` is `Idle | MoveToSlot | Fighting | Routing | Withdrawing | Dead`. Phase 2 further adds `Fire { mode, target: Option<RegimentId>, retarget_at: Tick }`, `Energy { e: S }`, `Statuses { list: SmallVec<[StatusEffect; 4]> }`, `Cooldowns { list: SmallVec<[(Handle<Ability>, u16); 4]> }`, `Experience(u8)`, `Visible { by_faction: u8 /* bitmask */ }`.

Projectiles (Phase 2, T2-030): `Projectile { id, shooter_regiment, side, launch_tick, land_tick, start: V2, end: V2, apex: S, kind, damage, pen }`, `Pos`, plus a `ProjectilePool` resource of free entities (REQ-PERF-008).

### 4.4 Resources

As built (Phase 1): `Clock { tick }`, `Phase`, `Sides(Vec<SideState { player, faction, deployment_zone, deployment_confirmed, defeated }>)`, `MapRes(Arc<LoadedMap>)` (heightmap, zone raster, river flags, deployment polygons; built by `new`/`restore` from `map_id`, `BattleWorld::empty` holds a flat placeholder `engine:flat`), `SpatialGridRes(SpatialGrid<SoldierId>)`, `AnchorGridRes(SpatialGrid<RegimentId>)`, `NavGridRes(NavGrid)`, `PathfinderRes(AStar)`, `PathRequests(BTreeSet<RegimentId>)`, `MeleeGateRes { side, may_fight, near_enemy, extent }` (T2-020, per regiment in `Ids` order, rebuilt every Stage 9), `CommandInbox(Vec<Command>)`, `Rejected(Vec<(Command, RejectReason)>)`, `StepEvents(Vec<BattleEvent>)`, `LastHash(StateHash)`, `Rng { seed: u64, streams: [RngStream; StreamId::COUNT = 9] }`, `Ids { soldiers, regiments, projectiles: IdAllocator, soldier_entities: Vec<(SoldierId, Entity)>, regiment_entities: Vec<(RegimentId, Entity)> }` (the canonical ascending order every exclusive system iterates), `Regs(Arc<Registries>)` (rules are read as `Regs.0.rules`, so a hot-reload swap carries them; there is no separate `Rules` resource), `SetupRes(Option<BattleSetup>)`, `ThreadCount`. Later phases add `HpaGraph`, `FlowFields`, `Weather`, `Timer`, `PendingDamage: Vec<(Tick, SoldierId, S, Angle<S>)>` (SIM-PROJ-008) and the side's edge and flow-field handle.

### 4.5 Schedule

One `Schedule` per stage, run in `Stage::ALL` order by `step` (T1-060; stages were already totally ordered, so nothing is lost and each stage can be timed through `StageObserver`), with one `SystemSet` per stage inside it. As built the systems of a stage are `.chain()`ed (explicit total order); parallelism lives *inside* systems (`par_iter` over soldiers, `ComputeTaskPool::scope` over grid rows), never between them. Every schedule is built with the `SingleThreadedExecutor`; `set_threads(n > 1)` swaps all 18 to the multi-threaded executor on the process-global pool. Systems that must be exclusive for determinism take `&mut World` (the apply steps). `Stage` exposes `COUNT = 18`, `ALL`, `index()`, `name()`; `build_schedules() -> Vec<Schedule>`; `StageObserver { begin(Stage), end(Stage) }` with `NoopObserver` for plain `step`. The ten stages without Phase 1 systems (1, 8..16) each hold one empty system named after the stage so the profiler shows every row (SAD §12 T-9).

Stage 0 `apply_commands`: sort incoming by `(player, seq)`, validate per SIM-CMD-003/004, mutate `Order`, `FormationState`, `Fire`, `Sides`; push `CommandRejected` events. As built (T1-047): `Move` / `AttackMove` (a move until Phase 2) write a fresh `Order` (target clamped to the map), clear the path, queue a `PathRequests` entry and request a reform; `Halt` ends the order and drops the path and wheel target; `SetFormation` rejects `UnknownContent` and `FormationNotAllowed` (template not in the unit's `formations`), then sets the template with `morph_until = tick + morph_ticks`, `ranks` (the template default when `None`), cancels any corridor morph and requests a reform; `SetFacing` goes through `formation::set_facing`; `SetSpeedMode` sets `Order.speed`; `GroupFormation` runs `arrange_group` and issues one move per placement with its ranks; `Deploy` is `WrongPhase` outside the deployment phase and otherwise teleports the anchor and its soldiers onto their slots; routing or shattered regiments reject everything but `Withdraw` (SIM-CMD-004).
Stage 2 `formation_layout`, `formation_apply`, `formation_integrity` (§7); Stage 3 `pursue_update` (T2-020: target check, `AttackMove` acquisition, re-path request, charge `run` switch; exclusive, ascending id), `serve_path_requests` (an attack order's destination is its target regiment's anchor), `regiment_follow_path` (holds an engaged attacker's anchor); Stage 4 `soldier_steer` (Fighting branch seeks the target); Stage 5 `integrate`; Stage 6 `rebuild_spatial_grids`; Stage 7 `collision_resolve` (§5, §6); Stage 9 `melee_gate`, `melee_target`, `melee_recount` (§8.1).
Stage 16 `battle_flow`: SIM-FLOW-011..017 (Phase 2; an empty placeholder in Phase 1).
Stage 17 `flush_events_and_hash`: copy `Pos→PrevPos`, `Facing→PrevFacing`; hash per SIM-DET-004 in ascending id (iterate a sorted `Vec<Entity>` maintained by the `Ids` resource); drain events.

### 4.6 Snapshot

`Snapshot { version: u32, tick, phase, setup: BattleSetup, ids, rng, sides, regiments: Vec<RegimentSnap>, soldiers: Vec<SoldierSnap>, projectiles: Vec<()> /* ProjectileSnap from T2-030 */, pending_damage: Vec<()> /* T2-031 */, timer }` encoded with postcard (`SNAPSHOT_VERSION = 3` since T2-020: `RegimentSnap` carries the order's target regiment, the `Combat` fields, the casualty ring as a `Vec<u16>` and `initial`; `SoldierSnap` carries the melee target and cooldown; version 2 added the required `map_id` in T1-030; older snapshots are not migrated). Phase 0 layout: `RegimentSnap { id, side, setup_id, unit_type: ContentId, anchor_pos, anchor_facing, morale, morale_state, order, ammo }`, `SoldierSnap { id, regiment, p, v, facing, hp, fatigue, slot, fsm_state, fsm_since }`, `IdsSnap { soldiers_next, regiments_next, projectiles_next }`; `PrevPos`/`PrevFacing` and `Body` are rebuilt on restore. As built (T1-030..T1-048) `RegimentSnap` also carries the order (target, facing, speed, since), the stored path (waypoints with corridors, next, requested) and the formation state (`formation` and `prior_formation` as ContentIds, `ranks`, `integrity`, `morph_until`, `needs_reform`, `laid_out_facing`); `restore` installs the map from `setup.map_id`, then `rebuild_derived` rebuilds the spatial and anchor grids (from positions), the `NavGrid` (from the map; `HpaGraph`/`FlowFields` and gate states arrive with their phases), the `PathRequests` queue (from `Path.requested`), the formation slot tables (from template, count and ranks), `Rank` (from `SlotRef`) and the attacker counts (from the melee targets, T2-020). Paths are stored, not re-requested (SIM-DET-005). Snapshot of 32k soldiers ≈ 32k × 40 B ≈ 1.3 MB.

### 4.7 Tests

- `step` with no commands on an empty world advances tick and hash changes only by tick.
- Command validation matrix (ownership, phase, routing).
- Snapshot round trip: hash(restore(snapshot(w))) == hash(w) and continues identically for 1,000 ticks.
- Threads 1 vs 8 hash equality on the full scenario set.

## 5. Spatial grid (`il_sim_battle::spatial`)

```rust
// As built (T1-031): generic over the stable id so the same type indexes soldiers and regiment anchors.
pub struct Entry<Id> { pub id: Id, pub entity: Entity, pub pos: V2 }
pub struct SpatialGrid<Id> { cell: S, inv_cell: S, cols: u32, rows: u32, heads: Vec<u32> /* per cell, first index */, next: Vec<u32> /* per entry */, entries: Vec<Entry<Id>> /* ascending id */, slots: Vec<u32> /* rebuild scratch */ }
impl<Id: Copy + Ord> SpatialGrid<Id> {
    pub fn new(width: S, height: S, cell: S) -> Self;                 // cols/rows = ceil(extent / cell); a non-positive cell means one cell
    pub fn ensure(&mut self, width: S, height: S, cell: S) -> bool;   // re-dimensions when the map or the rules changed (hot reload)
    pub fn rebuild(&mut self, iter: impl IntoIterator<Item = Entry<Id>>);   // sorted by id, inserted back to front so every cell chain ascends → deterministic bucket order
    pub fn cell_entries(&self, cx: u32, cy: u32) -> CellIter<'_, Id>;   // indices into entries(), ascending id
    pub fn query_circle(&self, c: V2, r: S, out: &mut Vec<Entry<Id>>);         // ascending id (sorted after collection); query_circle_indices for the index form
    pub fn for_each_pair(&self, f: impl FnMut(usize, usize));       // i<j within same and neighbouring cells, each pair once (self, E, NE, N, NW), rows ascending
    pub fn for_each_pair_in_row(&self, cy: u32, f: impl FnMut(usize, usize));   // the pairs of one row, so rows can run in parallel into per-row buffers
    pub fn cell_of(&self, p: V2) -> (u32, u32);                      // clamped to the grid
    pub fn query_circle_indices(&self, c: V2, r: S, out: &mut Vec<usize>);   // the index form of query_circle
    pub fn cell(&self) -> S; pub fn cols(&self) / rows(&self) -> u32; pub fn entries(&self) -> &[Entry<Id>]; pub fn len(&self) -> usize; pub fn is_empty(&self) -> bool;
}
pub fn rebuild_spatial_grids(/* Stage 6 system */);                 // SpatialGridRes (soldiers, movement.spatial_cell) and AnchorGridRes (anchors, movement.anchor_cell); also run by rebuild_derived
```

- Cell size `movement.spatial_cell` = 4 m (about 10 soldier diameters); at 2 km × 2 km that is 250k cells, 1 MB of heads. Rebuilt every tick (Stage 6) rather than incrementally: 32k inserts ≈ 0.3 ms; a full rebuild is simpler to keep deterministic (ADR-013).
- Pair iteration for collision uses the half-neighbourhood pattern (self, E, NE, N, NW) so each pair is visited once; pairs are collected per cell into a buffer, sorted by `(i, j)` id, then processed. Parallel over cell rows with results in per-soldier push buffers, applied in id order (SAD §8).
- A second grid instance (`AnchorGridRes`, `movement.anchor_cell` = 16 m) indexes regiment anchors for AI and visibility queries.

Tests: query results equal brute force on random layouts; pair enumeration is a permutation-invariant set.

## 6. Movement and pathfinding (`il_sim_battle::movement`, `::nav`)

### 6.1 Nav grid and paths

```rust
pub struct NavGrid { cell: S, inv_cell: S, cols: u32, rows: u32, cost: Vec<u16> /* 0 = impassable; else cost×100 */, passable_run_x: Vec<u8>, passable_run_y: Vec<u8> }
impl NavGrid {
    pub fn from_map(map: &LoadedMap, regs: &Registries, rules: &MovementRules) -> Self;   pub fn from_costs(cell: S, cols: u32, rows: u32, cost: Vec<u16>) -> Self;   // tests
    pub fn cell(&self) / cols / rows / cell_count; pub fn index(cx, cy) / coords(index); pub fn cell_of(p) / cell_center(cx, cy) / in_bounds(cx: i64, cy: i64);
    pub fn cost(cx, cy) -> u16; pub fn is_passable(cx, cy) / is_passable_at(p); pub fn corridor_width_at(p) -> S; pub fn passable_run_x / passable_run_y(cx, cy) -> u8; pub fn nearest_passable(cx, cy) -> Option<(u32, u32)>; pub fn segment_clear(a: V2, b: V2) -> bool;
    // Phase 5 (gates): pub fn update_gate(&mut self, gate: GateId, state: GateState) -> DirtyRect;
}
// Phase 3 (REQ-PATH-002):
pub struct HpaGraph { cluster: u32, gates: Vec<GateNode>, edges: Vec<(u32, u32, u32)>, cluster_of: Vec<u16> }
impl HpaGraph { pub fn build(nav: &NavGrid, cluster: u32) -> Self; pub fn repair(&mut self, nav: &NavGrid, dirty: DirtyRect); }
pub trait Pathfinder { fn find(&mut self, nav: &NavGrid, from: V2, to: V2, out: &mut Vec<V2>) -> PathResult; }
pub struct AStar { open: BinaryHeap<Reverse<(u32 /* f */, u32 /* node */)>>, g: Vec<u32>, came: Vec<u32>, g_epoch: Vec<u32>, closed_epoch: Vec<u32>, epoch: u32 }   // `new()`, `search_cells(nav, start, goal, out: &mut Vec<(u32, u32)>) -> Option<u32>` (cell path and cost); `dijkstra_cost(nav, start, goal)` is the test oracle
pub struct Hpa { abstract_astar: AStar, refine: AStar, graph: HpaGraph }   // Phase 3
pub fn string_pull(nav: &NavGrid, path: &mut Vec<V2>);
pub struct PathRequests(pub BTreeSet<RegimentId>);   // a resource (§4.4), served ascending, `movement.paths_per_tick` per tick (SIM-MOVE-005)
```

- A\* uses integer costs (octile × 100) so the heap order is deterministic regardless of `Scalar`. Ties in the heap are broken by node index.
- `Pathfinder` is a resource swapped by phase: `AStar` in Phase 1, `Hpa` from Phase 3 (REQ-PATH-002).
- As built (T1-032): `NavGrid::from_map(&LoadedMap, &Registries, &MovementRules)` marks a nav cell impassable when any zone cell whose centre lies in it is `passable: false` or a river cell without a `crossing` zone, and costs it the largest `move_cost × 100` of those zone cells (slope is not in the cost; `from_costs` builds test grids). Diagonal steps cost `ceil(cost × 141 / 100)` and never cut an impassable corner. `Pathfinder::find(nav, from, to, out) -> PathResult::{Found, NoPath, StartBlocked, GoalBlocked}`: blocked endpoints snap to the nearest passable cell within `SNAP_RADIUS` = 8 rings (ties by smaller `(cy, cx)`), `out[0] == from`, the last point is `to` (or the snapped cell centre); `string_pull` is greedy farthest-visible over `segment_clear`, a supercover DDA that also tests both side cells at an exact corner crossing. `corridor_width_at(p)` = `min(passable_run_x, passable_run_y) × cell`; each `Waypoint { p, corridor }` stores it so the corridor morph (SIM-MOVE-004) compares against the regiment's current width at follow time instead of a baked flag. `serve_path_requests` (Stage 3, exclusive) pops up to `paths_per_tick` ids from `PathRequests` (a `BTreeSet<RegimentId>`, rebuilt on restore from `Path.requested`), writes `Path { waypoints, next: 1, requested: false }`, and on failure resets the order to Idle with a `PathNotFound` event. `dijkstra_cost` is the optimality oracle.

### 6.2 Systems

| System | Stage | Parallel | Rule IDs |
|---|---|---|---|
| `serve_path_requests` | 3 | no | SIM-MOVE-002/005 |
| `regiment_follow_path` (anchor move, wheel, cohesion, corridor column morph) | 3, after `serve_path_requests` | per regiment (independent; parallel when a task pool exists) | SIM-MOVE-010..013, SIM-MOVE-004; as built (T1-042): waypoints within `waypoint_radius` are skipped in one tick, the anchor wheels toward the waypoint by `wheel_rate × dt` then advances `min(v_reg × dt, distance)` clamped to the map; `v_reg = mode_speed(unit, order.speed) × template.speed_mult × zone.move_mult(anchor) × slope_mult(anchor, dir)`, `× morph_speed_mult` while `tick < morph_until`, `× straggler_slowdown` while the straggler fraction (soldiers farther than `straggler_radius × sf` from their slot) exceeds `straggler_fraction`; on arrival the order becomes Idle, the ordered facing is taken, a prior template restored and a reform requested |
| `soldier_steer` (seek/flow, separation via grid, avoidance) → writes `Vel`, `Facing`, `Fsm` | 4 | par_iter over soldiers (reads previous tick grid) | SIM-MOVE-020..025, SIM-FLOW-002; as built (T1-043): the slot comes from the regiment's `Anchor` + `FormationState` through `Ids`, `v_max = mode_speed(unit, order.speed) × zone × slope`, neighbours are the `sep_max_neighbours` nearest grid entries (ties by id) within `2r_i + 2r_j + sep_margin` with `r_j` read from the neighbour's `Body`, avoidance tries ±15°, ±30°, ±45°, ±60°, ±90° against `NavGrid::segment_clear` over `lookahead_ticks` and stops when none is clear; facing tracks the slot facing within `slot_arrive_radius`, else the velocity |
| `integrate` | 5 | par_iter | `p += v × dt`; SIM-MOVE-042 clamp; as built (T1-043) `push_out` tries the full move, then x only, then y only, else stays (Phase 1 plan S12) |
| `collision_resolve` | 7 | pair buffers per cell row → id-order apply, ×`collision_iterations` | SIM-MOVE-040..043; as built (T1-044): the pair lists of this tick's grid are enumerated once per row (rows in parallel through `ComputeTaskPool::scope` when a pool exists), sorted `(i, j)`, then each pass folds them in row order into per-soldier pushes from the current positions and applies the pushes in ascending id through `push_out`; positions are written back once; coincident centres separate along +x |
| `rebuild_spatial_grids` | 6 | no | §5: soldiers into `SpatialGridRes`, anchors into `AnchorGridRes`, from end-of-tick positions |
| `compute_flow_fields` | on demand (start, nav change); Phase 3 | no | SIM-FLOW-001/003 |

Helpers exported from `movement` for tests and tools: `push_out` (integrate), `Disc`, `accumulate_pushes`, `pair_push` (collision), `seek_velocity` (steer), `mode_speed`, `zone_move_mult`, `slope_mult`, `formation_width`, `tick_dt`, `deg_to_rad` (regiment).

Note on Stage 4 reading the grid: steering at tick *t* uses the grid built at Stage 6 of tick *t−1* (end-of-tick positions of the previous tick). Collision at Stage 7 uses the grid rebuilt at Stage 6 of the same tick and, when it moved anyone, rebuilds it from the pushed positions (T1-044): the grid then always indexes end-of-tick positions, which is exactly what `rebuild_derived` reconstructs after a restore (SIM-DET-005); a grid of pre-collision positions could not be recovered from a snapshot.

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
// As built (T1-040): layout_slots(t, n, ranks, radius, out) dispatches on t.layout; effective_ranks(t, n, requested) clamps to [max(min_ranks, 1), max_ranks] and to n, never below 1;
// files_for(n, ranks) = max(ceil(n / ranks), 1); spacing(t, radius) = (spacing_file, spacing_rank) × 2 radius; ranks_used / files_used read a table back.
// Column widens beyond default_files_column only if it would exceed 255 ranks; Wedge ignores `ranks`; Square uses `ranks` as the depth of each side.
pub fn assign_slots(soldiers: &[AssignSoldier { id, pos, category }], slots: &[Slot], anchor: &Anchor, rules: &FormationRules, prev: &[Option<u16>], out: &mut Vec<Option<u16>>, scratch: &mut AssignScratch);  // SIM-FORM-022; as built (T1-041) the grid it searches is a private one over the *slots* (rings of keep_slot_radius doubling up to assign_search_radius, brute force beyond), rebuilt per call into `scratch`; the soldier grid is not needed
pub fn slot_world(anchor: &Anchor, slot: &Slot) -> V2;   // a + R(θ_a) · o (SIM-FORM-001); `frame(anchor) -> (right, forward)` gives the axes (`forward = (cos θ, sin θ)`, `right = (sin θ, −cos θ)`), `local_to_world(anchor, offset)` the same map
pub fn integrity(regiment: &Regiment, anchor: &Anchor, state: &FormationState, soldiers: &SoldierRead /* SystemParam alias over (&Soldier, &Pos, &SlotRef) */, ids: &Ids, radius: S) -> S;  // SIM-FORM-030, as built (T1-045); `formation_integrity` runs it every integrity_period_ticks with radius = integrity_radius × sf
pub fn set_facing(anchor: &mut Anchor, order: &mut Order, state: &mut FormationState, rules: &FormationRules, sr: S, facing: Angle<S>) -> bool;  // SIM-FORM-024 (T1-045): order.facing becomes the wheel target that regiment_follow_path turns toward at wheel_rate while halted; beyond turn_in_place_angle a halted regiment about-faces instead (anchor to the rear rank's centre, facing + π, reform), returning true
pub struct GroupFormationTemplate { pub id: ContentId, pub kind: GroupKind, pub gap: S, pub skirmishers_forward: bool, pub cavalry_flanks: bool, pub lines: u8 }
pub fn arrange_group(t: &GroupFormationTemplate, regiments: &[RegimentInfo { id, pos, category, count, template, radius }], anchor: V2, facing: Angle<S>, width: S, rules: &FormationRules, regs: &Registries) -> Vec<Placement { id, anchor, facing, ranks }>;  // SIM-FORM-040..042; as built (T1-046): regiments ordered by their anchor's projection on the group's right axis (ties by id), cavalry alternated onto the outer positions, skirmishers `skirmish_offset` ahead; ranks start at each template's minimum and the widest regiment deepens one rank at a time until the line (widths + gaps) fits `width × (1 + width_tolerance)`; double_line alternates regiments into `lines` lines `2 × gap` apart, echelons step successive regiments toward the named flank `2 × gap` back, refused flanks pull the flank regiment `3 × gap` back and turn it 45° inward; output ascending by id
pub fn ranks_for_width(t: &FormationTemplate, count: u16, radius: S, width: S, tolerance: S) -> u8;   // the SIM-FORM-042 loop for one regiment (the UI's single-regiment drag, T1-062)
pub fn lateral_order(regiments: &[RegimentInfo], right: V2, cavalry_flanks: bool) -> Vec<usize>; pub fn arranged_width(..) -> S;   // the pieces of arrange_group, exposed for tests
```

Systems: `formation_layout` (Stage 2, per regiment whose `needs_reform` is set, whose soldier count differs from its slot count, or whose anchor facing moved more than `reform_angle` since the last layout; parallel over regiments when a task pool exists and serial otherwise, each writing only its own `FormationState`), then `formation_apply` (Stage 2, exclusive: writes `SlotRef` and `Rank` to the soldiers in regiment id order), `formation_integrity` (Stage 2, every `integrity_period_ticks`). Resize (SIM-FORM-021) falls out of the assignment: a soldier whose slot vanished takes the nearest free one, so the rearmost soldiers close the front-rank gaps. `rebuild_formation_derived` recomputes slots and `Rank` on restore.

Assignment cost: greedy with grid candidates is O(n × k); swap passes O(n × files). Budget 2 ms for all reforming regiments; benchmark `assign_slots` at n = 500 must be < 0.5 ms (SIM-FORM-023).

Tests: layout functions produce `n` slots, centred front rank, no duplicates; assignment keeps slots within `keep_slot_radius`; group arrangement width within tolerance; golden slot tables for each layout at n ∈ {1, 7, 60, 160, 500}.

## 8. Combat, morale, fatigue, abilities, visibility, AI (`il_sim_battle::combat`, `::morale`, `::abilities`, `::visibility`, `::ai`)

### 8.1 Melee and death

```rust
pub struct CombatRules { pub base_hit: S, pub hit_scale: S, pub min_hit: S, pub max_hit: S, pub min_damage: S, pub engage_radius: S, pub retarget_period_ticks: u16, pub reach_slack: S,
    pub charge_window_ticks: u16, pub charge_dmg_share: S, pub charge_distance: S, pub charge_mass_mult: S, pub brace_integrity: S,
    pub flank_dmg_mult: S, pub rear_dmg_mult: S, pub flank_def_mult: S, pub rear_def_mult: S, pub height_defence: S, pub height_range: S, pub height_ref: S,
    pub second_rank_reach_bonus: S, pub exp_step: S, pub pursuit_hit_mult: S, pub pursue_repath_ticks: u16, pub corpse_ticks: u16, pub attack_move_radius: S,
    pub projectile_cap: u32, pub projectile_radius: S, pub scatter_scale: S, pub direct_apex: S, pub gravity: S, pub shield_mult: S, pub stat_hit_base: S, pub friendly_block_dist: S, pub volley: bool, pub ranged_retarget_ticks: u16 }

pub fn hit_probability(a: S, d: S, r: &CombatRules) -> S;                    // SIM-CMBT-011
pub fn melee_damage(dmg: S, armour: S, pen: S, mults: S, r: &CombatRules) -> S; // SIM-CMBT-013
pub fn attack_arc(defender_facing: Angle<S>, to_attacker: V2, frontal_arc: S) -> Arc;  // SIM-CMBT-014
pub struct AttackOutcome { attacker: SoldierId, target: SoldierId, hit: bool, damage: S, arc: Arc }
```

Systems as built (T2-020): `pursue_update` (Stage 3, exclusive; `combat::pursue`), `melee_gate` (Stage 9, exclusive; one pass over soldiers for each regiment's extent, then anchor-grid queries, into `MeleeGateRes`; SAD T-10), `melee_target` (Stage 9, staggered, par_iter over soldiers reading this tick's grid and the previous tick's `Attackers`, writes only its own `Fsm` and `MeleeState`; second-rank targeting through the slot ahead), `melee_recount` (Stage 9, exclusive: `Attackers` and `Combat.engaged`/`last_fighting` in ascending id, `Engaged` events). Planned: `melee_attack` (Stage 10, par_iter producing `AttackOutcome` into a per-thread buffer, then merged and sorted by attacker id, then applied to `Health`), `resolve_deaths` (Stage 15: soldiers with `hp ≤ 0` sorted by id → `Dead`, regiment soldier lists updated, `deaths_5s` ring, kill credit, events, `needs_reform`).

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
    pub engage_fatigue_ticks: u32, pub safe_radius: S, pub exp_bonus: S, pub w: MoraleWeights, pub state_mults: StateMultsTable }
pub struct StateMults { pub attack: S, pub defence: S, pub interval: S, pub speed: S }              // SIM-MOR-004
pub struct StateMultsTable { pub steady: StateMults, pub unsettled: StateMults, pub shaken: StateMults, pub broken: StateMults, pub routing: StateMults }
impl StateMultsTable { pub fn for_state(&self, discriminant: u8) -> &StateMults; }  // MoraleState as u8; Shattered (5) reads the routing row
pub struct MoraleWeights { pub casualty_rate: S, pub casualty_total: S, pub fatigue: S, pub general_aura: S, pub allies_near: S, pub allies_routing: S, pub high_ground: S, pub fear: S, pub flanked: S, pub outnumbered: S, pub integrity: S, pub engaged_duration: S, pub winning: S, pub recovery: S }
pub fn morale_factors(ctx: &RegimentContext) -> [S; 14];        // x_f per SIM-MOR-010..024, order = MoraleWeights field order
pub fn morale_state(m: S, current: MoraleState, r: &MoraleRules) -> MoraleState;  // SIM-MOR-003 hysteresis

pub struct FatigueRules { pub rate_idle: S, pub rate_walk: S, pub rate_march: S, pub rate_run: S, pub rate_fighting: S, pub rate_routing: S, pub armour_rate: S,
    pub thresholds: [S; 3], pub speed_loss: S, pub attack_loss: S, pub defence_loss: S, pub interval_gain: S }
pub fn fatigue_mults(f: S, r: &FatigueRules) -> FatigueMults;   // SIM-FAT-004

pub struct GeneralRules { pub aura_radius: S, pub aura_attack: S, pub aura_per_rank: S, pub hp_mult: S, pub wounded_hp: S }   // §9
pub struct VisibilityRules { pub period_ticks: u16, pub conceal_radius: S, pub height_bonus: S, pub eye_height: S, pub los_sample: S, pub memory_ticks: u32 }   // §11
pub enum TimeoutWinner { Defender, MostSoldiers }
pub struct BattleFlowRules { pub time_limit_ticks: u32, pub deploy_timeout_ticks: u32, pub pursuit_ticks: u32, pub fled_return_fraction: S, pub timeout_winner: TimeoutWinner, pub exp_per_kill: S, pub exp_survive: S, pub loot_per_enemy_killed: S }   // §12

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
- **Instancing.** One draw per atlas (LOD tiers are Phase 3). Instance layout 32 bytes (as built in T1-051; wgpu has no scalar `f16` vertex format): `pos: [f32; 2]` (projected screen pixels), `depth: f32`, `frame_facing: u32` (atlas column in bits 0..16, facing row in bits 16..24), `tint: [u8; 4]`, `scale: f32`, `flags: u32` (bit 0 selected, bit 1 hovered), `_reserved: u32` (`SpriteInstance`, `SpriteInstance::SIZE`, `pack_frame_facing`). 32k instances = 1 MB per frame, written with `queue.write_buffer` into a ring of 3 buffers. The colour target is 4× MSAA with alpha-to-coverage, resolved to the surface. Sprite sheets are `SpriteSet` content files (`content/sprites/*.json5`: atlas path, frame size, facings as rows, columns as frames, ground origin, named animations) over a PNG under `assets/`; `il_cli genart` generates the placeholder sheets.
- **Interpolation.** `p = lerp(prev, cur, alpha)`; facing snaps when the angle crosses a facing8 boundary (no angular lerp for sprites).
- **LOD.** Phase 3 (REQ-RNDR-004); nothing of it is built in Phase 1. `zoom < z1`: Detailed (full atlas frame, animation); `z1..z2`: Reduced (single frame per state, no animation); `> z2`: Aggregation — one quad per regiment rank block coloured by faction and shaded by density, computed from `FormationState` (REQ-RNDR-004).
- **Terrain.** As built (T1-053): `il_render::terrain::TerrainMesh::build(&LoadedMap, &Registries)` makes one vertex per height sample (`pos`, `height`, `shade` from the finite-difference normal under a fixed north-west light; 16 bytes) and two triangles per `height_cell` cell, plus an `R8Uint` zone-index raster at `zone_cell` (rows padded to 256 bytes; river cells without a `crossing` zone take slot 255 = water) and a 256-entry linear palette from `ZoneType.colour`. `terrain.wgsl` projects vertices with a 64-byte camera uniform that mirrors `Camera::world_to_screen`, writes depth 1.0 with no depth write so every sprite draws over it, and colours fragments from the palette times the shade with 2 m contour lines. Rivers and roads therefore come from the raster rather than separate strips; walls and gates as sprite strips arrive in Phase 5. `Renderer::set_terrain(&TerrainMesh)` uploads once per battle; `Renderer::render(&FrameScene { clear, camera, sprites, lines }, ui)` draws terrain, sprites and lines in one MSAA pass. Sprites take `height` from `LoadedMap::height_at` in `build_snapshot`. A line-list pipeline (`lines.rs`, `LineScene { vertices: Vec<LineVertex { pos, colour }> }`, screen-space, alpha-blended, no depth) draws the deployment outlines (`deployment_outlines`, ground-following, side tint) and serves the debug overlays (T1-054).
- **Debug overlays.** Line list pipeline fed from `BattleView` (nav grid, slots, paths, LOS radii, morale bars) toggled by `DebugFlags`. As built (T1-054): `il_render::debug::build_debug_lines(view, DebugFlags { nav_grid, slots, paths, anchors, spatial_cells }, camera, screen, &mut LineScene)` appends to the frame's line scene after the deployment outlines; every point is projected onto the terrain; grids are clipped to the visible bounds and skipped beyond 40k cells; the app toggles the flags through the `debug_nav_grid`, `debug_slots`, `debug_paths`, `debug_anchors`, `debug_spatial` bindings (F5..F9 by default; F1..F4 are the formation hotkeys) in `dev` builds and shows the enabled ones in the title.
- **Threading.** Phase 1: render on the main thread after the sim step from a `RenderSnapshot` (positions ×2, facings ×2, regiment blocks, projectiles, camera). Phase 3: the snapshot is sent over a channel to a render thread (REQ-RNDR-007); the snapshot type is designed now so only the plumbing changes (T-5).

```rust
// As built (T1-050..T1-054). Planned fields for later phases (projectiles, fog mask, LOD) join RenderSnapshot with their features.
pub struct Renderer { surface, device, queue, config, targets /* MSAA colour + depth, recreated on resize */, terrain_pipe, terrain: Option<TerrainGpu>, sprites: SpritePipeline, lines: LinePipeline, egui: EguiPass, atlases: Vec<Atlas> }
impl Renderer {
    pub fn new(window, size, vsync) -> Result<Self, RenderError>; pub fn resize(&mut self, size); pub fn set_vsync(&mut self, on: bool); pub fn size(&self); pub fn surface_format(&self); pub fn device(&self); pub fn queue(&self);
    pub fn load_atlas(&mut self, png: &[u8], ..) -> Result<AtlasId, AtlasError>; pub fn atlas(&self, id: AtlasId) -> &Atlas;   // `atlas_path` resolves a sprite set's sheet under the mod's assets root; `anim_column` picks the frame
    pub fn set_terrain(&mut self, mesh: &TerrainMesh); pub fn clear_terrain(&mut self); pub fn has_terrain(&self) -> bool;
    pub fn render(&mut self, frame: &FrameScene<'_> { clear: ClearColour /* ClearColour::FIELD */, camera: Option<Camera>, sprites: &SpriteScene, lines: &LineScene }, ui: Option<&mut EguiPaint<'_>>) -> Result<(), RenderError>;   // terrain, sprites and lines in one 4× MSAA pass resolved to the surface, then the egui-wgpu paint pass; `EguiPaint` borrows il_ui's tessellated `UiOutput`
}
pub struct Camera { pub center: Vec2 /* world m */, pub zoom: f32 /* px per m, MIN_ZOOM 2 ..= MAX_ZOOM 96, DEFAULT_ZOOM 12 */, pub rotation: u8 /* 0..=3 quarter turns */, pub pitch: f32 /* 0.5 */, pub elevation: f32 /* 0.8 */ }
impl Camera { pub fn new(center) -> Self; pub fn world_to_screen / screen_to_world / pan_screen / zoom_at / rotate / visible_bounds; pub fn rotate_to_view / rotate_to_world; pub fn facing_index(&self, facing8: u8) -> u8 /* (facing8 + 8 − 2·rotation) mod 8 */ }
pub struct RenderSnapshot { pub tick: Tick, pub alpha: f32, pub camera: Camera, pub soldiers: Vec<SoldierInst>, pub regiments: Vec<RegimentBlock>, pub counts: EntityCounts { soldiers, visible_soldiers, regiments } }
pub struct SoldierInst { pub pos: [f32; 2] /* world, interpolated */, pub height: f32, pub facing8: u8 /* not interpolated: facing snaps */, pub sprite_set: u16, pub side: u8, pub moving: bool, pub selected: bool }
pub struct SnapshotInput<'a> { pub alpha: f32, pub camera: Camera, pub screen: Vec2, pub selected: &'a BTreeSet<RegimentId> }
pub fn build_snapshot(view: &BattleView, input: &SnapshotInput, out: &mut RenderSnapshot);   // T1-052: clears and refills `out` (no per-frame allocation), lerps positions, snaps facing8, culls to camera bounds padded by CULL_PAD_METRES = 4; `height` from `LoadedMap::height_at`
pub struct SetAtlas<'a> { pub atlas: AtlasId, pub set: &'a SpriteSet }
pub fn scene_from_snapshot(snap: &RenderSnapshot, screen: Vec2, time: f32, sets: &[SetAtlas<'_>], out: &mut SpriteScene);   // projection, depth from projected ground y, facing remap, animation column (`SHEET_PIXELS_PER_METRE` = 30), `side_tint(side) -> [u8; 4]`
pub struct SpriteScene { pub batches: Vec<SpriteBatch { atlas, instances: Vec<SpriteInstance> }> }   pub struct LineScene { pub vertices: Vec<LineVertex { pos, colour }> }
pub struct TerrainVertex { pos, height, shade }   pub fn ground_height(map, p) -> f32;
```

Budget: 32k instances at 60 FPS: snapshot build ≈ 1.5 ms, GPU ≈ 2 ms on the target GPU.

Tests: projection round trip; facing index under rotation; snapshot culling, interpolation and selection flags (`crates/il_render/tests/snapshot.rs`); debug line generation (`tests/debug.rs`); the 32k-sprite frame-time check is `il_app --bench-sprites` (T1-051). LOD tier selection and a headless software-adapter frame remain planned with the LOD work (Phase 3).

## 11. UI and input (`il_ui`)

- **Input mapping.** `Bindings` loaded from `content/input/bindings.json5` (REQ-INP-005): `{ action: "select_all", keys: ["Ctrl+A"] }`. As built (T1-061): `il_ui::Bindings::from_content(&InputBindings) -> (Bindings, Vec<BindingError>)` parses chords (`[Ctrl+][Shift+][Alt+]Key`, Modding SDK §4.11) into `Chord { mods, trigger: Key(KeyCode) | Click(b) | DoubleClick(b) | Drag(b) | WheelUp | WheelDown | ModifierOnly }` keyed by `Action`; `InputState` accumulates winit events per frame (`begin_frame(time_seconds)`, `on_window_event(&WindowEvent, consumed_by_egui)` or the granular `key` / `cursor_moved` / `cursor_left` / `button` / `wheel` / `set_modifiers`, then `end_frame`) and recognises gestures itself (a press moving under `DRAG_THRESHOLD_PX` = 4 px is a `Click`, past it a `DragStart`/`DragEnd`; a second click within `DOUBLE_CLICK_SECONDS` = 0.35 s and `DOUBLE_CLICK_PX` = 6 px is `double`); `pressed / held / key_held / wheel_for / gesture / gestures / drag / button_dragging` (each against `&Bindings, Action`) and `gesture_matches` answer the app per frame, with `mods`, `cursor`, `cursor_delta` for raw state; the app hands in wall time, il_ui reads no clock. The planned intent set (`Select`, `AttackMove`, `Ability`, camera, pause and speed intents) did not materialise as intents: camera, pause, speed, selection and control groups are driven by the app straight from bindings, and `UiIntent` covers orders only (below).
- **Selection model.** `Selection { regiments: BTreeSet<RegimentId>, groups: [BTreeSet<RegimentId>; GROUPS = 10] }`, only own faction, only visible. As built (T1-061): `Selection::{new, click(hit, add), box_select(hits, add), set, set_group(n), recall_group(n, add), retain, contains, len, is_empty, clear}`; hit testing lives in `il_ui::pick` (`pick_regiment`, `regiments_in_box`, `regiments_of_type_on_screen`, `own_regiments`, `owned(view, id, player)`) over `BattleView` soldier positions through a `Project<'a> = dyn Fn(V2) -> Vec2 + 'a` projection closure the app builds from `Camera` and `ground_height`, so il_ui never depends on il_render; only regiments whose side belongs to the local player are returned; a soldier's hit circle is centred half a body above its ground point with radius `max(6 px, 1.5 × drawn radius)`.
- **Command emission.** `UiIntent → Command` with `tick = now + 1 + input_delay`, `seq` from a per-player counter. Drag-formation → `GroupFormation` if > 1 regiment else `Move { facing }` and `SetFormation { ranks }` derived per SIM-FORM-042. As built (T1-062): `il_ui::orders::drag_formation(from, to, centroid, flip) -> Option<DragFormation { anchor, forward, width }>` works in world metres (the app unprojects the screen points): `anchor` is the drag midpoint, `width` its length (under `MIN_DRAG_WIDTH_M` = 1 m is no gesture), `forward` the perpendicular pointing away from the selection's anchor centroid (`selection_centroid`), negated by `flip` (the `order_flip_facing` modifier); `DragFormation::facing() -> Angle<S>`. `UiIntent::{Move { target }, DragFormation(DragFormation), Halt, Formation(u8), SpeedMode(SpeedMode)}` and `commands_for(intent, &OrderContext { view, regiments, speed })` return the `CommandKind`s in queue order: a single-regiment drag gives `SetFormation { ranks: Some(il_sim_battle::ranks_for_width(..)) }` then `Move { facing }`, a multi-regiment drag `SetSpeedMode` (the run toggle; `GroupFormation` moves at each regiment's current order speed) then `GroupFormation` with `battle_line_template` (the registry's first `battle_line` template); `Formation(n)` is one `SetFormation { ranks: None }` per distinct n-th template of the selected unit types; `BattleSession::queue` stamps `tick + 1 + input_delay` and the per-player `seq`.
- **Overlays.** `il_ui::overlay::{selection_box, drag_formation_preview}` draw the box-select rectangle and the drag-formation preview through egui's painter.
- **Panels (egui).** Planned: battle regiment cards (top), command card (bottom), minimap with fog (bottom-right, rendered from `FogMask` and regiment blocks into an egui texture), clock and speed (top-right), casualties; deployment tray, zone outline, confirm; campaign province, settlement, army, diplomacy, research, faction, turn log, end-turn; menus: main, custom battle (map, sides, roster builder from registries), settings (bindings, audio, video), load/save (headers from `il_save`). As built (T1-070): `main_menu(ctx, &MenuModel) -> Option<MenuChoice>`, `battle_hud(ctx, &HudModel) -> Option<HudAction>` (`clock(tick) -> String` as `mm:ss`, speed, pause, menu, the `SelectedRegiment` card), `event_panel(ctx, &[EventLine])`, `profiler_overlay(ctx, &ProfilerStats { stages: Vec<StageStat { name, last_ms, mean_ms, max_ms }>, tick_last_ms, tick_mean_ms, tick_max_ms, ticks_sampled, frame_ms, fps, soldiers, regiments, visible_soldiers, ticks_last_frame, accumulator_alpha })`; `UiContext` / `UiOutput` wrap egui-winit so the app hands the tessellated output to il_render's `EguiPaint`. The rest arrives with its phase.
- **Localisation.** All labels via `Locale::get`; `il_app --show-keys` shows keys.

Budget: egui ≈ 1 ms per frame; minimap texture regenerated every 10 frames.

Tests: gesture geometry (drag vector → facing, width) and intent → command conversion on the ten-regiment scenario (`crates/il_ui/tests/orders.rs`); picking (`tests/pick.rs`); binding parse and selection rules inline in `bindings.rs` and `selection.rs`.

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
// As built (T1-070); Campaign and Editor states, `replay: Replay` and `net: Option<LockstepSession>` join with their phases.
pub enum AppState { MainMenu(MenuState { scenarios: Vec<PathBuf>, mods: Vec<PathBuf>, error: Option<String> }), Battle(Box<BattleSession>) }
pub enum Transition { StartBattle(PathBuf), QuitToMenu }
pub struct BattleSession { world: BattleWorld, accumulator: f64, speed: f32, paused: bool, local_player: PlayerId, input_delay: u32 /* 0 in Phase 1 */, next_seq: u16, pending: Vec<Command>, script: ScriptedCommands, command_log: Vec<Command>, events: VecDeque<EventLine> /* EVENT_RING = 256 */ }
impl BattleSession { pub fn new(world, local_player, script); pub fn queue(&mut self, kind: CommandKind); pub fn target_tick(&self) -> Tick /* tick + 1 + input_delay */; pub fn advance(&mut self, dt: f64) -> Vec<StepOutput>; pub fn advance_with(&mut self, dt: f64, observer: &mut dyn StageObserver) -> Vec<StepOutput>; pub fn alpha(&self) -> f32; pub fn speed / set_speed(f32) /* records SetSpeed { mult_x100 } */; pub fn paused / set_paused(bool) /* records Pause */; pub fn local_player; pub fn command_log; pub fn events }
pub const TICK: f64 = TICK_SECONDS; pub const MAX_CATCHUP_TICKS: u32 = 4;
pub struct Profiler;   // the app's StageObserver over `Instant` (SAD §9.3): `frame(frame_seconds, ticks_stepped)`, `stats() -> ProfilerStats` over a 60-tick window
```

Frame: poll winit → `il_ui` intents → commands stamped `tick + 1 + input_delay` into `pending` → `accumulator += dt × speed` (capped at `MAX_CATCHUP_TICKS` = 4 ticks, a constant in `session.rs` rather than a rules field) → while `accumulator ≥ TICK`: gather commands for `world.tick()+1` (local pending, AI internal, network) → `step` → route events to audio/UI/replay → `accumulator −= TICK` → build `RenderSnapshot(alpha)` → render → egui. Campaign state runs `apply` on intents and `end_turn` on End Turn; `BattleRequested` switches state; `Ended` returns the result via `resume_after_battle` or the auto-resolve path.

Tests (inline in `state.rs`, `session.rs`, `profiler.rs`): accumulator never runs more than the cap; pause records a `Pause` command; state transitions; the profiler window.

As built (T1-070): `il_app::state::AppState::{MainMenu(MenuState), Battle(Box<BattleSession>)}` with `AppState::apply(self, Transition::{StartBattle(path), QuitToMenu}, start, menu)` a pure function (a failed start keeps the menu up with the error); `MenuState::scan(scenarios_dir, mods)` lists `*.json5` under `--scenarios-dir` (default `tests/scenarios`) and the mod roots; the menu is `il_ui::main_menu`, the battle HUD (`mm:ss` clock, speed, pause, menu, the selection card with localised unit and formation names) `il_ui::battle_hud`, and `il_ui::event_panel` shows the session's 256-entry event ring (`BattleSession::events`, the routing stub: every `BattleEvent` and rejected command as text) in `dev` builds with the profiler. `BattleSession { world, accumulator: f64, speed: f32, paused, local_player, input_delay: u32 (0 in Phase 1), next_seq: u16, pending, script: ScriptedCommands, command_log, events }`; `queue(kind)` stamps `tick + 1 + input_delay` and the per-player `seq`; `advance_with(dt, observer)` caps the accumulator at `MAX_CATCHUP_TICKS = 4` ticks and returns one `StepOutput` per tick; `alpha()` feeds `build_snapshot`. A scenario on the command line starts in `Battle`; the `quit_to_menu` binding (Escape) or the HUD's Menu button returns to the menu and drops the session; transitions apply after the frame's render. Command line: `il_app [scenario.json5] [--content-root game] [--mod DIR]... [--scenarios-dir tests/scenarios] [--threads 1] [--bench-sprites] [--show-keys]` (`Launch { content_root, mods, scenarios_dir, threads, bench_sprites }`). The `dev` feature (on by default) starts `il_data::HotReload` over the mod roots and polls it every frame, shows the profiler and event panels (`toggle_profiler`, F12) and the F5..F9 debug overlays; `cargo build -p il_app --no-default-features` is the shipping configuration and CI builds both.

## 16. Editors (`il_editor`)

- **Map editor (Phase 3).** Operates on `MapDef` (`id`, `name_key`, `size`, `campaign_terrain_tags`, `weather_allowed`, heightmap `Vec<f32>` at `height_cell` stored as a 16-bit raw sidecar, `base_zone`, `zones` polygons (fords and bridges are polygons of a `crossing: true` zone type laid over a river), `rivers`, roads, `deployment` polygons, `reinforcement_edges`, and the reserved `structures` and `siege_points` lists; Modding SDK §6.1 shows the JSON5). Tools: raise/lower/smooth height brush, zone paint brush, polyline tool, polygon tool, piece placement; live nav grid preview; save to `content/maps/<id>.json5` plus the `.hgt` sidecar for the heightmap (JSON5 stores the reference and cell size; `il_cli genmap` writes the Phase 1 test map the same way).
- **Unit and formation editors (Phase 6).** egui property grids over `Registry<UnitType>` and `Registry<FormationTemplate>` entries with schema-driven widgets (from the JSON Schema `description`/ranges); preview panel renders a formation at chosen `n`; save writes the item into the selected mod folder with a `$override: "merge"` diff if it derives from another mod's item.

## 17. Testing and CI

| Test | Location | Runs | Requirement |
|---|---|---|---|
| Unit tests per formula | each crate | every push | REQ-TEST-001 |
| Determinism: each scenario twice, 1 thread and 8 threads, snapshot/restore at mid-point | `tests/tests/determinism.rs`, in-process on `BattleWorld` (`set_threads(1)` and `set_threads(8)`), plus an in-process `il_cli::run` twice comparison; CI also diffs two `il_cli run --hash-every 1000` logs | every push | REQ-TEST-002 |
| Content validation of `game/` | `tests/content.rs` | every push | REQ-TEST-005 |
| Scenario outcome bands (Simulation Spec §15.3), 50 seeds | `il_cli bands tests/scenarios/bands` (`il_cli::bands`: per file, per seed a single-threaded `BattleWorld` fed the scripted commands, seeds spread over `--jobs` OS threads; assertions evaluated over `BattleView` rows), driven in-process by `tests/tests/scenarios.rs` (`#[ignore]`; the non-ignored test parses every band file on each push) | nightly (`.github/workflows/nightly.yml`) and on demand | REQ-TEST-004 |
| Benchmarks per stage at 2k/10k/20k against the budget table at the top; fail at +20 % over the baseline | `il_cli bench` (per-stage mean/p95/max through `StageObserver`; `--baseline benches/baseline.json --strict`; the `StageTimer` is il_cli's one allowed `Instant` user because it only observes stage boundaries) plus criterion micro-benches in `benches/benches/` (`spatial`, `formation`, `nav`, `layout`, `tick`; T1-080) | every push, warn-only on CI runners; `--strict` on the target machine (`docs/evidence/phase1/machine.md`) | REQ-TEST-003, REQ-PERF-005 |
| Replay verify | `il_cli replay --verify` on recorded replays in `tests/replays/` | nightly | REQ-SAVE-005 |
| Cross-machine hash compare | manual runbook, `il_cli run --hash-log` on two machines and `il_cli desync-report` | before Phase 7 | REQ-TEST-006 |

`il_cli` subcommands: `run <scenario.json5> --ticks N [--hash-every K] [--threads T] [--snapshot-at T] [--restore-from F] [--mod DIR]...` (a scenario is a `BattleSetup` plus an optional `commands: [Command]` list fed by tick, T1-081; a restored run skips the commands up to the snapshot tick), `bench --soldiers N --ticks T [--threads] [--json F] [--baseline F] [--strict] [--record-baseline F --machine M --recorded D]` (T1-080; the setup is generated in code: `N / 200` regiments of alternating infantry on `rome:test_field` with a 600-tick move/reform script), `replay <file> --verify`, `validate <mods...>`, `bands <dir|file> [--seeds N] [--max-ticks T] [--jobs J] [--json F] [--content-root D] [--mod DIR]...` (T2-110; exit 1 when an active assertion fails), `desync-report <log_a> <log_b>`, `autoresolve <setup.json5>`, `genart [--mod-root]` (placeholder sprite sheets, T1-051), `genmap [--mod-root] [--id] [--seed]` (the deterministic Phase 1 test map and its heightmap, T1-030).

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
