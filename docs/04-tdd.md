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

Budget table (sum must fit 50 ms at P3, 25 ms at P2). The Phase 1 columns are the T1-083 means from `il_cli bench --ticks 600` (release, 8 threads, the move/reform script) when stages 8 to 16 held only a placeholder; the Phase 2 columns are the same three generated runs re-recorded with every combat system live (T2-111, 2026-09-07, `benches/baseline.json`) plus the 10k fight (`bench --scenario tests/scenarios/perf_10k.json5 --ticks 1200`: 10,042 soldiers, 52 regiments, side 0 attack-moving into the engine AI; `docs/evidence/phase2/bench_perf_10k.md`), all on the target machine in `docs/evidence/phase2/machine.md`. Milliseconds, means.

| Stage | Budget at 20k (ms) | Phase 1 2k | Phase 1 10k | Phase 1 20k | Phase 2 2k | Phase 2 10k | Phase 2 20k | Phase 2 10k fight | Section |
|---|---|---|---|---|---|---|---|---|---|
| 0 ApplyCommands | 0.2 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 | §4 |
| 1 AI | 2.0 | 0.03 | 0.05 | 0.07 | 0.00 | 0.00 | 0.00 | 0.15 | §8.5, §9 |
| 2 Formation | 2.0 | 0.17 | 0.35 | 0.58 | 0.15 | 0.47 | 0.80 | 0.44 | §7 |
| 3 RegimentMovement | 1.0 | 0.12 | 0.25 | 0.47 | 0.10 | 0.30 | 0.54 | 0.37 | §6 |
| 4 SoldierSteering | 8.0 | 1.03 | 3.56 | 7.74 | 0.65 | 3.43 | 7.81 | 3.33 | §6 |
| 5 Integrate | 0.5 | 0.10 | 0.19 | 0.33 | 0.09 | 0.21 | 0.37 | 0.22 | §6 |
| 6 SpatialGrid | 2.0 | 0.14 | 0.25 | 0.49 | 0.16 | 0.69 | 1.69 | 0.74 | §5 |
| 7 Collision | 8.0 | 1.94 | 5.61 | 11.16 | 1.51 | 6.01 | 12.13 | 4.42 | §6 |
| 8 Visibility | 1.0 | 0.07 | 0.10 | 0.11 | 0.01 | 0.01 | 0.03 | 0.03 | §8.4 |
| 9 Targeting | 4.0 | 0.06 | 0.06 | 0.08 | 0.28 | 1.34 | 3.03 | 1.51 | §8.1 |
| 10 Combat | 4.0 | 0.04 | 0.05 | 0.06 | 0.14 | 0.26 | 0.45 | 0.40 | §8.1 |
| 11 Projectiles | 3.0 | 0.04 | 0.04 | 0.06 | 0.00 | 0.00 | 0.01 | 0.01 | §8.2 |
| 12 Abilities | 0.5 | 0.03 | 0.04 | 0.05 | 0.00 | 0.01 | 0.01 | 0.01 | §8.3 |
| 13 Fatigue | 0.5 | 0.03 | 0.03 | 0.05 | 0.12 | 0.34 | 0.64 | 0.36 | §8.3 |
| 14 Morale | 1.0 | 0.03 | 0.03 | 0.04 | 0.13 | 0.61 | 1.31 | 0.58 | §8.3 |
| 15 Death | 1.0 | 0.03 | 0.03 | 0.04 | 0.03 | 0.11 | 0.23 | 0.11 | §8.1 |
| 16 BattleFlow | 0.3 | 0.03 | 0.03 | 0.03 | 0.00 | 0.00 | 0.00 | 0.01 | §4 |
| 17 Events + Hash | 3.0 | 0.12 | 0.49 | 1.07 | 0.15 | 0.64 | 1.41 | 0.65 | §2, §4 |
| **Total** | **42.0** | **4.03** | **11.17** | **22.43** | **3.52** | **14.45** | **30.50** | **13.33** | Phase 2 p95 tick 5.4 / 20.1 / 43.5 / 18.3 ms |

REQ-PERF-002 (P2: 10,000 soldiers at tick ≤ 25 ms) holds with margin: the 10k fight's tick is 13.3 ms mean, 18.3 ms p95, and every stage is inside its 20k budget at 10k. The generated 20k run (no enemy) sits at 30.5 ms, inside the 50 ms P3 bar, with Collision (12.1 ms against 8) and SoldierSteering (7.8 against 8) over their budgets as in Phase 1 and Targeting at 3.0 ms (the per-soldier pass of `melee_target` and the gate's table scan run for every soldier, enemy or not): the Phase 3 items of SAD §12 T-10. The combat stages cost what their budgets allow: at 10k in the fight Targeting 1.5, Morale 0.6, Fatigue 0.4, Combat 0.4, Events + Hash 0.65 ms.

Phase 3 pass (T3-022, T3-023; 2026-09-11, the same machine, `docs/evidence/phase3/machine.md`): at 20k on the generated run Collision 3.1 ms (was 11.3 in the T3-021 recording), SoldierSteering 4.8 (6.7), and Stage 9 with no enemy in sight 1.4 ms against 3.3 before (`melee_gate` 0.56, `ranged_target` 0.14, `melee_target` 0.68, `melee_recount` 0.03 against 1.95; the same split in the 10k fight at tick 1,000 gives 1.4 ms against 2.3). Formation stays at its T3-021 mean of 0.73 ms (p95 2.9: every regiment reforms on the script's move ticks; SIM-FORM-020 gives no licence to spread a reform over ticks, so the cost stays) and Events + Hash at 1.3 ms; the 32k hash cost is measured by T3-025's cap run. The 20k and 32k columns of the table are written by T3-080 from the bench keys.

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

Phase 0 pins (T0-003) are the versions in the table; later phases pin their own crates when they arrive and update this table. Phase 1 pins (T1-050) are the newest mutually compatible set on 2026-09-03; `egui-wgpu` 0.36 requires `wgpu` ^30 and `egui-winit` 0.36 requires `winit` ^0.30.13. Phase 3 pins (T3-001) are the newest *stable* mutually compatible set on 2026-09-10: `egui`, `egui-wgpu`, `egui-winit` 0.36.2, `glam` 0.33.7, `jsonschema` 0.56 (its 0.54–0.56 breaking changes are all in the `canonical` module, which il_data does not use), the toolchain 1.98.1, and a `cargo update` of the transitive lock (31 patch bumps); `bevy_ecs`/`bevy_tasks` 0.19.1, `wgpu` 30.0.1, `kira` 0.12.4, `png` 0.18.1 and `criterion` 0.8.2 were already the newest. Skipped as prereleases: `winit` 0.31.0-beta (egui-winit 0.36 needs winit ^0.30 anyway) and `notify` 9.0.0-rc. `tracing` left il_sim_battle in the same pass (SAD §12 T-11). The `il_cli run` hash logs of `idle_1000`, `move_reform_2000` and `perf_10k` were identical before and after the bump, so no floating-point result moved; the baseline was re-recorded the same day (`docs/evidence/phase3/machine.md`).

| Crate | Version (initial) | Why | Used by |
|---|---|---|---|
| `bevy_ecs` | 0.19.1 (feature `multi_threaded`) | standalone ECS with schedules and parallel executor | il_sim_battle, benches |
| `bevy_tasks` | 0.19.1 | `ComputeTaskPool` for `BattleWorld::set_threads` | il_sim_battle |
| `wgpu` | 30.0.1 | GPU API | il_render |
| `winit` | 0.30.13 | window and input events | il_app, il_ui (event types; a direct dependency so `cargo test -p il_ui` unifies winit's features like il_app) |
| `egui`, `egui-wgpu`, `egui-winit` | 0.36.2 (0.36.1 in Phase 1) | UI (`egui-wgpu` paint pass lives in il_render) | il_render (`egui`, `egui-wgpu` with feature `winit`), il_ui (`egui`, `egui-winit` without default features), il_app (`egui`) |
| `serde` (feature `derive`) | 1 | serialisation | il_core, il_data, il_sim_battle, il_cli |
| `json5` | 1.3 | test fixtures only since T1-081; scenarios and content go through `il_data::json5`, a span-carrying parser written in T1-020 because per-field positions are needed for diagnostics and merge provenance (OQ-7 amended) | il_sim_battle, il_render, il_ui (dev-dependencies) |
| `semver` | 1 | manifest versions and ranges | il_data |
| `serde_json` | 1 | save headers, schema validation input | il_data, il_sim_battle, il_cli, tests (il_save when it arrives) |
| `jsonschema` | 0.56 (`default-features = false`; 0.53 in Phase 2) | content validation, draft 2020-12 | il_data |
| `postcard` | 1.1 (feature `use-std`) | snapshot encoding (OQ-2 resolved in Phase 0) | il_sim_battle (il_save when it arrives) |
| `mlua` (`lua54`, `vendored`) | 0.10 | Lua | il_script (Phase 6; not in the workspace yet) |
| `glam` | 0.33.7 (0.33.6 in Phase 1) | render-side math only (never in sim) | il_render, il_ui, il_app |
| `png`, `bytemuck`, `pollster` | 0.18.1 / 1 / 0.4 | atlas files, GPU buffer casts, blocking on device creation | il_render; il_cli uses `png` only (`genart`) |
| `xxhash-rust` (`xxh3`) | 0.8 | state hash | il_core |
| `tracing` | 0.1 | one `warn!` in `Locale` for a missing key; no subscriber is installed yet (il_sim_battle declared it unused until T3-001 dropped it, SAD §12 T-11) | il_data |
| `criterion` | 0.8.2 | benchmarks (`benches/benches/*.rs`, `harness = false`; first bench in T1-031) | benches (dev-dependency; `il_cli` is a dev-dependency too, for the generated bench setups) |
| `kira` | 0.12.4 (default features: cpal, symphonia with wav) | audio (OQ-8: chosen for game-oriented mixing; T2-100) | il_audio; `il_cli gensound` writes its WAV placeholders with a hand-rolled 44-byte header and no audio dependency |
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
// As built (T1-020..T1-025; `Registries.abilities: Registry<Ability>` since T2-050; `ai_action_sets: Registry<AiActionSet>` and `ai_profiles: Registry<AiProfile>` since T2-080, `il_data::ai`). Kinds for later phases (technologies, buildings, sound sets) join `Registries` with their phases.
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
    pub factions: Registry<Faction>, pub zones: Registry<ZoneType>, pub maps: Registry<MapDef>, pub sprite_sets: Registry<SpriteSet>, pub sound_sets: Registry<SoundSet> /* T2-100, audio only */,
    pub abilities: Registry<Ability>, pub ai_action_sets: Registry<AiActionSet>, pub ai_profiles: Registry<AiProfile>,   // T2-050, T2-080; `Faction.ai_profile_handle` resolves to a profile
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
        pub general: GeneralSetup, pub regiments: Vec<RegimentSetup>, pub reinforcements: Vec<ReinforcementGroup>,
        pub ai_profile: Option<ContentId> /* overrides the faction's profile for the engine AI (T2-080); `SetupError::UnknownAiProfile` */ }
    pub struct RegimentSetup { pub id: u32 /* campaign regiment id, echoed in result */, pub unit_type: ContentId,
        pub count: u16, pub experience: u8, pub fatigue: f32 /* data-side f32, converted */, pub formation: Option<ContentId>,
        pub position: Option<[f32; 2]>, pub facing_deg: Option<f32> /* the pre-deploy override (PRD OQ-9, T2-070): a placed regiment starts deployed, a side with every regiment placed starts confirmed */,
        pub units: Vec<UnitGroupSetup { unit_type: ContentId, count: u16, experience: u8 }> /* T3-002 (SIM-FORM-012), the sim reads it from T3-040: the ordered unit groups of a mixed regiment; `unit_type` + `count` + `experience` become `Option`s at the serde level then and are the single-group shorthand; `BattleWorld::new` rejects both forms together (`SetupError::BothUnitForms`), an empty list (`EmptyComposition`) and an unknown group unit (`UnknownUnit`); `RegimentSetup::groups() -> Vec<UnitGroupSetup>` normalises either form */ }
    // `map_id` is required (T1-030); `BattleWorld::new` fails with `SetupError::UnknownMap`, `MissingDeploymentZone` or `PositionOutOfMap`.
    pub const SOLDIER_CAP: u32 = 32_768;                  // `BattleSetup::soldier_total()` (initial plus reinforcements) above it is `SetupError::OverCap` (SIM-CORE-006)
    pub enum Weather { Clear, Rain, Fog }   pub struct VictoryRules { pub timeout_winner: Option<u8> }
    pub struct GeneralSetup { pub unit_type: ContentId, pub rank: u8, pub name_key: String, pub bodyguard: Option<u32> }   // T2-043
    pub struct ReinforcementGroup { pub arrival_tick: u32 /* since the Battle phase began */, pub edge: MapEdge /* listed by the map for the side's zone */, pub regiments: Vec<RegimentSetup> }   // T2-070
    pub struct Scenario { #[serde(flatten)] pub setup: BattleSetup, pub commands: Vec<Command>, pub determinism: Option<DeterminismBudget { ticks, snapshot_at }> /* the determinism test's budget for the file, default 10,000 / 5,000; T2-111 */ }   // a scenario file (T1-081); `script() -> ScriptedCommands`
    pub struct ScriptedCommands { .. }   // sorted by (tick, player, seq); `take_for(tick) -> Vec<Command>` hands over everything stamped `tick` or earlier (stale ones too, so the sim rejects them visibly), `remaining`, `is_empty`
    pub struct BattleResult { pub winner: Option<u8>, pub duration_ticks: u32, pub sides: Vec<SideResult>, pub summary: BattleSummary { total_killed, total_fled } }
    pub struct SideResult { pub regiments: Vec<RegimentResult>, pub general_fate: GeneralFate, pub loot: i64 }
    pub struct RegimentResult { pub id: u32, pub initial: u16, pub survivors: u16, pub fled: u16, pub killed: u16, pub experience_gain: u16, pub ammo_left: u16, pub arrived: bool /* false for a reinforcement group that never entered, T2-070 */,
        pub units: Vec<UnitGroupResult { unit_type: ContentId, initial: u16, survivors: u16, killed: u16, fled: u16 }> /* T3-002 (SIM-FORM-015), filled from T3-041: one row per unit group in setup order, summing to the totals; a single-group regiment has one row */ }
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
    // `result(&self) -> BattleResult` (T2-071): the result as it stands now (`result::compute`), `winner` only once the phase is Ended; `Ended { result }` carries the final one
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
pub struct StepOutput { pub hash: StateHash, pub events: Vec<BattleEvent>, pub rejected: Vec<(Command, RejectReason)>, pub ai_commands: Vec<Command> /* queued at Stage 1 for the next tick (SIM-CMD-005, T2-080); `set_ai_enabled(false)` / `ai_enabled()` let a replay feed them instead of re-running the AI */ }

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
    VolleyFired { regiment: RegimentId, count: u16 } /* T2-030 */, ProjectileLanded { pos: V2, hit: bool },
    FireBlocked { regiment: RegimentId, blocker: RegimentId } /* SIM-PROJ-009, T2-030 */,
    Charge { regiment: RegimentId, target: RegimentId }, Engaged { regiment: RegimentId },
    MoraleState { regiment: RegimentId, from: MoraleState, to: MoraleState },
    Rallied { regiment: RegimentId }, Shattered { regiment: RegimentId }, SoldierFled { id: SoldierId, regiment: RegimentId, pos: V2 },  // T2-042; `MoraleState` is `MoraleChanged` as built (T2-041)
    GeneralDied { side: u8, soldier: SoldierId },  // T2-043
    AbilityUsed { regiment: RegimentId, ability: ContentId, targets: u8 }, StatusExpired { regiment: RegimentId, ability: ContentId },  // T2-050 (ids, not handles: events are serialised)
    PhaseChanged { from: BattlePhase, to: BattlePhase },
    CommandRejected { command_seq: u16, player: PlayerId, reason: RejectReason }, ReinforcementsArrived { side: u8 },
    Ended { result: Box<BattleResult> },
    DeploymentConfirmed { side: u8 }, Surrendered { side: u8 }, Withdrawing { regiment: RegimentId }, SoldierWithdrew { id: SoldierId, regiment: RegimentId, pos: V2 }, ReinforcementsDropped { side: u8, group: u8 },  // T2-070
}
pub enum BattlePhase { Deployment, Battle, Pursuit, Ended }
pub enum SpeedMode { Walk, Run, March }
pub enum FireMode { FireAtWill, Hold, Target(RegimentId) }
pub enum AbilityTarget { SelfTarget, Point(V2), Regiment(RegimentId) }
pub enum RejectReason { StaleTick { command_tick: Tick, current: Tick }, UnknownRegiment(RegimentId), NotOwner(RegimentId), Routing(RegimentId), WrongPhase, UnknownContent(ContentId), FormationNotAllowed { regiment: RegimentId, template: ContentId }, InvalidTarget(RegimentId) /* AttackRegiment or FireMode::Target at an own-side or empty regiment, T2-020/T2-030 */, NotRanged(RegimentId) /* FireMode at a unit without `ranged`, T2-030 */, Ability { regiment: RegimentId, ability: ContentId, why: AbilityFail } /* T2-050 */, NotVisible(RegimentId) /* a hidden enemy named as a target, T2-060 */, OutsideDeploymentZone { regiment: RegimentId } /* T2-070 */, NotImplemented /* unreachable since T2-070; kept so a future kind is never dropped */ }
pub enum AbilityFail { NotOwned, OnCooldown, NoEnergy, BadTarget, OutOfRange, Engaged, Moving }   // SIM-ABIL-003 (T2-050)
```

### 4.3 Components (soldier-level, SoA via bevy_ecs tables)

| Component | Fields | Hashed | Interpolated |
|---|---|---|---|
| `Soldier` | `id: SoldierId, regiment: RegimentId, unit: Handle<UnitType>, category: UnitCategory`; T3-040 adds `group: u8` (the index into `Regiment.units`, SIM-FORM-012, so two groups of one unit type reconcile in the result; identity like `id`, stored in `SoldierSnap.group`) | id only | — |
| `Pos` | `p: V2` | yes | yes (`PrevPos` written at Stage 17) |
| `Vel` | `v: V2` | yes | — |
| `Facing` | `theta: Angle<S>` | yes | yes (`PrevFacing`) |
| `Body` | `r: S, m: S` (`m` is `unit.mass × charge_mass_mult` inside a charge window, restored by `rebuild_derived`; T2-021) | no (derived) | — |
| `Health` | `hp: S` | yes | — |
| `FatigueC` | `f: S` | yes | — |
| `SlotRef` | `slot: Option<u16>` | yes | — |
| `Fsm` | `state: SoldierState, since: Tick` | yes | — |
| `MeleeState` (T2-020) | `target: Option<SoldierId>, cooldown: u16` | yes | — |
| `Attackers` (T2-020) | `n: u8`, soldiers targeting this one; recounted after targeting and on restore | no (derived) | — |
| `RangedState` (T2-030) | `ammo: u16, cooldown: u16`; present only on soldiers whose unit has `ranged` (ammo from `unit.ranged.ammo`; the cooldown counts only when `combat.volley` is false) | yes | — |
| `Rank` | `rank: u8, file: u16` | no | — |
| `GeneralTag` (T2-040 declared, T2-043 spawned) | `rank: u8`, only on the side's general | yes (as `Option<u8>` per soldier) | — |
| (`Dead` marker) | not used: the dead are despawned at Stage 15 in the same tick, so no query ever filters them (T2-022) | — | — |

Regiment-level components live on regiment entities (≈ 200). As built (Phase 1): `Regiment { id, side: u8, setup_id: u32, unit: Handle<UnitType>, soldiers: Vec<SoldierId> (ascending) }` (one unit type per regiment until Phase 3, plan S15; the Phase 1 `ammo: u16` moved to the per-soldier `RangedState` in T2-030; T3-002 specifies and T3-040 builds `units: Vec<UnitGroup { unit: Handle<UnitType>, count: u16, experience: u8 }>` in setup order, `unit` staying the first group's for the code that reads one unit, with `RegimentSnap.units` as ContentIds and a `SNAPSHOT_VERSION` bump in T3-040), `Anchor { pos: V2, facing: Angle<S> }`, `FormationState { template, ranks: u8, files: u16, slots: Vec<Slot> (derived), assignment: Vec<Option<u16>>, integrity: S, morph_until: Tick, needs_reform: bool, prior_template: Option<Handle<FormationTemplate>> (corridor morph), laid_out_facing: Angle<S>, dirty: bool }` (`FormationState::new(template, ranks, slots, facing)`), `Order { kind: OrderKind, target: V2, facing: Option<Angle<S>>, speed: SpeedMode, since: Tick }` (`OrderKind::moves()`), `Path { waypoints: Vec<Waypoint { p: V2, corridor: S }>, next: u16, requested: bool }` (stored, hashed and snapshotted, T1-032; `is_active()`, `current()`), `Morale { m: S, state: MoraleState, deaths_5s: [u16; DEATHS_RING = 100], initial: u16, rout_count: u8, engaged_since: Tick, arc_hit: [Tick; 3] }` (the ring and `initial` since T2-020, written by death; `rout_count`, `engaged_since` and the last front/flank/rear attack ticks declared in T2-040 and written from T2-041/042), `RegimentFatigue { mean: S }` (SIM-FAT-005, refreshed every ten ticks at Stage 13; hashed and snapshotted, T2-040), `Combat { engaged: bool, last_fighting: Tick, charge_until: Tick, experience: u8, kills: u32, fled: u16, withdrawn: u16 }` (T2-020; `fled` since T2-040, written from T2-042; `withdrawn` since T2-050, written from T2-070; hashed and snapshotted), `Order` additionally carries `target_regiment: Option<RegimentId>` (T2-020). `SoldierState` is `Idle | MoveToSlot | Fighting | Routing | Withdrawing | Dead`. T2-030 adds `Fire { mode: FireMode, target: Option<RegimentId>, cooldown: u16 }` (present only on regiments whose unit has `ranged`; hashed and snapshotted; the retarget tick is the stagger `tick % ranged_retarget_ticks == id % ranged_retarget_ticks`, so no `retarget_at` field is stored); T2-050 adds `Energy { e: S }` (SIM-ABIL-006), `Statuses { list: Vec<StatusEffect { source: Handle<Ability>, remaining: u16, stacks: u8, hostile: bool }>, mults: StatMults }` (the list hashed by the source ability's ContentId and snapshotted in application order; `mults` derived, refreshed when the list changes and on restore) and `Cooldowns(Vec<u16>)` (one entry per ability slot, `abilities::slots`: the unit's abilities then its living general's; hashed length-prefixed, snapshotted); experience stays inside `Combat`, and visibility is the per-side `Visibility` resource (T2-060), not a component.

Projectiles (T2-030): projectiles are not entities. `Projectile { id: ProjectileId, shooter: SoldierId, shooter_regiment, side: u8, launch_tick, land_tick, start: V2, end: V2, apex: S, arc: ProjectileArc, damage: S, pen: S }` lives in the `Projectiles(Vec<Projectile>)` resource, reserved once at `combat.projectile_cap` and never grown past it (REQ-PERF-008's pooling policy), ascending by id (new ids appended in shooter order, landed ones removed in place). The position at tick `t` is `start + (end − start) × u` and the height `apex × 4u(1 − u)` with `u = (t − launch) / (land − launch)` (`Projectile::position_at`, `height_at`, `progress`), so nothing is integrated and a restore rebuilds nothing.

### 4.4 Resources

As built (Phase 1): `Clock { tick }`, `Phase`, `Sides(Vec<SideState { player, faction, deployment_zone, deployment_confirmed, defeated, surrendered, reinforcements_spawned: u8, escape_edge: MapEdge, general: Option<SoldierId>, general_regiment: Option<RegimentId>, general_dead: bool }>)` (the general fields since T2-040, written by T2-042/043; `surrendered` and `reinforcements_spawned` since T2-050, written by T2-070; every field but `player`, `faction` and `deployment_zone` hashed right after the battle flow), `BattleFlow { battle_start: Tick, pursuit_start: Tick, winner: Option<u8>, ended_at: Tick }` (T2-050 layout, written by T2-070, `ended_at` since T2-071; hashed right after the phase, snapshotted), `Visibility { masks: Vec<Vec<bool>>, memory: Vec<Vec<Option<Seen>>> }` (T2-050 layout, filled by T2-060 at Stage 8, §8.4: per side and regiment index; the masks hashed after the morale shocks and snapshotted, the memory snapshotted only), `MoraleShocks(Vec<Shock { regiment, kind: ShockKind }>)` (T2-040: the one-time morale shocks queued for Stage 14, `Disengage | ChargedFront | ChargedFlank | GeneralDeath | Rout`; amounts come from `Rules` when applied; hashed and snapshotted because death queues at Stage 15 for the next tick), `MapRes(Arc<LoadedMap>)` (heightmap, zone raster, river flags, deployment polygons; built by `new`/`restore` from `map_id`, `BattleWorld::empty` holds a flat placeholder `engine:flat`), `SpatialGridRes(SpatialGrid<SoldierId>)`, `AnchorGridRes(SpatialGrid<RegimentId>)`, `NavGridRes(NavGrid)`, `PathfinderRes(AStar)`, `PathRequests(BTreeSet<RegimentId>)`, `MeleeGateRes { side, may_fight, near_enemy, extent, in_aura, status: Vec<StatMults> }` (T2-020, per regiment in `Ids` order, rebuilt every Stage 9; `in_aura` since T2-043: inside the side's living, non-routing general's aura, read by Stage 10 and Stage 14; `status` since T2-050: each regiment's cached status multipliers for the parallel Stage 10 systems), `Outcomes(Mutex<Vec<AttackOutcome>>)` and `Kills(Vec<Kill { victim, killer: Option<SoldierId>, killer_regiment: Option<RegimentId> }>)` (T2-021, transient within a tick: filled at Stage 10, the kills drained at Stage 15; the killer's regiment is resolved when the kill is recorded so a shooter that fell while its projectile flew still gets the credit, T2-030), `RangedGateRes { may_fire, target, volley_ready, blockers }` (T2-030, per regiment in `Ids` order, written by `ranged_target` every Stage 9), `Shots(Mutex<Vec<Shot>>)` (T2-030, transient: the tick's shots in thread order until `ranged_spawn` sorts them), `Projectiles(Vec<Projectile>)` (T2-030, state: hashed and snapshotted, §4.3), `PendingDamage(Vec<Pending { apply_tick, target, damage, shooter, shooter_regiment }>)` (T2-030, state: hashed and snapshotted in queue order; filled and applied from T2-031), `CommandInbox(Vec<Command>)`, `Rejected(Vec<(Command, RejectReason)>)`, `StepEvents(Vec<BattleEvent>)`, `LastHash(StateHash)`, `Rng { seed: u64, streams: [RngStream; StreamId::COUNT = 9] }`, `Ids { soldiers, regiments, projectiles: IdAllocator, soldier_entities: Vec<(SoldierId, Entity)>, regiment_entities: Vec<(RegimentId, Entity)> }` (the canonical ascending order every exclusive system iterates), `Regs(Arc<Registries>)` (rules are read as `Regs.0.rules`, so a hot-reload swap carries them; there is no separate `Rules` resource), `SetupRes(Option<BattleSetup>)`, `ThreadCount`, `FlowFields { fields: Vec<FlowField> }` (T2-042: one escape field per side, index = side, derived by `flow::rebuild_flow_fields` from the nav grid and `SideState.escape_edge`; never hashed or snapshotted; `for_side(side)`). T2-080 adds `AiState { outbox: Vec<Command>, plans: Vec<Option<ArmyPlan>> }` (`il_sim_battle::ai`: the commands Stage 1 queued for the next tick and one plan per side the engine decides for; hashed after the visibility masks, snapshotted) and `AiEnabled(bool)` (not state). T3-020 made `PathfinderRes` a `Hpa` (the `HpaGraph` inside it is derived, rebuilt with the nav grid). Later phases add `Weather` (`Timer` became `BattleFlow`).

### 4.5 Schedule

One `Schedule` per stage, run in `Stage::ALL` order by `step` (T1-060; stages were already totally ordered, so nothing is lost and each stage can be timed through `StageObserver`), with one `SystemSet` per stage inside it. As built the systems of a stage are `.chain()`ed (explicit total order); parallelism lives *inside* systems (`par_iter` over soldiers, `ComputeTaskPool::scope` over grid rows), never between them. Every schedule is built with the `SingleThreadedExecutor`; `set_threads(n > 1)` swaps all 18 to the multi-threaded executor on the process-global pool. Systems that must be exclusive for determinism take `&mut World` (the apply steps). `Stage` exposes `COUNT = 18`, `ALL`, `index()`, `name()`; `build_schedules() -> Vec<Schedule>`; `StageObserver { begin(Stage), end(Stage) }` with `NoopObserver` for plain `step`. Every stage holds real systems since T2-080 (Stage 1 `ai_decide`); SAD §12 T-9 is closed.

Stage 0 `apply_commands`: sort incoming by `(player, seq)`, validate per SIM-CMD-003/004, mutate `Order`, `FormationState`, `Fire` (`FireMode`, T2-030: `NotRanged` for a regiment without `ranged`, `InvalidTarget` for an own-side or empty `Target`; sets the mode and clears the target), `Sides`; push `CommandRejected` events. As built (T1-047): `Move` / `AttackMove` (a move until Phase 2) write a fresh `Order` (target clamped to the map), clear the path, queue a `PathRequests` entry and request a reform; `Halt` ends the order and drops the path and wheel target; `SetFormation` rejects `UnknownContent` and `FormationNotAllowed` (template not in the unit's `formations`), then sets the template with `morph_until = tick + morph_ticks`, `ranks` (the template default when `None`), cancels any corridor morph and requests a reform; `SetFacing` goes through `formation::set_facing`; `SetSpeedMode` sets `Order.speed`; `GroupFormation` runs `arrange_group` and issues one move per placement with its ranks; a phase gate (SIM-FLOW-010, T2-070) answers `WrongPhase` before anything else; `Deploy` checks the side's polygon (`OutsideDeploymentZone`) and teleports the anchor and its soldiers onto their slots; `ConfirmDeployment` and `Surrender` mark every side the player owns; `Withdraw` sets the order at `March`, the soldiers `Withdrawing`, and is a no-op on a routing regiment (SIM-CMD-004); `UseAbility` (T2-050) runs `abilities::use_ability` per SIM-ABIL-003 in Battle and Pursuit only.
Stage 1 `ai_decide` (T2-080, §8.5): for every side owned by `PlayerId(255)`, in side order, the Deployment placement (T2-081), the army decision when due (T2-082) and the due regiments (T2-081); every decision is stamped `tick + 1`, `PlayerId(255)`, `seq` in emission order and pushed to `AiState.outbox`, which `step` moves into the inbox before the next Stage 0. Stage 2 `formation_layout`, `formation_apply`, `formation_integrity` (§7); Stage 3 `pursue_update` (T2-020: target check, `AttackMove` acquisition, re-path request, charge `run` switch; exclusive, ascending id), `serve_path_requests` (an attack order's destination is its target regiment's anchor), `regiment_follow_path` (holds an engaged attacker's anchor); Stage 4 `soldier_steer` (Fighting branch seeks the target; Routing and Withdrawing soldiers follow the escape field, T2-042); Stage 5 `integrate`; Stage 6 `rebuild_spatial_grids`; Stage 7 `collision_resolve` (§5, §6); Stage 8 `visibility_update` (T2-060, §8.4); Stage 9 `melee_gate`, `ranged_target` (T2-030, §8.2), `melee_target`, `melee_recount` (§8.1); Stage 10 `melee_attack`, `apply_outcomes` (§8.1), `ranged_fire`, `ranged_spawn` (T2-030, §8.2); Stage 12 `ability_tick` (T2-050, §8.3).
Stage 16 `battle_flow` (T2-070, `flow_battle.rs`): SIM-FLOW-011..017 as built: engine-AI confirmations and the deployment end, reinforcements (`spawn_regiment` with a placement, `Visibility::resize`), defeat detection (`SideState.defeated`), the Pursuit and Ended transitions with `PhaseChanged` and `Ended { result }` (`result::compute`). `Stage::runs_in(phase)` decides per stage whether `step` runs it (Deployment: 0, 6, 8, 16, 17; Ended: 0 and 17), so no system carries a phase guard; `BattleWorld::new` starts in Battle when every side is pre-placed and in Deployment otherwise, auto-placing the unplaced regiments (`flow_battle::auto_placements`, a battle line at the zone centre).
Stage 17 `flush_events_and_hash`: copy `Pos→PrevPos`, `Facing→PrevFacing`; hash per SIM-DET-004 in ascending id (iterate a sorted `Vec<Entity>` maintained by the `Ids` resource); drain events.

### 4.6 Snapshot

`Snapshot { version: u32, tick, phase, setup: BattleSetup, ids, rng, sides, regiments: Vec<RegimentSnap>, soldiers: Vec<SoldierSnap>, projectiles: Vec<Projectile>, pending_damage: Vec<Pending>, morale_shocks: Vec<Shock>, flow: BattleFlow, visibility: Vec<Vec<bool>>, memory: Vec<Vec<Option<Seen>>>, ai: AiSnap { outbox: Vec<Command>, plans: Vec<Option<ArmyPlan>> } }` encoded with postcard (`SNAPSHOT_VERSION = 10` since T3-010, when the army plan gained `standoff_since` and `run_in`; version 9 since T2-080, when the AI state joined; version 8 since T2-071, when `BattleFlow` gained `ended_at`; version 7 since T2-050: `RegimentSnap` gained `withdrawn`, `energy`, `cooldowns: Vec<u16>` and `statuses: Vec<StatusSnap { ability: ContentId, remaining, stacks, hostile }>`, `SideState` its `surrendered` and `reinforcements_spawned`, `timer` became `flow: BattleFlow`, and `visibility` and `memory` are stored per side; version 6 since T2-040: `RegimentSnap` gained `fled`, `rout_count`, `engaged_since`, `arc_hit` and `fatigue_mean`, `SoldierSnap` an optional `general` rank, the morale shock queue is stored, and `SideState` carries its escape edge and general fields; version 5 since T2-030: `RegimentSnap.ammo` gave way to `fire: Option<FireSnap { mode, target, cooldown }>`, `SoldierSnap` gained `ranged: Option<(ammo, cooldown)>` (presence stored, never rederived from the unit), and the projectile list and pending damage queue are stored verbatim (restore checks the projectile ids ascend); version 4 since T2-022, when `RegimentSnap` gained `files`: the width is hashed state that the live world refreshes only at Stage 2, so a snapshot taken in a tick with deaths must carry the old value rather than rederive it; version 3 (T2-020) added the order's target regiment, the `Combat` fields, the casualty ring as a `Vec<u16>` and `initial` to `RegimentSnap` and the melee target and cooldown to `SoldierSnap`; version 2 added the required `map_id` in T1-030; older snapshots are not migrated). Phase 0 layout: `RegimentSnap { id, side, setup_id, unit_type: ContentId, anchor_pos, anchor_facing, morale, morale_state, order, ammo }`, `SoldierSnap { id, regiment, p, v, facing, hp, fatigue, slot, fsm_state, fsm_since }`, `IdsSnap { soldiers_next, regiments_next, projectiles_next }`; `PrevPos`/`PrevFacing` and `Body` are rebuilt on restore. As built (T1-030..T1-048) `RegimentSnap` also carries the order (target, facing, speed, since), the stored path (waypoints with corridors, next, requested) and the formation state (`formation` and `prior_formation` as ContentIds, `ranks`, `integrity`, `morph_until`, `needs_reform`, `laid_out_facing`); `restore` installs the map from `setup.map_id`, then `rebuild_derived` rebuilds the spatial and anchor grids (from positions), the `NavGrid` (from the map) and the `HpaGraph` inside `PathfinderRes` (from the nav grid and the movement rules, T3-020; gate states arrive with Phase 5), the `FlowFields` (from the nav grid and each side's escape edge, T2-042), the `PathRequests` queue (from `Path.requested`), the formation slot tables (from template, count and ranks), `Rank` (from `SlotRef`), the attacker counts (from the melee targets, T2-020) and the cached status multipliers (from the status lists, T2-050). Paths are stored, not re-requested (SIM-DET-005). Snapshot of 32k soldiers ≈ 32k × 40 B ≈ 1.3 MB.

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
// As built (T3-020, `il_sim_battle::hpa`): the abstract graph is a pure function of the nav grid and the two rules, derived data (never hashed or snapshotted), rebuilt by `rebuild_derived` (so by `new` and `restore`).
pub struct GateNode { pub cell: u32 /* nav index */, pub cluster: u16, pub pair: u32 /* the node across the border */ }
pub struct DirtyRect { pub x0, y0, x1, y1: u32 }   // inclusive cells
pub struct HpaGraph { cluster, gate_split, clusters_x, clusters_y, cols, rows: u32, cluster_of: Vec<u16> /* per cell */,
    border_gates: Vec<Vec<(u32, u32)>> /* per border (vertical borders row by row, then horizontal): gate cell pairs, the first in the west or north cluster */,
    cluster_edges: Vec<Vec<(u32, u32, u32)>> /* per cluster: (cell_u, cell_v, cost) for every ordered pair of distinct gate cells a cluster-bounded path joins */,
    nodes: Vec<GateNode> /* sorted by (cell, pair cell); the index is the node id */, node_offsets: Vec<u32>, adj: Vec<(u32 /* node */, u32 /* cost */)> /* CSR adjacency, ascending target: the intra edges from the node's cell, a zero-cost edge to another node on the same cell (a corner cell on two borders), the inter edge to `pair` */,
    cluster_offsets: Vec<u32>, cluster_nodes: Vec<u32> /* CSR of node ids per cluster */, node_ids: Vec<u32> }
impl HpaGraph {
    pub fn build(nav: &NavGrid, cluster: u32, gate_split: u32) -> Self;   // `build_with(.., parallel: bool)` forces the per-cluster work serial for the thread-count test
    pub fn repair(&mut self, nav: &NavGrid, dirty: DirtyRect);   // re-gates every border of the clusters touching `dirty` (one cell around), recomputes the intra edges of those clusters and of the clusters across the re-gated borders, reassembles; equals a fresh `build` (tested); Phase 5 gates and walls call it (SIM-MOVE-006)
    pub fn matches(&self, nav, cluster, gate_split) -> bool; pub fn is_built; pub fn cluster / clusters_x / clusters_y / cluster_count / node_count; pub fn nodes() -> &[GateNode]; pub fn cluster_of(cx, cy) / cluster_of_index(i) -> u16; pub fn cluster_rect(c) -> (x0, y0, x1, y1); pub fn edges(node) -> &[(u32, u32)]; pub fn cluster_node_ids(c) -> &[u32]; pub fn nodes_at(cell) -> &[u32]; pub fn gate_cells(c) -> Vec<u32>;
}
pub struct ClusterSearch { dist: Vec<u32>, heap }   // `run(nav, rect, start)`: a Dijkstra bounded to one cluster rectangle over `NavGrid::for_each_neighbour` (the corner-cut rule only reads cells inside an axis-aligned rectangle, so it equals the plain Dijkstra on the cropped cluster: the test oracle); `dist_at(nav, rect, cell)`
// Gates (SIM-MOVE-003): along each border the maximal runs of cells passable on *both* sides; one gate pair at the run's centre (`start + (len − 1) / 2`) and, when the run is longer than `hpa_gate_split`, one at each end. Intra edges: one `ClusterSearch` per gate cell of the cluster, in parallel over clusters through `ComputeTaskPool::scope` when a pool exists, each cluster into its own slot (so 1 and 8 threads build the same graph). Inter edge cost: `step_cost(cost(pair cell), false)`. A cluster whose cells are all passable at one cost skips the searches: its intra costs are the closed form `(max(dx, dy) − min(dx, dy)) × cardinal + min(dx, dy) × diagonal` (exact on a uniform 8-connected grid); a search stops once every other gate cell of the cluster is settled. `rome:test_field` (200 × 150 cells, 13 × 10 clusters at 16) has 1,382 gate nodes; `tests/maps/tiny` at cluster 1 has 2. Build time on the target machine (`benches/benches/nav.rs::hpa_build_*`, 2026-09-10): the test map 3.7 ms; a 400 × 300-cell (1600 × 1200 m) open grid 4.2 ms; a 400 × 300 random grid with 25 % rock (7,986 nodes, 98k intra edges: the adversarial case, no real map looks like it) 20 ms on the 8-thread pool and 105 ms serial, the bounded searches being the whole of it.
pub trait Pathfinder { fn find(&mut self, nav: &NavGrid, from: V2, to: V2, out: &mut Vec<V2>) -> PathResult; }
pub struct AStar { open: BinaryHeap<Reverse<(u32 /* f */, u32 /* node */)>>, g: Vec<u32>, came: Vec<u32>, g_epoch: Vec<u32>, closed_epoch: Vec<u32>, epoch: u32 }   // `new()`, `search_cells(nav, start, goal, out: &mut Vec<(u32, u32)>) -> Option<u32>` (cell path and cost); `dijkstra_cost(nav, start, goal)` is the test oracle
pub struct Hpa { abstract_astar: AStar, refine: AStar, graph: HpaGraph }   // Phase 3; as built (T3-020/021): `Hpa { graph: HpaGraph, astar: AStar /* the fallback for a world whose graph is not built for this grid */, refine: AStar, abstract_search: AbstractSearch, search: ClusterSearch, start_edges, goal_edges: Vec<(u32, u32)>, abstract_path: Vec<u32>, cells, segment: Vec<(u32, u32)> }` with `new`, `graph()`, `rebuild(nav, &MovementRules)`, `ensure(nav, rules) -> bool` (rebuilds only when `graph.matches` fails: a swapped nav grid in a test, a world built by `empty`), `repair(nav, dirty)`, `last_cells()` (the refined cell path of the last `find`, the tests' cost measure); `PathfinderRes(pub Hpa)` since T3-020, and `serve_path_requests` calls `ensure` before `find`.
// `Hpa::find` (T3-021, SIM-MOVE-002): (1) snap the endpoints exactly as `AStar::find` does (`nav::snap_endpoints`, shared); (2) start and goal in one cluster: `refine.search_cells_within` bounded to it, done if found; (3) link the start to its cluster's gate nodes with a forward `ClusterSearch` and the gate nodes of the goal's cluster to the goal with a reverse-cost one (`run_reverse`: each relaxation pays the *source* cell's step cost, so the distance read at a gate cell is the forward cost gate → goal), giving the temporary edges S → gates and gates → G; (4) A\* over the gate nodes plus S and G (`AbstractSearch`: integer `g`, the octile heuristic between node cells, `Reverse((f, node))` so ties break by node index, epoch-stamped like `AStar`); `NoPath` when it empties or an endpoint links to no gate; (5) refine across cluster pairs: the anchors are the start, the far cell of every inter edge and the goal, and each segment is an A\* bounded to the two clusters its gate joins (`search_cells_within` with a cluster predicate), so the refined path may cross a border anywhere along it and the abstract intra edges only choose the clusters; (6) `nav::emit_path` (shared): `from`, the inner cell centres, `end`, then `string_pull`. Measured on 25 %-rock 64 × 48 random grids (`hpa_paths_are_within_ten_percent_of_astar_on_random_grids`, 969 solvable requests) at the flagship rules 16 / 6: mean cost 1.026 × A\*, 97 % of paths within 10 %, worst 1.41; at 16 / 1 mean 1.009, worst 1.30; refining every segment inside one cluster instead gave mean 1.040 and 59 paths over, and a whole-path refinement inside the corridor of the abstract path's clusters gave mean 1.003 and worst 1.12 at 283 µs per corner-to-corner find against 125 µs, and was not adopted (the owner's decision, 2026-09-10: the 10 % bound is the corpus mean and SIM-MOVE-003 stays as written). Same request, same path at 1 and 8 threads (Stage 3 test); `move_reform_2000` still morphs at the bridge (T1-042's test).
pub fn string_pull(nav: &NavGrid, path: &mut Vec<V2>);
pub struct PathRequests(pub BTreeSet<RegimentId>);   // a resource (§4.4), served ascending, `movement.paths_per_tick` per tick (SIM-MOVE-005)
```

- A\* uses integer costs (octile × 100) so the heap order is deterministic regardless of `Scalar`. Ties in the heap are broken by node index.
- `Pathfinder` is a resource swapped by phase: `AStar` in Phase 1, `Hpa` from Phase 3 (REQ-PATH-002).
- As built (T1-032): `NavGrid::from_map(&LoadedMap, &Registries, &MovementRules)` marks a nav cell impassable when any zone cell whose centre lies in it is `passable: false` or a river cell without a `crossing` zone, and costs it the largest `move_cost × 100` of those zone cells (slope is not in the cost; `from_costs` builds test grids). Diagonal steps cost `ceil(cost × 141 / 100)` and never cut an impassable corner. `Pathfinder::find(nav, from, to, out) -> PathResult::{Found, NoPath, StartBlocked, GoalBlocked}`: blocked endpoints snap to the nearest passable cell within `SNAP_RADIUS` = 8 rings (ties by smaller `(cy, cx)`), `out[0] == from`, the last point is `to` (or the snapped cell centre); `string_pull` is greedy farthest-visible over `segment_clear`, a supercover DDA that also tests both side cells at an exact corner crossing. `corridor_width_at(p)` = `min(passable_run_x, passable_run_y) × cell`; each `Waypoint { p, corridor }` stores it so the corridor morph (SIM-MOVE-004) compares against the regiment's current width at follow time instead of a baked flag. `serve_path_requests` (Stage 3, exclusive) pops up to `paths_per_tick` ids from `PathRequests` (a `BTreeSet<RegimentId>`, rebuilt on restore from `Path.requested`), writes `Path { waypoints, next: 1, requested: false }`, and on failure resets the order to Idle with a `PathNotFound` event. `dijkstra_cost` is the optimality oracle.

### 6.2 Systems

| System | Stage | Parallel | Rule IDs |
|---|---|---|---|
| `serve_path_requests` | 3 | no | SIM-MOVE-002/005; since T3-021 the `PathfinderRes` it calls is `Hpa` (`ensure` then `find`); `paths_per_tick` and `PathRequests` unchanged |
| `regiment_follow_path` (anchor move, wheel, cohesion, corridor column morph) | 3, after `serve_path_requests` | per regiment (independent; parallel when a task pool exists) | SIM-MOVE-010..013, SIM-MOVE-004; as built (T1-042): waypoints within `waypoint_radius` are skipped in one tick, the anchor wheels toward the waypoint by `wheel_rate × dt` then advances `min(v_reg × dt, distance)` clamped to the map; `v_reg = mode_speed(unit, order.speed) × template.speed_mult × zone.move_mult(anchor) × slope_mult(anchor, dir)`, `× morph_speed_mult` while `tick < morph_until`, `× straggler_slowdown` while the straggler fraction (soldiers farther than `straggler_radius × sf` from their slot) exceeds `straggler_fraction`; on arrival the order becomes Idle, the ordered facing is taken, a prior template restored and a reform requested |
| `soldier_steer` (seek/flow, separation via grid, avoidance) → writes `Vel`, `Facing`, `Fsm` | 4 | par_iter over soldiers (reads previous tick grid) | SIM-MOVE-020..025, SIM-FLOW-002; as built (T1-043): the slot comes from the regiment's `Anchor` + `FormationState` through `Ids`, `v_max = mode_speed(unit, order.speed) × zone × slope`, neighbours are the `sep_max_neighbours` nearest grid entries (ties by id) within `2r_i + 2r_j + sep_margin` with `r_j` read from the neighbour's `Body`, avoidance tries ±15°, ±30°, ±45°, ±60°, ±90° against `NavGrid::segment_clear` over `lookahead_ticks` and stops when none is clear; facing tracks the slot facing within `slot_arrive_radius`, else the velocity. As built since T3-022 (same additions in the same order, so the same `Vel` bit for bit): `r_j` comes from the `SoldierBodies` table aligned with the grid entries instead of an ECS lookup per neighbour, the candidates within the query circle are keyed `(d², entry index)` and the `sep_max_neighbours` nearest selected with `select_nth_unstable_by` before only those are sorted (the index ascends with the id, which is the order the stable sort by distance gave), and the regiment components are read from a per-tick table indexed by `Ids.regiment_index` |
| `integrate` | 5 | par_iter | `p += v × dt`; SIM-MOVE-042 clamp; as built (T1-043) `push_out` tries the full move, then x only, then y only, else stays (Phase 1 plan S12) |
| `collision_resolve` | 7 | pair buffers per cell row → id-order apply, ×`collision_iterations` | SIM-MOVE-040..043; as built (T1-044): the pair lists of this tick's grid are enumerated once per row (rows in parallel through `ComputeTaskPool::scope` when a pool exists), sorted `(i, j)`, then each pass folds them in row order into per-soldier pushes from the current positions and applies the pushes in ascending id through `push_out`; positions are written back once; coincident centres separate along +x. As built since T3-022, every position bit for bit the T1-044 position (`S` is an `f32`, so a push is the same sum only if the same additions happen in the same order; each change below removes only work that added nothing): the buffers persist in the `CollisionScratch` resource; radii and masses come from `SoldierBodies`, filled beside the grid at Stage 6 and kept aligned by Stage 7's rebuild (same ids, same order) and by the death pass (filtered together); the fold skips a pair whose `d² > reach² × (1 + 2⁻¹⁶)` before the square root (`pair_push` returns `None` for every such pair, the slack covering both roundings); a pass that changed no position ends the loop; and the pair set is *narrowed* at enumeration to the pairs closer than `2 r_max + M` (`M` = 1 m, a code constant, not a rule) with a guard: the sum of each pass's largest displacement bounds every soldier's travel, and while twice that bound stays under `M × (1 − 2⁻¹⁰)` no dropped pair can overlap; the guard is local (the largest travel sum per grid cell), and a row whose cells and half-neighbourhood could hold a dropped pair past the bound is re-enumerated in full for the next pass, as T1-044 enumerated every row, the other rows staying narrowed (`CollisionScratch` counts: in the 10k fight over 6,000 ticks 17,005 of 900,000 row lists were widened, 1.9 %, spread over 4,498 ticks; in the idle 20k bench none). The parallel two-phase fold and the per-row steering gather the task list named were not needed and were not built |
| `rebuild_spatial_grids` | 6 | no | §5: soldiers into `SpatialGridRes`, anchors into `AnchorGridRes`, from end-of-tick positions |
| `flow::rebuild_flow_fields` | `rebuild_derived` (new, restore) and any nav-grid change (T2-042) | no | SIM-FLOW-001/003 |

Helpers exported from `movement` for tests and tools: `push_out` (integrate), `Disc`, `accumulate_pushes`, `pair_push` (collision), `seek_velocity` (steer), `mode_speed`, `zone_move_mult`, `slope_mult`, `formation_width`, `tick_dt`, `deg_to_rad` (regiment).

Note on Stage 4 reading the grid: steering at tick *t* uses the grid built at Stage 6 of tick *t−1* (end-of-tick positions of the previous tick). Collision at Stage 7 uses the grid rebuilt at Stage 6 of the same tick and, when it moved anyone, rebuilds it from the pushed positions (T1-044): the grid then always indexes end-of-tick positions, which is exactly what `rebuild_derived` reconstructs after a restore (SIM-DET-005); a grid of pre-collision positions could not be recovered from a snapshot.

Terrain sampling (as built, T1-030): `LoadedMap::from_def(&MapDef, zone_cell) -> Result<LoadedMap, MapError>`; `height_at(p) -> S` bilinear on a `Vec<S>` of `height_cols × height_rows` samples at the map's `height_cell` (positions outside the map read the nearest edge); `zone_at(p) -> Option<Handle<ZoneType>>` from a rasterised `Vec<u8>` at `movement.zone_cell` (2 m) indexing the map's `zone_handles` table (`[0]` = `base_zone`; `None` only on the flat placeholder map); `river_at(p) -> bool` from a capsule raster of the rivers; `in_bounds`, `clamp`, `zone_cell_of`, `deployment_polygon(side)`. Zone polygons are rasterised by scanline at cell centres (even-odd, half-open on the right), later polygons overriding earlier ones. The heightmap sidecar is read by the `il_data` pipeline (`HeightmapRef::samples`, from the `assets_root` of the mod that last wrote `heightmap.path`; a missing or mis-sized file is a load diagnostic), so the sim never touches the filesystem.

Budget: steering 8 ms at 20k (400 ns per soldier with 8 neighbours); collision 8 ms. Measured after T3-022 on the target machine (`docs/evidence/phase3/machine.md`, 8 threads, means over three sittings of `bench --soldiers 20000`): SoldierSteering 4.7 to 5.8 ms (6.7 before), Collision 3.0 to 3.5 ms (11.5 before); the 10k fight's row is in `benches/baseline.json` and the 20k fight's in `docs/evidence/phase3/bench_perf_20k.md` (T3-024).

Tests: A\* optimality vs Dijkstra on random grids; HPA\* path cost within 10 % of A\*; string pulling never crosses impassable cells; collision conserves momentum-weighted centre for equal masses; steering unit tests for arrive damping; T3-022: the pre-checked fold and the narrowed pair set (with and without a forced widening, one to four passes) equal the plain T1-044 fold bit for bit on random mixed-radius crowds, and the four `il_cli run` hash logs (`idle_1000`, `move_reform_2000`, `perf_10k`, `ai_skirmish_300`) were byte-identical before and after.

## 7. Formations (`il_sim_battle::formation`)

```rust
pub struct FormationTemplate { pub id: ContentId, pub name_key: String, pub layout: Layout, pub default_ranks: u8, pub min_ranks: u8, pub max_ranks: u8,
    pub spacing_file: S, pub spacing_rank: S, pub role_zones: Vec<RoleZone>, pub morph_ticks: u16,
    pub integrity_bonus_attack: S, pub integrity_bonus_defence: S, pub speed_mult: S, pub custom_slots: Vec<V2>, pub min_files: u8, pub loose_mult: S, pub default_files_column: u8 }
pub enum Layout { Line, Column, Square, Wedge, Phalanx, Loose, Custom }
pub struct Slot { pub offset: V2, pub facing_offset: Angle<S>, pub rank: u8, pub file: u16 /* u16 since T1-040: a 2,000-man single rank */, pub category: Option<UnitCategory> }
pub trait LayoutFn { fn layout(&self, t: &FormationTemplate, n: u16, ranks: u8, radius: S, out: &mut Vec<Slot>); }
pub fn layout_for(layout: Layout) -> &'static dyn LayoutFn;   // SIM-FORM-003..009
// T3-002 (SIM-FORM-013), built in T3-040: `layout_slots` takes the regiment's category counts so a template with `role_zones` gives each zone's slots that category (`Slot.category`), the unzoned categories following from the front, and a template without zones lays the groups out in list order; `assign_slots` already matches `AssignSoldier.category` to `Slot.category`.
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

// il_sim_battle::combat::formulas (T2-021), all pure:
pub fn hit_probability(a: S, d: S, r: &CombatRules) -> S;                    // SIM-CMBT-011
pub fn melee_damage(dmg: S, armour: S, pen: S, mults: S, r: &CombatRules) -> S; // SIM-CMBT-013
pub fn attack_arc(defender_facing: Angle<S>, to_attacker: V2, frontal_arc_deg: S) -> Arc;  // SIM-CMBT-014; Arc { Front, Flank, Rear }, FLANK_HALF_ARC_DEG = 150
pub fn arc_mults(arc: Arc, r: &CombatRules) -> (S, S);                      // (damage, defence)
pub fn fatigue_mults(f: S, r: &FatigueRules) -> FatigueMults;               // SIM-FAT-004
pub fn morale_mults(state: MoraleState, r: &MoraleRules) -> &StateMults;    // SIM-MOR-004
pub fn experience_mult(experience: u8, r: &CombatRules) -> S;               // SIM-CMBT-017
pub fn charge_mults(charge_bonus: S, charging: bool, negated: bool, r: &CombatRules) -> (S, S);  // SIM-CMBT-015
pub fn braced(anti_cavalry_bonus: S, integrity: S, order: OrderKind, engaged: bool, arc: Arc, r: &CombatRules) -> bool;
pub fn terrain_defence_mult(zone_defence_mult: S, ford: bool, h_j: S, h_i: S, m: &MovementRules, r: &CombatRules) -> S;  // SIM-CMBT-016
pub fn cooldown_ticks(base: u16, fatigue_interval: S, morale_interval: S, status: S) -> u16;  // SIM-CMBT-010
pub struct StatMults { pub attack: S, pub defence: S, pub armour_mult: S, pub armour_add: S, pub damage: S, pub speed: S, pub attack_interval: S, pub morale_per_s: S, pub fatigue_rate: S, pub los_radius: S, pub accuracy: S }  // SIM-ABIL-005 (T2-050); `armour(unit_armour) -> S`
pub fn status_mults(statuses: &[StatusEffect], regs: &Registries) -> StatMults;  // SIM-ABIL-005: mults multiply, adds sum × stacks, hostile statuses contribute debuffs only
pub fn aura_attack_mult(in_aura: bool, r: &GeneralRules) -> S;                  // SIM-GEN-002 (T2-043)
pub struct AttackOutcome { attacker: SoldierId, target: SoldierId, hit: bool, damage: S, arc: Arc }
```

Systems as built (T2-020): `pursue_update` (Stage 3, exclusive; `combat::pursue`), `melee_gate` (Stage 9, exclusive; one pass over soldiers for each regiment's extent, then anchor-grid queries, into `MeleeGateRes`; SAD T-10), `melee_target` (Stage 9, staggered, par_iter over soldiers reading this tick's grid and the previous tick's `Attackers`, writes only its own `Fsm` and `MeleeState`; second-rank targeting through the slot ahead), `melee_recount` (Stage 9, exclusive: `Attackers` and `Combat.engaged`/`last_fighting` in ascending id, `Engaged` events; as built since T3-023, SAD T-10: after `melee_target` only soldiers of regiments the gate let fight, `may_fight && near_enemy`, can hold a target or be `Fighting`, so the recount walks those regiments' lists only, zeroes only the soldiers counted last tick, kept in the `AttackersScratch` resource, and gives a gated-out regiment `engaged == false` without a walk; the restore path, where the gate has not run, keeps the full walk; the counts and flags are the ones the full walk gave). `melee_attack` (Stage 10, par_iter over fighting soldiers: cooldown count-down, reach check, arc, bracing, the SIM-CMBT-011 product, one `hash_draw` per attack, damage per SIM-CMBT-013, outcome into the shared `Outcomes` buffer, cooldown per SIM-CMBT-010), `apply_outcomes` (Stage 10, exclusive: sort by attacker id, subtract hp, queue zero crossings into `Kills`). The charge window and charge mass are opened and closed by `melee_recount`. Planned: `melee_attack` (Stage 10, par_iter producing `AttackOutcome` into a per-thread buffer, then merged and sorted by attacker id, then applied to `Health`), `resolve_deaths` (Stage 15, exclusive, as built in T2-022: the tick's ring slot is zeroed on every regiment; the `Kills` queue is sorted by victim id and each victim emits `SoldierDied`, leaves `Regiment.soldiers` and the parallel `FormationState.assignment` with `needs_reform` set, adds to `Morale.deaths_5s[tick % 100]` and to the killer regiment's `Combat.kills`; dangling `MeleeState.target`s are cleared; the entities leave `Ids.soldier_entities`, are despawned, and the spatial grid is rebuilt without them; `detach_from_regiment` and `remove_soldiers` are shared with `resolve_fled`, which follows it at Stage 15 and removes every Routing or Withdrawing soldier standing in an escape-edge cell, counting it in `Combat.fled` and emitting `SoldierFled`, T2-042). Regiments with no soldiers stay (ids, `BattleResult`); the gate excludes them and pursuit retargets away from them.

Budget: targeting 4 ms, combat 4 ms (only engaged soldiers do work; typical 3–6k engaged at P3).

Tests: `hit_probability` monotonic and clamped; arc classification golden cases; charge negation by braced anti-cavalry; scenario bands (Simulation Spec §15.3).

### 8.2 Ranged and projectiles

Systems as built in T2-030 (`combat/ranged.rs`, formulas in `combat/formulas.rs`): `ranged_target` (Stage 9, exclusive, ascending regiment id, after `melee_gate` whose `extent` it reads: target selection per SIM-PROJ-001/002 with `count_in_annulus` over the candidate's soldiers, then fills `RangedGateRes` with `may_fire`, the target, `volley_ready` and the friendly `blockers` within `friendly_block_dist + extents` of a direct-fire regiment); `ranged_fire` (Stage 10, parallel over soldiers with `RangedState`, writes only that component: gate check, cooldown, ammo, FSM state, hashed target pick (draw 1) with the nearest-in-annulus fallback, `flight_ticks`, aim prediction, `scatter` (draw 0), the segment-versus-extent-circle friendly block, then a `Shot` into the shared `Shots` buffer); `ranged_spawn` (Stage 10, exclusive: stable sort by shooter id, per shot in that order a `Projectile` with the next `ProjectileId` while the list is under `projectile_cap` (the statistical branch is T2-032), ammo spent, `VolleyFired` and `FireBlocked` per regiment in id order, volley cooldowns reset to `cooldown_ticks(reload_ticks, max fatigue interval, 1, 1)`, then every `Fire.cooldown > 0` counts down once). Stage 11 as built in T2-031 is one exclusive system, `projectile_stage`: `projectile_land` (the projectiles with `land_tick == tick`, ascending id; `query_circle_indices` at the landing point with `max soldier radius + projectile_radius`; `pick_victim` takes the nearest circle covering the point, ties to the lowest id; the arc from the victim's facing toward the launch point; `ranged_damage`; one `Pending` entry per hit and a `ProjectileLanded` event per landing), `apply_pending_damage` (the queue stable-sorted by `(apply_tick, target)`, the due entries applied to `Health` with kills into `Kills` carrying the shooter's regiment, the rest kept), then `retain(land_tick > tick)`. There is no `projectile_advance`: the position is closed-form (§4.3) and the renderer evaluates it. Over the cap `ranged_spawn` calls `statistical_shot` (T2-032, SIM-PROJ-008): `footprint_area` and `stat_hit_probability` from `formulas.rs`, the hit roll on draw index 2, the nearest soldier of the target regiment to the aim point, `ranged_damage` from the victim's facing, and a `Pending` entry at the would-be `land_tick`.

Aim prediction: `aim = target.p + target.v × flight_ticks × dt`; `flight_ticks(arc, d, speed, gravity)` is `ceil(d / speed)` in ticks for direct and `ceil(d × √2 / min(sqrt(d × g), speed))` for the 45° indirect launch, at least 1; `apex_height` is `direct_apex` or `d / 4`; `range_mult(h_shooter, h_target, height_range, height_ref)` per SIM-PROJ-002; `ranged_damage(damage, armour, pen, arc, shield)` per SIM-PROJ-006. Randomness: the `combat_ranged` stream in the `hash_draw` form, index 0 the scatter angle, 1 the target pick, 2 the statistical hit roll (T2-032).

Budget: 3 ms at 8k live projectiles.

Tests: flight time golden values; landing hit selection equals nearest; statistical path expected casualties within 10 % of simulated over 50 seeds (T-3).

### 8.3 Morale, fatigue, abilities

```rust
pub struct MoraleRules { pub t_unsettled: S, pub t_shaken: S, pub t_broken: S, pub t_routing: S, pub hysteresis: S, pub rally_margin: S, pub rally_safe_radius: S,
    pub max_routs: u8, pub shatter_strength: S, pub general_death_shock: S, pub rout_shock: S, pub rout_shock_radius: S, pub disengage_penalty: S, pub charged_penalty: S,
    pub casualty_rate_ref: S, pub casualty_total_ref: S, pub fatigue_start: S, pub ally_radius: S, pub allies_ref: S, pub routing_ref: S, pub outnumber_ref: S, pub outnumber_radius: S,
    pub engage_fatigue_ticks: u32, pub safe_radius: S, pub exp_bonus: S, pub w: MoraleWeights, pub state_mults: StateMultsTable }
pub struct StateMults { pub attack: S, pub defence: S, pub interval: S, pub speed: S }              // SIM-MOR-004
pub struct StateMultsTable { pub steady: StateMults, pub unsettled: StateMults, pub shaken: StateMults, pub broken: StateMults, pub routing: StateMults }
impl StateMultsTable { pub fn for_state(&self, discriminant: u8) -> &StateMults; }  // MoraleState as u8; Shattered (5) reads the routing row
pub struct MoraleWeights { pub casualty_rate: S, pub casualty_total: S, pub fatigue: S, pub general_aura: S, pub allies_near: S, pub allies_routing: S, pub high_ground: S, pub fear: S, pub flanked: S, pub outnumbered: S, pub integrity: S, pub engaged_duration: S, pub winning: S, pub recovery: S }
// il_sim_battle::morale::factors (T2-041; `MoraleInputs`, not the AI's `RegimentContext`)
pub struct MoraleInputs { count: u16, initial: u16, own_deaths_5s: u32, enemy_deaths_5s: u32, fatigue_mean: S, in_aura: bool, allies_steady: u32, allies_routing: u32,
    height_delta: Option<S>, fear: bool, hit_flank: bool, hit_rear: bool, surrounded: bool, enemies_near: u32, own_near: u32, integrity: S, engaged_ticks: Option<u32>, enemy_within_safe: bool, state: MoraleState }
pub fn morale_factors(i: &MoraleInputs, m: &MoraleRules, c: &CombatRules, f: &FormationRules) -> [S; FACTORS];  // activations per SIM-MOR-010..024, MoraleWeights order; sign in the weight
pub fn morale_delta(x: &[S; FACTORS], w: &MoraleWeights, dt: S) -> S;              // SIM-MOR-002
pub fn shock_amount(kind: ShockKind, state: MoraleState, r: &MoraleRules) -> S;     // SIM-MOR-014/025/026/033
pub fn morale_state(m: S, current: MoraleState, r: &MoraleRules) -> MoraleState;  // SIM-MOR-003 hysteresis; Routing/Shattered returned unchanged
// il_sim_battle::flow (T2-042)
pub struct FlowField { cols, rows, edge: MapEdge, dir: Vec<u8> /* NEIGHBOURS index or NO_DIRECTION = 8 */, dist: Vec<u32>, vec: [V2; 9] }
pub fn escape_edge(map: &LoadedMap, zone: u8) -> MapEdge;          // SIM-FLOW-001: nearest edge to the polygon's mean vertex
impl FlowField { pub fn build(nav: &NavGrid, edge: MapEdge) -> Self; pub fn direction_at(&self, nav, p: V2) -> V2; pub fn is_exit(&self, nav, p: V2) -> bool; pub fn code(&self, cx, cy) -> u8; pub fn cost(&self, cx, cy) -> u32 }
pub fn rebuild_flow_fields(world: &mut World);                     // one field per side
// il_sim_battle::morale::general (T2-043)
pub fn general_fate(world: &World, side: u8, lost: bool) -> GeneralFate;   // SIM-GEN-004; `BattleWorld::general_fate`
pub fn aura_attack_mult(in_aura: bool, r: &GeneralRules) -> S;             // SIM-GEN-002 (combat::formulas); `MeleeGateRes.in_aura` per regiment
// il_sim_battle::nav additions (T2-042): `pub(crate) NEIGHBOURS`, `pub fn step_cost(cost: u16, diagonal: bool) -> u32`,
// `NavGrid::for_each_neighbour(cx, cy, f: FnMut(k, nx, ny, step))` shared by A*, `dijkstra_cost` and the flow field.

pub struct FatigueRules { pub rate_idle: S, pub rate_walk: S, pub rate_march: S, pub rate_run: S, pub rate_fighting: S, pub rate_routing: S, pub armour_rate: S,
    pub thresholds: [S; 3], pub speed_loss: S, pub attack_loss: S, pub defence_loss: S, pub interval_gain: S }
pub fn fatigue_mults(f: S, r: &FatigueRules) -> FatigueMults;   // SIM-FAT-004; FatigueMults { speed, attack, defence, interval }
// il_sim_battle::morale::fatigue (T2-040)
pub enum Activity { Idle, Walk, March, Run, Fighting, Routing }
pub enum FatigueState { Fresh, Active, Tired, Exhausted }
pub fn activity(state: SoldierState, regiment_moving: bool, order_speed: SpeedMode) -> Option<Activity>;  // SIM-FAT-002 as built; None for Dead
pub fn fatigue_rate(activity: Activity, armour: S, fatigue_rate_mult: S, zone_fatigue_mult: S, r: &FatigueRules) -> S;  // per second
pub fn fatigue_state(f: S, r: &FatigueRules) -> FatigueState;   // SIM-FAT-003
pub const FATIGUE_MEAN_PERIOD: u32 = 10;                        // SIM-FAT-005

pub struct GeneralRules { pub aura_radius: S, pub aura_attack: S, pub aura_per_rank: S, pub hp_mult: S, pub wounded_hp: S }   // §9
pub struct VisibilityRules { pub period_ticks: u16, pub conceal_radius: S, pub height_bonus: S, pub eye_height: S, pub los_sample: S, pub memory_ticks: u32 }   // §11
pub enum TimeoutWinner { Defender, MostSoldiers }
pub struct BattleFlowRules { pub time_limit_ticks: u32, pub deploy_timeout_ticks: u32, pub pursuit_ticks: u32, pub fled_return_fraction: S, pub timeout_winner: TimeoutWinner, pub exp_per_kill: S, pub exp_survive: S, pub loot_per_enemy_killed: S }   // §12

// As built (T2-050, `il_data::ability`): every effect kind parses; `Ability::resolve` rejects the non-executable kinds and a zero duration with a diagnostic (SIM-ABIL-002).
pub struct Ability { pub id: ContentId, pub name_key: String, pub description_key: Option<String>, pub icon: Option<String>, pub targeting: Targeting, pub radius: S, pub range: S, pub cooldown_ticks: u16, pub duration_ticks: u16, pub energy_cost: S,
    pub effects: Vec<Effect>, pub stacking: Stacking, pub max_stacks: u8, pub requires_not_engaged: bool, pub requires_not_moving: bool, pub deprecated: Option<String> }
pub enum Targeting { SelfTarget /* "self" */, RegimentAlly, RegimentEnemy, Point, Area }   pub enum Stacking { Refresh, Stack, Highest }
pub enum Stat { Attack, Defence, Armour, Damage, Speed, AttackInterval, MoralePerS, FatigueRate, LosRadius, Accuracy }
pub enum Effect { Buff { stat: Stat, mult: S, add: S }, Debuff { stat: Stat, mult: S, add: S }, Damage { amount: S, armour_penetration: S, per_tick: bool },
    Heal { amount: S, per_tick: bool }, Summon { unit_type: ContentId, count: u16, formation: ContentId }, Fear, Area { effects: Vec<Effect>, radius: S, duration_ticks: u16 }, Teleport { max_distance: S } }   // tagged by `type`
pub struct StatusEffect { pub source: Handle<Ability>, pub remaining: u16, pub stacks: u8, pub hostile: bool }   // il_sim_battle::components
// il_sim_battle::abilities: `use_ability(world, entity, &ContentId, &AbilityTarget, tick) -> Result<(), RejectReason>` (Stage 0), `apply_status(list, source, &Ability, hostile) -> bool` (SIM-ABIL-004), `expire(list) -> Vec<Handle<Ability>>`, `refresh_mults`, `rebuild_status_mults(world)` (restore), `slots(world, entity) -> Vec<Handle<Ability>>` (the unit's abilities, then the living general's for its bodyguard).
// `UnitType` carries `ability_ids: Vec<ContentId>` (hashed) and `abilities: Vec<Handle<Ability>>` (resolved), `energy_max: S`, `energy_regen: S` (SIM-ABIL-006).
```

Systems: `ability_tick` (Stage 12, exclusive, ascending regiment id: slot cooldowns down, energy toward `energy_max`, statuses down with the expired ones removed (`StatusExpired`) and the multipliers refreshed; per-tick effects are Phase 5), `fatigue_tick` (Stage 13, par_iter over soldiers; reads the regiment's `Order`, `Path` and `Combat` through `movement::anchor_moves` for the activity and the zone's `fatigue_mult` at the soldier's position; writes only its own `FatigueC`), `regiment_fatigue_mean` (Stage 13, exclusive, every 10 ticks, ascending id), `morale_tick` (Stage 14, exclusive: gathers every regiment's `MoraleInputs` from the pre-stage state (anchor grid for allies, a scan for the nearest enemy anchor, soldier grid for `outnumbered`), then per regiment in id order applies the queued `MoraleShocks` (queue order), the factor delta and the hysteresis transition, emits `MoraleChanged` and maintains `engaged_since`; rout/rally/shatter per SIM-MOR-030..033 in `morale/rout.rs`, T2-042: `enter_routing` (the shatter test, `rout_count`, the order and FSM changes, the `Rout` shocks, `Shattered`), `try_rally` (`Rallied`, the halt, the facing, the reform) and `follow_centroid` for every Routing or Shattered regiment at the end of the stage). Shock producers: Stage 0 (`Disengage`), Stage 9 `melee_recount` (`ChargedFront`/`ChargedFlank` on the `Charge` tick), Stage 15 (`GeneralDeath`, T2-043) and Stage 14 itself (`Rout`, T2-042); Stage 10 `apply_outcomes` stamps `Morale.arc_hit[arc]` for every melee attack.

Budget: morale 1 ms (200 regiments × grid queries), fatigue 0.5 ms, abilities 0.5 ms.

Tests: factor functions golden; hysteresis state machine table; rally requires safe radius; shatter conditions; stacking rules (`abilities::status` unit tests); `tests/abilities.rs` (testudo against row 5's volley, expiry to the tick, cooldown, ownership, target kind, range, engagement, war cry, the multiplier algebra, 1 vs 8 threads with a mid-status restore).

### 8.4 Visibility

As built (T2-060, `il_sim_battle::visibility`). `Visibility { masks: Vec<Vec<bool>>, memory: Vec<Vec<Option<Seen { pos, facing, count, tick }>>> }` resource, indexed by side then regiment index (`Ids` order); `sees(side, index)`, `resize(sides, regiments)` (T2-070 appends reinforcements). Pure functions: `los_radius(unit_los, zone_los_mult, h_anchor, h_mean, status_los, &VisibilityRules, &CombatRules) -> S` (SIM-VIS-001), `segment_clear(&LoadedMap, from, to, &VisibilityRules) -> bool` (SIM-VIS-002), `sample_indices(len) -> Vec<usize>` and `regiment_visible(&LoadedMap, &VisibilityRules, &Observer { anchor, los_radius }, &Target { anchor, conceal, samples }) -> bool` (SIM-VIS-003), `sees_regiment(&World, side, RegimentId) -> bool` (the check every consumer calls). System `visibility_update` (Stage 8, exclusive): side `s` refreshes when `tick % period_ticks == s % period_ticks`, or every side to a full mask during a revealed deployment (SIM-VIS-006); a refresh gathers every regiment (side, count, anchor, facing, `conceal` from the zone under the anchor, `los_radius`, the sampled soldiers' positions), then per enemy regiment scans the side's observers ascending until one sees it (a linear scan: `n²` distance checks per refresh and one line test per pair that passes them; the anchor grid is not needed at 200 regiments), writes the mask and the memory (`Some(Seen)` on a sighting, dropped `memory_ticks` after the last one). `recompute_all` runs in `BattleWorld::new`; `restore` reads the stored masks. The masks are hashed after the morale shocks and snapshotted; the memory is snapshotted only (SIM-DET-004/005). `BattleView::{visible(side, id), visible_regiments(side), seen(side, id), los_radius(id)}` are the accessors UI, render and AI use; the app shows the first side the local player owns. Measured cost: see the T2-111 budget row (the update runs on one side every ten ticks). Consumers: `command.rs` (`NotVisible` for `AttackRegiment` and `FireMode::Target`), `abilities::use_ability` (enemy targets), `combat::ranged_target` (fire-at-will candidates and the retained target) and `combat::pursue::acquire`. Tests: `visibility.rs` unit tests (the radius, a synthetic ridge, the sample indices, concealment) and `tests/visibility.rs` (the south-east hill of `rome:test_field` found by scanning `height_at`, the forest, the stagger and the memory, `AttackMove`, 1 vs 8 threads with a mid-period restore).

### 8.5 Battle AI (`il_data::ai` + `il_ai` + `il_sim_battle::ai`)

As built (T2-080). The data types live in `il_data::ai` so the loader validates them; `il_ai` (depends on `il_core` and `il_data`) scores and selects; `il_sim_battle::ai` computes the inputs, runs Stage 1 and turns choices into Commands.

```rust
// il_data::ai (content/ai/actions/*.json5, content/ai/profiles/*.json5; Simulation Spec §15.4)
pub enum InputScope { Regiment, Army }
pub enum InputId { Constant, DistanceToNearestEnemy, StrengthRatio, EnemyShare, OwnMorale, OwnFatigue, Engaged, EngagedFrontal, IsFlankExposed, EnemyFlankOpen,
    CavalryApproaching, InfantryThreat, EnemyRangedInRange, EnemyInOwnRange, FriendlyInLineOfFire, Outnumbered, SlotError, Ammo,
    ArmyStrengthRatio, ArmyMoraleMean, ArmyFatigueMean, TerrainAdvantage, TimeRemaining, Aggression, CasualtyFraction, EnemyVisible }   // `scope()`; names checked at load
pub enum Curve { Linear { m, b }, Quadratic { k }, Logistic { k, mid }, Step { threshold } }   // SIM-AI-001; logistic is the algebraic sigmoid
pub struct Consideration { pub input: InputId, pub scale: S /* raw value at x = 1 */, pub curve: Curve }
pub enum Channel { Movement, Formation, Fire, Ability, Stance }
pub enum ActionKind { EngageNearest, HoldPosition, FallBack, FollowCentroid, UseAbility { ability: ContentId }, SwitchFormation { layout: Layout }, FireAtWill, HoldFire, Attack, Defend, Hold, Retreat }   // `channel()`, `scope()`
pub struct ActionDef { pub name: String, pub kind: ActionKind, pub base: S, pub threshold: S, pub noise: S, pub considerations: Vec<Consideration> }
pub struct AiActionSet { pub id: ContentId, pub scope: InputScope, pub actions: Vec<ActionDef> }   // `channel_actions(channel)`; resolve: scopes, unique names, existing abilities
pub struct AiProfile { pub id, pub name_key: Option<String>, pub aggression, general_aggression, reserve_fraction, stance_margin: S, pub army_period_ticks, regiment_period_ticks: u16,
    pub approach_distance, advance_step, line_tolerance, skirmish_range_frac, flank_offset, charge_trigger_dist, charge_max_fatigue, defend_search_radius, counter_charge_dist, screen_offset, screen_gap, reserve_offset, commit_morale: S, pub standoff_max_ticks: u32 /* T3-010 */,
    pub action_set_ids: Vec<ContentId>, pub army_set: Option<Handle<AiActionSet>>, pub regiment_set: Option<Handle<AiActionSet>> /* resolved: exactly one of each */, pub campaign: Option<CampaignProfile> }
// il_ai
pub fn evaluate(curve: &Curve, x: S) -> S;   pub fn normalise(raw: S, scale: S) -> S;   pub fn sat(x: S) -> S;
pub trait InputProvider { fn input(&self, id: InputId) -> S; }   // raw values; implemented by RegimentContext / ArmyContext in il_sim_battle
pub fn score(action: &ActionDef, inputs: &dyn InputProvider, rng: Option<&mut RngStream>) -> S;   // base × Π curves, then × (1 + noise·(2u − 1)) only when noise > 0
pub struct Choice<'a> { pub index: usize, pub action: &'a ActionDef, pub score: S }
pub fn select<'a>(set: &'a AiActionSet, channel: Channel, eligible: &mut dyn FnMut(usize, &ActionDef) -> bool, inputs: &dyn InputProvider, rng: Option<&mut RngStream>) -> Option<Choice<'a>>;   // SIM-AI-001: highest score ≥ threshold, ties by index
pub fn due(tick: Tick, period: u16, key: u32) -> bool;   // SIM-AI-002 stagger
// il_sim_battle::ai
pub enum Stance { Attack, Defend, Hold, Retreat }
pub enum Role { Line { slot }, Skirmish { slot }, Reserve { slot }, Flank { slot, charge: Option<RegimentId> }, Counter { slot }, Screen { slot }, Committed { target: RegimentId }, Bodyguard }
pub struct ArmyPlan { pub stance, pub stance_score: S, pub decided_at: Tick, pub target: V2, pub line_anchor: V2, pub line_facing: Angle<S>, pub line_width: S, pub formed: bool, pub assignments: Vec<Assignment { regiment, role }> /* ascending id */, pub charging: bool, pub standoff_since: Option<Tick>, pub run_in: bool /* T3-010 */ }   // `role_of(id)`
pub fn lateral_room(map: &LoadedMap, right: V2, gap: S) -> S;   pub fn lay_line(regs, infos: &[RegimentInfo], anchor, facing, width, room) -> (Vec<Placement>, bool /* doubled */);   // T3-010: the line's room across the map and the fold onto a double line
pub struct AiState { pub outbox: Vec<Command>, pub plans: Vec<Option<ArmyPlan>> }   // resource; hashed and snapshotted
pub struct AiEnabled(pub bool);                                                       // resource; not state
pub fn side_profile(world: &World, side: u8) -> Option<Handle<AiProfile>>;            // the setup's override, else the faction's
pub fn ai_decide(world: &mut World);                                                  // Stage 1
```

`ai_decide` runs for every side whose `SideState.player == PlayerId::ENGINE_AI` and is not defeated, in side order: in Deployment an unconfirmed side is placed and confirmed (`ai::deploy::commands`, SIM-AI-020; T2-081); in Battle and Pursuit the army decides when `due(tick, army_period_ticks, side)` (T2-082) and each living, non-routing regiment when `due(tick, regiment_period_ticks, id)` (`ai::regiment::decide`, T2-081). As built (T2-081) `ai::inputs::SideSnapshot::build(world, side)` reads the side's regiments and its visible enemies once per tick into `RegRow`s (anchor, facing, count, unit, category, order, engaged, moving, morale, fatigue, range, fire mode, half width, cost weight, layout, morph flag, ability slots with cooldowns, energy, ammo fraction) plus the enemy zones' mean and the bodyguard; `RegimentContext::new(&snap, &me, slot, &regs)` implements `InputProvider` for the SIM-AI-021 inputs; `regiment::decide(regs, snap, me, profile, set, plan, rng, out)` runs the movement, formation, fire and per-slot ability channels through `il_ai::select` with the applicability rules of SIM-AI-021 and pushes `CommandKind`s into `Decisions` only when they change something; the regiment stream (`StreamId::AiRegiment`) is cloned out of `Rng` for the side's decisions and written back. The army (T2-082, `ai::army`): `ArmyContext { snap, profile, map, height_ref, tick, battle_start, time_limit }: InputProvider` for the SIM-AI-010 inputs; `choose_stance(set, &ctx, current: Option<(Stance, S)>, margin, rng) -> (Stance, S)` with the hysteresis margin; `build_plan(regs, map, snap, profile, stance, score, prev, tick, out) -> ArmyPlan` partitions the standing regiments into roles, lays the line with `arrange_group`, places skirmishers, reserves and cavalry, commits reserves and counter-charges, queues a retreat's `Withdraw`s; `highest_ground(map, centre, radius)` is the defend search; `army::decide` runs it when `due` with the `AiArmy` stream and stores the plan in `AiState.plans[side]` before the regiments decide. Decisions become `Command { tick: tick + 1, player: PlayerId(255), seq: emission index, kind }` in `AiState.outbox`; `step` appends the outbox to the inbox before Stage 0 and reports the new outbox as `StepOutput.ai_commands`. Inputs read only state and the derived data `rebuild_derived` recreates (never `MeleeGateRes` / `RangedGateRes`), so a restored battle decides exactly as the uninterrupted one. `BattleView::ai_plan(side)` and `ai_outbox()` expose the state to the overlay and the log. A replay that feeds the logged `ai_commands` with `set_ai_enabled(false)` reproduces the same positions, counts and events; its per-tick hashes differ from the live run's by exactly the outbox, which is hashed state (`tests/ai.rs`).

Budget: 2 ms (staggered: about 10 regiments and at most one army per tick at 10k).

Tests: curve goldens, threshold and tie-break, lazy noise and cadence (`il_ai`); the two kinds' diagnostics (`il_data`); the outbox and plans in the hash and snapshot (`tests/hash.rs`, `tests/snapshot.rs`); the regiment and army behaviours in `crates/il_sim_battle/tests/ai.rs` (T2-081/082); scenario: AI army beats a passive player army (T2-082 bands).

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

Auto-resolve (SIM-CAMP-045): `il_app` constructs a `BattleWorld`, replaces both players by AI, steps until `Ended` or `autoresolve_max_ticks`, and feeds `ApplyBattleResult`. In `il_cli` the same path runs headless: as built (T2-071, T2-082) `il_cli::autoresolve::autoresolve(&AutoresolveOptions { scenario, max_ticks, threads, json, content_root, mods, ai: AiPlayers::{All, None, Players(Vec<PlayerId>)} }, out) -> (BattleResult, ended)` hands the chosen players' sides to the engine with `TransferControl` commands at tick 1 (every player by default, their scripted commands dropped with a note; `--ai none` keeps the scripted run), the default cap being the time limit plus the deployment timeout and the pursuit length, and prints `BattleWorld::result()` as JSON; exit code 2 when the battle did not end.

Budget: end_turn < 5 s with 30 factions (REQ-PERF-009); dominated by auto-resolves, which are bounded.

Tests: economy arithmetic golden; interception creates exactly one battle with reinforcements; `BattleResult` application table (survivors, general fates); campaign determinism over 100 AI turns with hash per turn.

## 10. Renderer (`il_render`)

### 10.1 Design

- **Projection.** World (x, y, h) → screen: isometric with fixed pitch. `screen = P × R(k × 90°) × (x, y)` plus `−h × pitch_scale` on screen y, where `k ∈ 0..4` is the snap rotation (OQ-1 resolved as 4 snaps for MVP; 8 as Could). Sprite facing index = `(facing8 − 2k) mod 8`, so 8 facing sets suffice for all snaps. As built (T1-052): `Camera { center: Vec2 (world), zoom (px/m, 2..96), rotation: u8 (0..=3, quarter turns clockwise), pitch (0.5), elevation (0.8) }`; world → view applies `R(−k·90°)`; `world_to_screen`, `screen_to_world`, `pan_screen`, `zoom_at` (keeps the point under the cursor fixed), `rotate`, `visible_bounds` (culling AABB).
- **Depth.** Painter's order by projected y (back to front), with instance sort per frame on the CPU (32k sort ≈ 1 ms) or by depth in a depth buffer using projected y as z; the latter is chosen (no CPU sort, alpha edges handled by alpha-to-coverage).
- **Instancing.** One draw per atlas (LOD tiers are Phase 3). Instance layout 32 bytes (as built in T1-051; wgpu has no scalar `f16` vertex format): `pos: [f32; 2]` (projected screen pixels), `depth: f32`, `frame_facing: u32` (atlas column in bits 0..16, facing row in bits 16..24), `tint: [u8; 4]`, `scale: f32`, `flags: u32` (bit 0 selected, bit 1 hovered), `_reserved: u32` (`SpriteInstance`, `SpriteInstance::SIZE`, `pack_frame_facing`). 32k instances = 1 MB per frame, written with `queue.write_buffer` into a ring of 3 buffers. The colour target is 4× MSAA with alpha-to-coverage, resolved to the surface. Sprite sheets are `SpriteSet` content files (`content/sprites/*.json5`: atlas path, frame size, facings as rows, columns as frames, ground origin, named animations) over a PNG under `assets/`; `il_cli genart` generates the placeholder sheets.
- **Interpolation.** `p = lerp(prev, cur, alpha)`; facing snaps when the angle crosses a facing8 boundary (no angular lerp for sprites).
- **LOD** (REQ-RNDR-004; specified in T3-003, built in T3-031). `pub enum DetailTier { Detailed, Reduced, Aggregation }` and `pub fn detail_tier(zoom: f32, z1: f32, z2: f32) -> DetailTier` in il_render: `zoom ≥ z1` Detailed, `z2 ≤ zoom < z1` Reduced, `zoom < z2` Aggregation (zoom is `Camera.zoom` in pixels per metre, so far away is small; `MIN_ZOOM` 2 is always Aggregation and `MAX_ZOOM` 96 always Detailed). The thresholds are Video settings with il_render defaults: `Settings.detail_z1` / `detail_z2` (§15), `DETAIL_Z1 = 24.0`, `DETAIL_Z2 = 8.0` as starting values (decision: settings with engine defaults; `z1 > z2` enforced when applied). `build_snapshot` selects the tier once per frame from `SnapshotInput.camera.zoom` and the two thresholds and writes it to `RenderSnapshot.tier`:

  | Tier | Soldiers | Animation | Regiment blocks | Projectiles | Corpses |
  |---|---|---|---|---|---|
  | Detailed | one `SoldierInst` per visible soldier, full atlas frame | the animation column advances (`scene_from_snapshot` picks it from `time`) | none | drawn | drawn |
  | Reduced | one `SoldierInst` per visible soldier | none: one frame per soldier state (`SoldierInst.moving` picks the standing or walking frame's first column) | none | drawn | every fourth (`id % 4 == 0`) |
  | Aggregation | none for regiments in formation; Routing and Withdrawing soldiers still draw as Reduced sprites so a rout stays visible | none | one `BlockInst { anchor: [f32; 2], height: f32, facing8: u8, ranks: u8, files: u16, count: u16, side: u8, sprite_set: u16 }` per regiment rank block from `FormationState` (ranks × files at the template spacing, `count` the living soldiers), rendered as a faction-tinted quad through the sprite pipeline's atlas of block frames, shaded by density (`count / (ranks × files)`) | culled (REQ-RNDR-009) | culled (REQ-RNDR-009) |

  `RenderSnapshot` gains `tier: DetailTier` and `blocks: Vec<BlockInst>` (empty below Aggregation); `EntityCounts` gains `blocks`; the profiler overlay shows the active tier. REQ-RNDR-009 (a Phase 2 Should never audited) is this table's projectile and corpse columns plus the existing instancing. Tier boundaries and the block generation are unit-tested from `BattleView` (`crates/il_render/tests/snapshot.rs` pins the block count for a known regiment); a headless frame test runs when a `wgpu` fallback (software) adapter exists. Simulation LOD (PRD OQ-4, reduced-rate updates for regiments far from combat) is *not* built in Phase 3 unless T3-024 misses the 20k tick budget; T3-080 records the outcome.
- **Terrain.** As built (T1-053): `il_render::terrain::TerrainMesh::build(&LoadedMap, &Registries)` makes one vertex per height sample (`pos`, `height`, `shade` from the finite-difference normal under a fixed north-west light; 16 bytes) and two triangles per `height_cell` cell, plus an `R8Uint` zone-index raster at `zone_cell` (rows padded to 256 bytes; river cells without a `crossing` zone take slot 255 = water) and a 256-entry linear palette from `ZoneType.colour`. `terrain.wgsl` projects vertices with a 64-byte camera uniform that mirrors `Camera::world_to_screen`, writes depth 1.0 with no depth write so every sprite draws over it, and colours fragments from the palette times the shade with 2 m contour lines. Rivers and roads therefore come from the raster rather than separate strips; walls and gates as sprite strips arrive in Phase 5. `Renderer::set_terrain(&TerrainMesh)` uploads once per battle; `Renderer::render(&FrameScene { clear, camera, sprites, lines }, ui)` draws terrain, sprites and lines in one MSAA pass. Sprites take `height` from `LoadedMap::height_at` in `build_snapshot`. A line-list pipeline (`lines.rs`, `LineScene { vertices: Vec<LineVertex { pos, colour }> }`, screen-space, alpha-blended, no depth) draws the deployment outlines (`deployment_outlines`, ground-following, side tint) and serves the debug overlays (T1-054).
- **Debug overlays.** Line list pipeline fed from `BattleView` (nav grid, slots, paths, LOS radii, morale bars) toggled by `DebugFlags`. As built (T1-054): `il_render::debug::build_debug_lines(view, DebugFlags { nav_grid, slots, paths, anchors, spatial_cells, morale, flow, los, ai }, flow_side, camera, screen, &mut LineScene)` (`los`, T2-060: a ring at `BattleView::los_radius` around each regiment of `flow_side` and an X on every enemy anchor that side does not see; `debug_los` on `Ctrl+F5`; `ai`, T2-081: for every side owned by `PlayerId(255)` with a plan, the line segment with a facing tick, a link from each regiment to its role slot and a line to its charge or commit target; `debug_ai` on `Ctrl+F6`, the stance per AI side in the title) appends to the frame's line scene after the deployment outlines; every point is projected onto the terrain; grids are clipped to the visible bounds and skipped beyond 40k cells; the app toggles the flags through the `debug_nav_grid`, `debug_slots`, `debug_paths`, `debug_anchors`, `debug_spatial` bindings (F5..F9 by default; F1..F4 are the formation hotkeys) in `dev` builds and shows the enabled ones in the title.
- **Projectiles (T2-031).** `build_snapshot` reads `BattleView::projectiles()` and evaluates each arc at the interpolated time `tick − 1 + alpha` into a `ProjectileInst { a, b, height, side }`: a segment of `PROJECTILE_HALF_LENGTH` (0.4 m) either side of the position along the direction of travel, lifted by the ground height plus the arc height; culled like soldiers. The app projects both ends with `Camera::world_to_screen` at that height and appends them to the frame's `LineScene` in a fixed pale colour (no sprite, no atlas change; a sprite pass can replace it later).
- **Threading.** Phase 1: render on the main thread after the sim step from a `RenderSnapshot` (positions ×2, facings ×2, regiment blocks, projectiles, camera). Phase 3 (REQ-RNDR-007, SAD §8; specified in T3-003, built in T3-030): the winit loop, the sim step, `build_snapshot`, `scene_from_snapshot`, the debug lines and egui's tessellation stay on the main thread; `Renderer` (the wgpu device, surface, targets, atlases, instance buffers, present) moves to a render thread. Per frame the main thread sends one owned `FrameJob { snapshot: RenderSnapshot, sprites: SpriteScene, lines: LineScene, ui: UiOutput /* tessellated */, camera: Camera, screen: Vec2, clear: ClearColour, resize: Option<PhysicalSize>, vsync: Option<bool>, terrain: Option<Arc<TerrainMesh>> /* set once per battle */ }` over a bounded channel of one slot (`il_render::thread::{RenderThread, FrameSender}`) that *replaces* a stale unrendered job rather than blocking, so the accumulator never waits on the GPU; the render thread builds the instance buffers and presents, and reports its GPU submit time back through a second channel for the profiler, which shows both rows (frame build on main, GPU submit on render). Resize and vsync changes travel in the job; atlas and terrain uploads go through the same channel as one-off jobs. `il_app --single-thread-render` (`Launch.single_thread_render`, §15) keeps the Phase 2 path (the renderer on the main thread) for debugging and for machines where the surface must stay on the main thread; toggling it changes nothing but the profiler rows. `il_app --bench-sprites` runs through the thread. The snapshot type was designed in Phase 1 so only this plumbing changes (T-5).

```rust
// As built (T1-050..T1-054; T2-060 added `SnapshotInput.observer_side: Option<u8>`, `RegimentBlock.visible` and `RenderSnapshot.ghosts: Vec<GhostInst { id, side, pos, facing8, count }>`: with an observer side the soldiers of regiments it does not see are skipped and its memory of them (SIM-VIS-005) becomes ghosts, drawn by `terrain::ghost_markers` as grey diamonds with a facing tick through the line pipeline; `None` draws everything). Planned fields for later phases (LOD, the minimap fog texture) join RenderSnapshot with their features.
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
pub struct SoldierInst { pub pos: [f32; 2] /* world, interpolated */, pub height: f32, pub facing8: u8 /* not interpolated: facing snaps */, pub sprite_set: u16, pub side: u8, pub moving: bool, pub selected: bool, pub corpse: bool /* T2-022 */ }
pub struct Corpse { pub pos: [f32; 2], pub side: u8, pub sprite_set: u16, pub facing8: u8, pub died: Tick }   // T2-022: kept by BattleSession from SoldierDied for combat.corpse_ticks; drawn at half brightness a hair behind the living
pub struct SnapshotInput<'a> { pub alpha: f32, pub camera: Camera, pub screen: Vec2, pub selected: &'a BTreeSet<RegimentId>, pub corpses: &'a [Corpse] }
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
- **Selection model.** `Selection { regiments: BTreeSet<RegimentId>, groups: [BTreeSet<RegimentId>; GROUPS = 10] }`, only own side (which a side always sees, SIM-VIS-004). As built (T1-061): `Selection::{new, click(hit, add), box_select(hits, add), set, set_group(n), recall_group(n, add), retain, contains, len, is_empty, clear}`; hit testing lives in `il_ui::pick` (`pick_regiment`, `regiments_in_box`, `regiments_of_type_on_screen`, `own_regiments`, `owned(view, id, player)`) over `BattleView` soldier positions through a `Project<'a> = dyn Fn(V2) -> Vec2 + 'a` projection closure the app builds from `Camera` and `ground_height`, so il_ui never depends on il_render; only regiments whose side belongs to the local player are returned; a soldier's hit circle is centred half a body above its ground point with radius `max(6 px, 1.5 × drawn radius)`.
- **Command emission.** `UiIntent → Command` with `tick = now + 1 + input_delay`, `seq` from a per-player counter. Drag-formation → `GroupFormation` if > 1 regiment else `Move { facing }` and `SetFormation { ranks }` derived per SIM-FORM-042. As built (T1-062): `il_ui::orders::drag_formation(from, to, centroid, flip) -> Option<DragFormation { anchor, forward, width }>` works in world metres (the app unprojects the screen points): `anchor` is the drag midpoint, `width` its length (under `MIN_DRAG_WIDTH_M` = 1 m is no gesture), `forward` the perpendicular pointing away from the selection's anchor centroid (`selection_centroid`), negated by `flip` (the `order_flip_facing` modifier); `DragFormation::facing() -> Angle<S>`. `UiIntent::{Move { target }, DragFormation(DragFormation), Halt, Formation(u8), SpeedMode(SpeedMode)}` and `commands_for(intent, &OrderContext { view, regiments, speed })` return the `CommandKind`s in queue order: a single-regiment drag gives `SetFormation { ranks: Some(il_sim_battle::ranks_for_width(..)) }` then `Move { facing }`, a multi-regiment drag `SetSpeedMode` (the run toggle; `GroupFormation` moves at each regiment's current order speed) then `GroupFormation` with `battle_line_template` (the registry's first `battle_line` template); `Formation(n)` is one `SetFormation { ranks: None }` per distinct n-th template of the selected unit types; `BattleSession::queue` stamps `tick + 1 + input_delay` and the per-player `seq`.
- **Overlays.** `il_ui::overlay::{selection_box, drag_formation_preview}` draw the box-select rectangle and the drag-formation preview through egui's painter.
- **Panels (egui).** Planned: campaign province, settlement, army, diplomacy, research, faction, turn log, end-turn. As built (T1-070, T2-091): `main_menu(ctx, &MenuModel) -> Option<MenuChoice::{CustomBattle, Scenarios, Load, Settings, Exit}>` (the root) and `scenario_list` (`Start(i)`, `Back`); `custom_battle::custom_battle(ctx, &mut BuilderState, &BuilderCatalog { maps: Vec<MapChoice { id, name, zones, weather }>, factions: Vec<FactionChoice { id, name, units, generals }> }, locale) -> Option<BuilderAction::{Start, Save, RandomSeed, Back}>` with `BuilderState::{default_for, soldiers, to_setup(&catalog) -> Result<BattleSetup, BuildError>}` (REQ-UI-007, REQ-SIM-063: map, weather from `weather_allowed`, seed, time limit in minutes × 1200 ticks, 2..4 sides bounded by the map's deployment polygons, each a faction, a controller (You = player 0, Engine AI = player 255, Idle = 1, 2, …), a general of the faction's `general` category (every general in the registry when it lists none) and roster rows of the faction's units with count 1..1000 and experience 0..9; at most one You; the soldier cap counted with the generals; no positions, so the battle opens in Deployment; Save writes `Scenario { setup, commands: [] }` as pretty JSON into `--scenarios-dir`, a second click overwrites an existing file); `settings::settings_screen(ctx, &mut SettingsState { draft: SettingsDraft, tab, capturing: Option<Capture { row, slot }>, dirty, note }, locale, over_battle) -> Option<SettingsAction::{Apply, Save, Back}>` (REQ-INP-005; Video: `ui_scale`, vsync, borderless fullscreen, sim threads for the next battle; Audio: the volumes T2-100 reads; Bindings: every action with its chords, click a chord or `+` to capture the next key or mouse chord, `−` removes, Reset restores the mods' default, a chord bound twice is flagged; the app owns the capture from `InputState::key_presses` and the frame's clicks, `Chord::to_text` renders it); `load_screen::load_screen(ctx, &[SaveEntry { path, name, created, tick, summary, problem }], locale) -> Option<LoadAction::{Load(path), Back}>` (a save of another content or schema is listed with its problem and no button); `result::result_screen(ctx, &ResultScreenModel { winner, duration, sides: &[ResultSide { name, tint, fate, loot, rows: Vec<ResultRow { unit, initial, survivors, killed, fled, experience, ammo, arrived }> }], replay_path, can_rematch }) -> Option<ResultAction::{Menu, Rematch}>` (REQ-UI-004; the rows are named through the setup's rosters by `RegimentResult.id`; Rematch is the same setup with `seed + 1`). `battle_hud(ctx, &HudModel) -> Option<HudAction>` (`clock(tick) -> String` as `mm:ss`, speed, pause, the Menu button that opens the pause menu, the phase with the deployment's Confirm button, T2-090), `event_panel(ctx, &[EventLine])`, `profiler_overlay(ctx, &ProfilerStats { stages: Vec<StageStat { name, last_ms, mean_ms, max_ms }>, tick_last_ms, tick_mean_ms, tick_max_ms, ticks_sampled, frame_ms, fps, soldiers, regiments, visible_soldiers, ticks_last_frame, accumulator_alpha })`; `UiContext` / `UiOutput` wrap egui-winit so the app hands the tessellated output to il_render's `EguiPaint`. The battle screen (T2-090, REQ-UI-001, REQ-UI-003): `cards::card_strip(ctx, &CardStripModel { cards: &[RegimentCard { id, unit, soldiers, initial, morale_state, fatigue, volleys, engaged, selected, group, tint }] }) -> Vec<CardAction::{Select { id, add }, Centre(id)}>`, a scrollable column down the left edge, one card per own regiment (strength bar over `RegimentRow.initial`, a morale dot in the F10 palette, the fatigue state, the mean volleys left of a ranged unit, an engaged mark, the control group; click selects, Shift adds, a double click centres the camera; the same strip is the deployment tray); `command_card::command_card(ctx, &CommandCardModel { selection: &[SelectedRegiment], fire, run, armed_attack_move, formations, abilities: Vec<AbilitySlot>, presets, deploying }) -> Option<CommandAction::{Halt, ArmAttackMove, Withdraw, ToggleFire, ToggleRun, Formation(n), Ability(n), Preset(id)}>` at the bottom centre: the T1-070 selection rows (`selection_grid`; unit, soldiers, formation, order, morale, fatigue and, since T2-050, the ability slots with cooldowns and the active statuses) over buttons for every key order; during the deployment only the formations and the preset picker, which re-lays the whole side in its zone through `orders::preset_deploy_commands` (`arrange_group` at the zone centre facing the other zones, 0.8 × the zone's width, one `Deploy` each); `casualties::casualties_line(ctx, &[SideTally { name, tint, alive, killed, fled }])` at the top (alive from the view, the losses tallied by the app from `SoldierDied`, `SoldierFled`, `SoldierWithdrew`); `minimap::Minimap::show(ctx, &MinimapInput { map, zone_colours, zone_crossing, discs, blocks: &[MiniBlock], viewport }) -> Option<MinimapAction::{Pan(world), Order(world)}>` bottom right: `render_fog` paints a ≤ 256 px terrain texture (zone colours, water on open river cells) darkened to 45 % outside the union of the observer side's `los_radius` discs (the sim has no fog grid, only per-regiment visibility), rebuilt every `REFRESH_FRAMES` = 10, with a block per regiment the observer sees (remembered enemies as grey ghosts) and the camera's viewport quadrilateral; a left click or drag pans, a right click orders a `Move`; `pause_menu::pause_menu(ctx, &PauseModel { can_surrender, has_settings }) -> Option<PauseAction::{Resume, Surrender, Settings, Quit}>` on `pause_menu` (Escape) or the Menu button: opening pauses, Resume restores the earlier pause state, Surrender sends the command; Escape only disarms an armed attack-move first. `UiIntent` gained `AttackMove { target }`, `AttackRegiment { target }`, `Withdraw` and `GroupPreset { template }` (the selection's centroid, mean facing and lateral extent plus 20 m, at least 40 m; nothing during the deployment); `pick::pick_enemy_regiment(view, project, ppm, observer_side, cursor)` finds a visible enemy under a right click. The app keeps the armed attack-move cursor (`battle_ui::Armed`) and turns every panel click into the same `UiIntent` the key gives (`il_app::battle_ui`). Text scales with the window: the egui zoom factor is `logical window height / 1080 × ui_scale` (REQ-UI-006; `ui_scale` is a setting from T2-091). The rest arrives with its phase.
- **Localisation.** All labels via `Locale::get`; `il_app --show-keys` shows keys.

Budget: egui ≈ 1 ms per frame; minimap texture regenerated every 10 frames, at most 256 px on its long side.

Tests: gesture geometry (drag vector → facing, width) and intent → command conversion on the ten-regiment scenario, the combat intents and the presets in both phases (`crates/il_ui/tests/orders.rs`); picking, own and enemy (`tests/pick.rs`); the minimap's fog texture and mapping (`tests/minimap.rs`); every battle panel drawn headless with a filled model (`tests/panels.rs`); the builder's default draft builds a setup the sim opens in Deployment, its errors, a saved setup parsing back as a scenario and building the same world, the menu screens drawn headless and a captured chord landing in its slot (`tests/custom_battle.rs`, T2-091); binding parse, chord text round trip and selection rules inline in `bindings.rs` and `selection.rs`. Clicking the panels is the docs/08 §4f checkpoint.

## 12. Audio (`il_audio`)

```rust
// As built (T2-100). The sim never calls audio (REQ-AUD-001): the app hands each frame's events to the router.
pub trait AudioSink { fn play(&mut self, req: &PlayRequest); fn set_mix(&mut self, effects: f32, roar: f32); fn set_volumes(&mut self, v: Volumes); fn stop_all(&mut self); }
pub struct NullSink;          // records calls: the tests and machines without a device
pub struct PlayRequest { sample: SampleId, gain: f32, pan: f32 }
pub struct Volumes { master, effects, music }   // the settings' sliders, linear 0..1
pub struct FrameInput { camera_center: [f32; 2], zoom: f32, bounds: ([f32; 2], [f32; 2]), engaged: u32, now_ms: u64, observer_side: Option<u8>, tick: Tick }
pub struct RegimentPos { id: RegimentId, side: u8, pos: [f32; 2], unit: Handle<UnitType> }   // the frame's anchors, for regiment-only events
pub struct EventRouter;       // new(&SoundSet, &Registry<UnitType>); sample_paths(); set_durations(&[u32]); route(&FrameInput, &[BattleEvent], &[RegimentPos], &mut dyn AudioSink); reset(&mut dyn AudioSink)
pub fn near_weight(zoom, far, near) -> f32;   // sat(ln(zoom / far) / ln(near / far))
pub fn roar_gain(engaged, ref_engaged, near) -> f32;   // sat(engaged / ref) × (1 − 0.5 × near)
pub struct AudioEngine;       // kira: try_new() -> Result<Self, String>; load(sample_paths, roar_path, assets_dir) -> (durations_ms, warnings); impl AudioSink
```

- **Sound sets** (`il_data::SoundSet`, `content/sounds/*.json5`, `sound-set.schema.json`, Modding SDK §4.14): a closed `SoundEvent` list (`charge`, `cavalry_charge`, `clash`, `volley`, `arrow_hit`, `death`, `cavalry_death`, `rout`, `rally`, `shatter`, `general_died`, `ability`, `phase`, `victory`, `defeat`), each with `samples` (WAV paths under the mod's `assets_root`), `min_interval_ms`, `max_voices` and `gain`; the `roar` loop with `ref_engaged`; the `zoom` curve (`far`, `near` in camera pixels per metre); `max_voices` over every event; `cull_pad_m`. `Faction.sound_set` names the set the faction's player hears; the app falls back to the registry's first set. A unit's `sounds.charge` / `sounds.die` override the set's samples for its `Charge` and `SoldierDied` (cavalry without them fall back to `cavalry_charge` / `cavalry_death`); `select`, `move`, `attack` stay unread until Phase 6 (REQ-AUD-003). Audio-only: not hashed, never read by the sim.
- **Event mapping**: `Charge` → charge, `Engaged` → clash, `VolleyFired` → volley, `ProjectileLanded { hit }` → arrow_hit, `SoldierDied` → death, `MoraleChanged { to: Routing }` → rout, `Rallied`, `Shattered`, `GeneralDied`, `AbilityUsed` → ability, `PhaseChanged` → phase, `Ended` → victory for the observer's side, defeat for another side's win, phase for a draw or a spectator. Everything else is silent. The variant is `(tick + id) mod n`: no randomness anywhere in audio.
- **Listener**: a placed event outside the camera's visible bounds plus `cull_pad_m` is dropped; inside, gain × `(1 − 0.5 × d / half_diagonal)` from the camera centre and the pan from the x offset. `GeneralDied`, `PhaseChanged`, `Ended` have no position and never cull.
- **Rate limits**: per event `min_interval_ms` (a second play of the same event inside the interval, or in the same frame, is dropped) and `max_voices` (concurrent plays, each lasting its sample's length as the engine reported it); the set's `max_voices` over every event; at most `MAX_PLAYS_PER_FRAME = 8` new plays a frame.
- **Zoom mixing** (REQ-AUD-002): `near = sat(ln(zoom / far) / ln(near / far))`; the effects track's gain is `near` (at or below `far` only the roar plays), the roar loop's gain `sat(engaged / ref_engaged) × (1 − 0.5 × near)` where `engaged` sums the soldiers of regiments whose `engaged` flag is set; both tweened over 250 ms. Kira tracks: main (master), `effects` (effects slider × near), `roar` under it, `music` (present, silent until Phase 4). Linear sliders become `Decibels(20 log10 v)`, 0 = silence.
- **App** (`il_app::audio::AppAudio`): opens the device at start (`audio disabled: <reason>` once and a `NullSink` when there is none); on the first frame of a battle picks the set, loads the samples from `<content_root>/assets` (one warning per missing file) and starts the roar silent; every frame after the step hands the ticks' events, the view's regiment anchors and the camera to the router; the settings' Audio tab applies live; `--mute` starts the master at 0; leaving a battle stops the roar and forgets the limits.
- Music (REQ-AUD-004, Phase 4) is not built: no `MusicState`.

Budget: < 0.5 ms per frame (the router walks the frame's events once; a 10k battle emits a few hundred a tick). Tests (`crates/il_audio/src/router.rs`, `engine.rs`): the zoom curve's endpoints and geometric midpoint, the roar gain, only-the-roar at far zoom, the interval and the per-event and global voice caps, culling and panning, the cavalry fallback and the unit override, the stable variant, `Ended` → victory/defeat/phase, `reset`, `decibels`, and `AudioEngine::try_new` returning without a panic with or without a device. `il_cli gensound` (`crates/il_cli/src/gensound.rs`) writes the eighteen placeholder samples (22,050 Hz mono 16-bit, under 1 MB together) and `game/content/sounds/battle.json5`; its tests check the WAV headers, the normalisation, the roar's seam and that the set names every file.

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
// As built (T2-101). `Campaign` bodies, `Migrate` steps and checkpoints arrive with their phases.
pub const MAGIC: &[u8; 4] = b"ILSV"; pub const REPLAY_VERSION: u32 = 1;
pub enum SaveKind { Battle, Replay, Campaign }
pub enum Compression { None }   // the header names it, so zstd can join without a format break
pub struct SaveHeader { pub engine_version: String, pub schema_version: u32 /* SNAPSHOT_VERSION for battle bodies, REPLAY_VERSION for replays */, pub kind: SaveKind, pub mods: Vec<(String, String)>, pub content_registry_hash: u64, pub mod_list_hash: u64, pub created: String /* UTC YYYY-MM-DDTHH:MM:SSZ, no calendar crate */, pub turn: Option<u32>, pub tick: Option<u32>, pub summary: String, pub compression: Compression }
pub struct SaveFile { pub header: SaveHeader, pub body: Vec<u8> /* postcard */ }
pub fn encode(&SaveHeader, body: &[u8]) -> Vec<u8>; pub fn decode(&[u8]) -> Result<SaveFile, SaveError>;   // "ILSV", u32 LE header length, JSON header, body
pub fn write(path, &SaveHeader, body) -> Result<(), SaveError>; pub fn read_header(path) -> Result<SaveHeader, SaveError>; pub fn read(path) -> Result<SaveFile, SaveError>;
pub fn header_for(&Registries, SaveKind, schema_version, tick, summary) -> SaveHeader; pub fn content_matches(&SaveHeader, &Registries) -> bool;
pub trait Migrate { fn migrate(from: u32, body: Vec<u8>) -> Result<Vec<u8>, MigrateError>; }   // chain of version steps, one function per bump
pub struct Replay { pub setup: BattleSetup, pub commands: Vec<Command> /* fed: local, scripted, the --ai transfers */, pub ai_commands: Vec<Command> /* the engine AI's, kept apart */, pub hashes: Vec<StateHash> /* one per tick, tick 1 first */, pub checkpoints: Vec<(Tick, Vec<u8>)>, pub ended_tick: Option<u32> }
pub struct BattleSave { pub snapshot: Vec<u8> /* postcard Snapshot */, pub replay: Replay /* so far */, pub script: Vec<Command> /* not yet fed */, pub local_player: PlayerId, pub scenario_stem: String }
pub fn verify(&Replay, Arc<Registries>, threads) -> Result<VerifyReport { ticks, divergence: Option<Divergence { tick, expected, got }> }, SaveError>;
```

- Campaign save (Phase 4) = header + campaign snapshot + `script_state: Option<String>` (the Lua `il.state` table as JSON, Modding SDK §5) + (if mid-battle) battle snapshot and the pending `BattleId`.
- On load: registries rebuilt from the header's mod list (REQ-SAVE-004: missing required mod → refuse; different versions → warn); handles inside snapshots are stored as ContentIds and re-resolved during restore. A unit ContentId that no longer exists resolves to the engine placeholder `il:missing_unit` and is flagged in the UI; missing buildings and technologies are dropped; a missing faction aborts the load (Modding SDK §8). In Phase 2 the app and `il_cli replay --verify` load the content they were given and refuse a file whose `content_registry_hash` differs (`--force` verifies anyway).
- Replay recording is on in every battle (T2-101): the session keeps the fed commands, the AI's commands and every tick's hash in memory and writes `replays/<scenario-stem>-<YYYYMMDD-HHMMSS>.ilrp` once, at `Ended`, on Quit, on a load over the battle and on the window closing (a crash loses it; nothing is written per tick). Playback and verification run with the AI **on** and feed `commands` only, so the AI regenerates `ai_commands` and the hashes match bit for bit (with the AI off the hashed outbox would differ, §8.5); `ai_commands` is kept for the Phase 3 viewer and the network path. `il_app --replay <file>` is a watch-only playback (no local commands; the title shows `replay tick/ticks`, `replay OK` or `replay MISMATCH at tick N`; it stops at the recording's end). `il_cli autoresolve --record-replay F` records a headless battle and `il_cli replay F --verify [--threads T] [--force]` re-simulates it and prints `verified N ticks` or `divergence at tick T: expected X got Y` (exit 1); without `--verify` the header prints as JSON.
- Battle quick save (REQ-SAVE-006): `quick_save` (Ctrl+S) writes `saves/quick.ilsv` (a `BattleSave`); `quick_load` (Ctrl+L) or the main menu's Load screen (T2-091) rebuilds the session from the snapshot with the logs, hashes and the script's remainder, so the hash sequence continues unchanged and the replay written at the end covers the battle from tick 0. A quick save is refused after `Ended` and in a playback. Checkpoints every 1,200 ticks arrive with the Phase 3 viewer.

Tests (`crates/il_save/src/lib.rs`, `crates/il_app/src/session.rs`, `crates/il_cli/tests/replay.rs`): container round trip, bad magic, truncation and a bad header; header-only reads; UTC timestamp goldens; replay and save body round trips; a recording of the AI skirmish verifies on one and eight threads and a corrupted hash names its tick; save at tick 400 and load in a fresh session gives the same hashes to tick 800 as the uninterrupted run and the loaded session's replay verifies from tick 0; a playback feeds the recording, refuses local commands, stops at its end and notices a differing hash; `autoresolve --record-replay` then `replay --verify` end to end, a different content refused unless forced. The nightly records `ai_skirmish_300` to its end and verifies it on eight threads (§17).

## 15. App shell (`il_app`)

```rust
// As built (T1-070); Campaign and Editor states, `replay: Replay` and `net: Option<LockstepSession>` join with their phases.
pub enum AppState { MainMenu(MenuState { scenarios: Vec<PathBuf>, mods: Vec<PathBuf>, error: Option<String> }), Battle(Box<BattleSession>), Editor(Box<EditorSession>) /* T3-003 spec, T3-060 build: `il_editor::EditorSession` (§16), entered from the main menu's Editor button (`MenuChoice::Editor`, a map picker or New) through `Transition::OpenEditor { map: Option<ContentId> }`; its Quit is `QuitToMenu`; the editor owns the battle camera and a `TerrainMesh` of its document and renders through the same `FrameScene` */ }
pub enum MenuScreen { Root, Scenarios, CustomBattle(Box<BuilderState>), Settings(Box<SettingsState>), Load(Vec<SaveEntry>) }   // T2-091
pub struct MenuState { scenarios: Vec<PathBuf>, mods: Vec<PathBuf>, error: Option<String>, screen: MenuScreen }
pub enum Transition { StartBattle(PathBuf), StartSetup { setup: Box<BattleSetup>, stem, ai } /* the builder, Rematch; T2-091 */, LoadSave(PathBuf) /* T2-101 */, OpenEditor { map: Option<ContentId> } /* T3-060 */, QuitToMenu }
pub struct Settings { ui_scale: f32, vsync: bool, fullscreen: bool, threads: usize, volume: Volume { master, effects, music }, bindings: Vec<BindingOverride { action, keys }>, replays_dir: String, saves_dir: String, detail_z1: f32, detail_z2: f32 /* T3-003 spec, T3-031 build: the LOD thresholds of §10.1 on the Video tab, defaulting to il_render's DETAIL_Z1 / DETAIL_Z2; a file without them reads the defaults */ }   // il_app::settings, T2-091
pub enum SessionMode { Live, Replay { expected: Vec<StateHash>, mismatch: Option<Tick> } }   // T2-101
pub struct BattleSession { world: BattleWorld, accumulator: f64, speed: f32, paused: bool, local_player: PlayerId, input_delay: u32 /* 0 in Phase 1 */, next_seq: u16, pending: Vec<Command>, script: ScriptedCommands, command_log: Vec<Command> /* fed */, ai_log: Vec<Command> /* the engine AI's, T2-101 plan I1 */, hashes: Vec<StateHash>, events: VecDeque<EventLine> /* EVENT_RING = 256 */, corpses, result, ai_players, mode: SessionMode, scenario_stem: String }
impl BattleSession { pub fn new(world, local_player, script, ai_players: Vec<PlayerId> /* `--ai`: TransferControl to the engine at tick 1, issued as the engine */); pub fn with_stem(self, stem); pub fn from_replay(Replay, regs, threads) /* playback: AI on, no local commands */; pub fn from_save(BattleSave, regs, threads) /* restore + logs + the script's remainder */; pub fn replay(&self) -> Option<Replay>; pub fn save(&self) -> Option<BattleSave>; pub fn queue(&mut self, kind: CommandKind) /* ignored in a playback */; pub fn target_tick(&self) -> Tick /* tick + 1 + input_delay */; pub fn advance(&mut self, dt: f64) -> Vec<StepOutput>; pub fn advance_with(&mut self, dt: f64, observer: &mut dyn StageObserver) -> Vec<StepOutput> /* stops at Ended and at a recording's end */; pub fn alpha(&self) -> f32; pub fn speed / set_speed(f32) /* records SetSpeed { mult_x100 } */; pub fn paused / set_paused(bool) /* records Pause */; pub fn surrender(); pub fn note(text); pub fn local_player; pub fn observer_side; pub fn command_log; pub fn ai_log; pub fn hashes; pub fn events; pub fn replay_finished / replay_mismatch }
pub const TICK: f64 = TICK_SECONDS; pub const MAX_CATCHUP_TICKS: u32 = 4;
pub struct Profiler;   // the app's StageObserver over `Instant` (SAD §9.3): `frame(frame_seconds, ticks_stepped)`, `stats() -> ProfilerStats` over a 60-tick window
```

Frame: poll winit → `il_ui` intents → commands stamped `tick + 1 + input_delay` into `pending` → `accumulator += dt × speed` (capped at `MAX_CATCHUP_TICKS` = 4 ticks, a constant in `session.rs` rather than a rules field) → while `accumulator ≥ TICK`: gather commands for `world.tick()+1` (local pending, AI internal, network) → `step` → route events to audio/UI/replay → `accumulator −= TICK` → build `RenderSnapshot(alpha)` → render → egui. Campaign state runs `apply` on intents and `end_turn` on End Turn; `BattleRequested` switches state; `Ended` returns the result via `resume_after_battle` or the auto-resolve path.

Tests (inline in `state.rs`, `session.rs`, `profiler.rs`): accumulator never runs more than the cap; pause records a `Pause` command; state transitions; the profiler window.

As built (T1-070): `il_app::state::AppState::{MainMenu(MenuState), Battle(Box<BattleSession>)}` with `AppState::apply(self, Transition::{StartBattle(path), QuitToMenu}, start, menu)` a pure function (a failed start keeps the menu up with the error); `MenuState::scan(scenarios_dir, mods)` lists `*.json5` under `--scenarios-dir` (default `tests/scenarios`) and the mod roots; the menu is `il_ui::main_menu`, the battle HUD (`mm:ss` clock, speed, pause, menu, the selection card with localised unit and formation names) `il_ui::battle_hud`, and `il_ui::event_panel` shows the session's 256-entry event ring (`BattleSession::events`, the routing stub: every `BattleEvent` and rejected command as text) in `dev` builds with the profiler. `BattleSession { world, accumulator: f64, speed: f32, paused, local_player, input_delay: u32 (0 in Phase 1), next_seq: u16, pending, script: ScriptedCommands, command_log, events }`; `queue(kind)` stamps `tick + 1 + input_delay` and the per-player `seq`; `advance_with(dt, observer)` caps the accumulator at `MAX_CATCHUP_TICKS = 4` ticks and returns one `StepOutput` per tick; `alpha()` feeds `build_snapshot`. A scenario on the command line starts in `Battle`; the `pause_menu` binding (Escape) or the HUD's Menu button opens the pause menu, whose Quit returns to the menu and drops the session (T2-090; Escape was `quit_to_menu` until then); transitions apply after the frame's render. Command line: `il_app [scenario.json5] [--content-root game] [--mod DIR]... [--scenarios-dir tests/scenarios] [--threads N] [--bench-sprites] [--show-keys] [--ai P]... [--replay F] [--replays-dir D] [--saves-dir D] [--settings F] [--mute] [--single-thread-render]` (`Launch { content_root, mods, scenarios_dir, threads, bench_sprites, ai, replays_dir, saves_dir, settings, settings_path, mute, single_thread_render /* T3-003 spec, T3-030 build: §10.1 Threading */ }`; T2-100: `App.audio: AppAudio` routes every stepped tick's events after the render snapshot, §12; T2-091: `settings.json5` is read first from `--settings`, else `%APPDATA%\IronLegion\` (Windows), `$XDG_CONFIG_HOME/IronLegion` or `~/.config/IronLegion`, else the working directory, through the environment alone (`il_app::settings::config_dir`); a missing file is the defaults, a bad one the defaults plus a warning; `--threads`, `--replays-dir` and `--saves-dir` win over the file's values; the bindings run with the file's overrides applied by action (`settings::effective_bindings`, re-applied on a hot reload); the egui zoom factor is `logical window height / 1080 × ui_scale`; vsync and fullscreen apply once the window exists and again on Apply; the main menu's screens (`MenuScreen`) and the pause menu's Settings are driven by `il_app::menus` (`builder_catalog`, `draft_from` / `settings_from`, `captured_chord`, `save_entries`, `write_scenario`, `App::{menu_frame, settings_action, apply_settings, apply_window_settings, capture_chord}`); a settings screen capturing a chord owns the frame's input; T2-101: `--replay` starts a watch-only playback, the replay of every live battle lands under `--replays-dir`, `quick_save` / `quick_load` use `<saves-dir>/quick.ilsv`, `il_app::replay_io` writes and reads both through `il_save`; the replay is written once per battle at `Ended`, on Quit, on a load over the battle and on the window closing; the result window names the file). The `dev` feature (on by default) starts `il_data::HotReload` over the mod roots and polls it every frame, shows the profiler and event panels (`toggle_profiler`, F12) and the F5..F9 debug overlays; `cargo build -p il_app --no-default-features` is the shipping configuration and CI builds both.

## 16. Editors (`il_editor`)

- **Map editor (Phase 3; specified in T3-003, built in T3-060..063).** A new crate `crates/il_editor` (SAD §5.2: a presentation crate that may depend on `il_core`, `il_data`, `il_sim_battle` for `NavGrid` and `LoadedMap`, `il_render` and `il_ui`, never on `il_app`; it may write files, so its own `clippy.toml` re-allows `std::fs` like il_data's). Operates on `MapDef` (`id`, `name_key`, `size`, `campaign_terrain_tags`, `weather_allowed`, heightmap `Vec<f32>` at `height_cell` stored as a 16-bit raw sidecar, `base_zone`, `zones` polygons (fords and bridges are polygons of a `crossing: true` zone type laid over a river), `rivers`, roads, `deployment` polygons, `reinforcement_edges`, and the reserved `structures` and `siege_points` lists; Modding SDK §6.1 shows the JSON5).

  ```rust
  pub struct MapDocument { pub def: MapDef, pub heights: Vec<f32> /* height_cols × height_rows */, pub dirty: bool }   // T3-060: `from_registry(&Registries, id)`, `blank(size, height_cell, base_zone)`, `save(&self, mod_root) -> Result<Saved { json5, hgt }, EditorError>` writing `content/maps/<id>.json5` and `assets/maps/<id>.hgt` exactly as `il_cli genmap` does (the writer is shared: `il_data::map_def::write_map`), `to_loaded(&self) -> LoadedMap` for the terrain view and the nav preview
  pub struct History { undo: VecDeque<MapDocument>, redo: Vec<MapDocument>, cap: usize /* 64 */ }
  pub enum Tool { Select, HeightBrush { op: Raise | Lower | Smooth | Flatten, radius, strength, falloff: Falloff }, ZoneBrush { zone: Handle<ZoneType>, radius }, River { width }, Road { width }, Polygon { zone: Handle<ZoneType> }, Deployment { side: u8 }, ReinforcementEdge { side: u8 }, Structure { kind }, SiegePoint { kind } }   // T3-061 brushes, T3-062 vector tools
  pub struct EditorSession { pub doc: MapDocument, pub history: History, pub tool: Tool, pub camera: Camera, pub terrain: TerrainMesh /* rebuilt on a debounced edit, within a frame of the last stroke */, pub zone_raster: Vec<u8> /* the zone brush's working raster at `movement.zone_cell`, turned into polygons on save: marching squares then simplification, later polygons overriding earlier ones (SIM-MOVE-031) */, pub nav_preview: NavPreview { grid: Option<NavGrid>, worker: Worker /* `NavGrid::from_map` on a thread, debounced; drawn as the F5 overlay with corridor widths under 12 m marked */ }, pub diagnostics: Vec<Diagnostic> /* `il_data`'s validation of the merged document value, shown in a panel with the field name; Save is disabled while an error stands */, pub target_mod: Option<PathBuf> /* the folder picker's choice; never `game/` unless chosen; a save into a loaded mod triggers the hot-reload path */ }
  ```

  Tools: raise/lower/smooth/flatten height brush (radius, strength, falloff curve in a tool panel), zone paint brush, polyline tool (rivers with width, roads rasterised to a road polygon), polygon tool (ford and bridge zones, arbitrary zone polygons), deployment tool (one polygon per side plus reinforcement edges), structures and siege points (placeable, inert until Phase 5), metadata panel (`id`, `name_key`, `size`, `campaign_terrain_tags`, `weather_allowed`, `base_zone`), vertex drag and delete on every polygon; live nav grid preview; undo/redo over 64 steps; save to `content/maps/<id>.json5` plus the `.hgt` sidecar for the heightmap (JSON5 stores the reference and cell size; `il_cli genmap` writes the Phase 1 test map the same way). Budget: a brush stroke frame under 2 ms on the 1600 × 1200 m map (T3-061). Tests: a golden round trip of a painted hill and forest through save and reload (`height_at` and `zone_at` samples equal), `il_cli validate` clean on `tests/mods/editor_out/` after saving `rome:test_field` byte-identically (T3-060), the corridor closing in the preview when rock is painted across the bridge (T3-063).
- **Unit and formation editors (Phase 6).** egui property grids over `Registry<UnitType>` and `Registry<FormationTemplate>` entries with schema-driven widgets (from the JSON Schema `description`/ranges); preview panel renders a formation at chosen `n`; save writes the item into the selected mod folder with a `$override: "merge"` diff if it derives from another mod's item.

## 17. Testing and CI

| Test | Location | Runs | Requirement |
|---|---|---|---|
| Unit tests per formula | each crate | every push | REQ-TEST-001 |
| Utility-AI curves and selection (T2-080): curve goldens, threshold and tie-break, lazy noise, cadence, identical choices on eight threads | `crates/il_ai/tests/curves.rs`; the two AI kinds' load diagnostics in `il_data::ai` | every push | REQ-AI-001, REQ-AI-005 |
| Battle AI behaviour (T2-081/082): square against cavalry, phalanx when engaged frontally, deployment geometry, engage, hold fire, abilities, fall back, the bodyguard, zero rejected AI commands over a fight, 1 vs 8 threads with a restore, a replay with the AI off; stance and hysteresis, the stepping line, skirmishers, flank groups, defend ground, counter-charges, reserves, the retreat screen | `crates/il_sim_battle/tests/ai.rs`, `tests/ai_army.rs`; `tests/scenarios/ai_skirmish_300.json5` in the determinism corpus; `il_cli autoresolve` on `phases_all_four.json5` AI versus AI | every push | REQ-AI-003, REQ-AI-005 |
| Audio router (T2-100): zoom curve, roar gain, rate limits, culling and panning, unit overrides, `Ended` mapping; the engine constructor without a device; `gensound` WAV headers and determinism | `crates/il_audio/src/{router,engine}.rs`, `crates/il_cli/src/gensound.rs`; `dep_rules.rs` keeps audio out of the sim | every push | REQ-AUD-001, REQ-AUD-002 |
| The 20k tick (T3-022/023/024): the pre-checked and narrowed collision fold equal the plain fold bit for bit; `genmap` presets (`test_field` regenerates the committed map byte for byte, `plains` at any size); the 20k fight `tests/scenarios/large/perf_20k.json5` (a scripted attacker into the engine AI on `rome:wide_field`) timed to its end by `bench --scenario`, keyed `perf_20k` in the baseline and compared warn-only in CI at 300 ticks; the 32k cap file `large/cap_32768.json5` with `bench --memory` (T3-025) | `crates/il_sim_battle/src/movement/collision.rs`, `crates/il_cli/src/genmap.rs`, `tests/scenarios/large/`, `docs/evidence/phase3/{bench_perf_20k,memory_32k}.md` | every push (unit tests, the schema test walks `large/`), the bench lines in CI, the large runs by hand | REQ-PERF-003, REQ-PERF-004, REQ-PERF-007 |
| Determinism: each scenario twice, 1 thread and 8 threads, snapshot/restore at mid-point (the file's `determinism: { ticks, snapshot_at }` budget, default 10,000 / 5,000; `perf_10k.json5` runs 800 / 400 in the debug build, T2-111; the band files 1,500 / 1,000 through the band harness's `SeedDriver` with their pins and harness applied, and identical rejection counts rather than none for the AI-driven files, T2-112) | `tests/tests/determinism.rs`, three tests (the classic scenarios, the band files, the 10k fight), in-process on `BattleWorld` (`set_threads(1)` and `set_threads(8)`), plus an in-process `il_cli::run` twice comparison; CI also diffs two `il_cli run` logs per scenario (`idle_1000` and `move_reform_2000` for 10,000 ticks every 1,000, `perf_10k` for 1,500 every 100) | every push | REQ-TEST-002 |
| HPA\* graph (T3-020): cluster geometry and gate runs on an open grid, one gate per run at or under the split, every intra edge cost equal to the cropped-cluster Dijkstra oracle on 40 random grids, inter edges across a border at the destination's step cost, `repair` equal to a fresh `build`, `matches` / `ensure`; golden gate nodes on `tests/maps/tiny` (2) and `rome:test_field` (1,382) with the banks connected through the graph, and the same graph serial and on 8 threads | `crates/il_sim_battle/src/hpa.rs`, `crates/il_sim_battle/tests/hpa.rs`; `benches/benches/nav.rs::hpa_build_*` | every push | REQ-PATH-001 |
| HPA\* search (T3-021): 1,000 random requests on 25 %-rock grids agree with A\* on reachability, never cross an impassable cell, keep every pulled segment clear, and cost within 10 % of A\* as a corpus mean at the flagship rules (the per-configuration spread printed); the world's pathfinder crosses the river at the bridge; Stage 3 serves the same paths at 1 and 8 threads; the bridge column morph (T1-042) | `crates/il_sim_battle/src/hpa.rs`, `tests/nav.rs`, `tests/movement.rs`; `benches/benches/nav.rs::hpa_find_corner_to_corner` against `astar_find_corner_to_corner` | every push | REQ-PATH-001, REQ-PATH-002 |
| Content validation of `game/` | `tests/content.rs` | every push | REQ-TEST-005 |
| Every scenario file under `tests/scenarios/` (band files included) validates against `docs/schemas/scenario.schema.json`, and the schema accepts a `units` composition while rejecting both regiment forms together, an empty composition and a zero count (T3-002; `il_data::schema::validate_free` compiles any 2020-12 schema for tests) | `tests/tests/content.rs` | every push | REQ-FORM-008, REQ-SIM-063 |
| Every rules-schema property is read by a system in `il_sim_battle` or `il_ai`, or allow-listed with its reason (T2-113, closing T2-010's grep check) | `tests/tests/rules_fields_read.rs` | every push | REQ-TEST-001 |
| Scenario outcome bands (Simulation Spec §15.3), 50 seeds | `il_cli bands tests/scenarios/bands` (`il_cli::bands`: per file, per seed a single-threaded `BattleWorld` fed the scripted commands, seeds spread over `--jobs` OS threads; assertions evaluated over `BattleView` rows; a file's `bands.mods` load extra rules overrides for that file, and `mean_loss_matches` clauses are settled across files after every file has run, T2-032; `bands.pin_morale` holds listed sides at morale 100 after every tick and the `casualties`, `routed_before_loss` and `mean_loss_matches` clauses count the dead only, the fled being tracked separately, T2-042; `bands.harness` lists interventions applied through `ecs_mut` before a tick, `kill_general: side`, and the `routs_first { side }` clause reads which side's first Routing regiment came first, T2-043; T3-011: `SeedOutcome.rejected_by_tick`, `FileReport.ai_driven` and `BandReport.rejected_scripted`), driven in-process by `tests/tests/scenarios.rs` (`#[ignore]`; the non-ignored tests parse every band file on each push and run one seed of each for 200 ticks). Rejected commands (T3-011): the files whose commands are all scripted must reject none; the files with an engine-owned side (`il_cli::bands::ai_driven`, the two AI rows) may hit the one-tick race of SIM-CMD-005, so the test runs them a second time under the same options and requires every seed to reject the same count on the same ticks and to end on the same hash | nightly (`.github/workflows/nightly.yml`) and on demand | REQ-TEST-004 |
| Benchmarks per stage at 2k/10k/20k and on the 10k fight (`--scenario tests/scenarios/perf_10k.json5`, keyed `perf_10k`, T2-111) against the budget table at the top; fail at +20 % over the baseline | `il_cli bench` (per-stage mean/p95/max through `StageObserver`; `--baseline benches/baseline.json --strict`; the `StageTimer` is il_cli's one allowed `Instant` user because it only observes stage boundaries) plus criterion micro-benches in `benches/benches/` (`spatial`, `formation`, `nav`, `layout`, `tick`; T1-080) | every push, warn-only on CI runners; `--strict` on the target machine (`docs/evidence/phase1/machine.md`) | REQ-TEST-003, REQ-PERF-005 |
| Replay verify (T2-101) | `il_cli autoresolve tests/scenarios/ai_skirmish_300.json5 --record-replay F` then `il_cli replay F --verify --threads 8` (the generated file is the `tests/replays/` of the plan; nothing binary is committed); every push runs the in-process recording, verification, save/load and playback tests (`crates/il_cli/tests/replay.rs`, `crates/il_app/src/session.rs`) | nightly (`.github/workflows/nightly.yml`) | REQ-SAVE-005, REQ-SAVE-006 |
| Cross-machine hash compare | manual runbook, `il_cli run --hash-log` on two machines and `il_cli desync-report` | before Phase 7 | REQ-TEST-006 |

`il_cli` subcommands: `run <scenario.json5> --ticks N [--hash-every K] [--threads T] [--snapshot-at T] [--restore-from F] [--mod DIR]...` (a scenario is a `BattleSetup` plus an optional `commands: [Command]` list fed by tick, T1-081; a restored run skips the commands up to the snapshot tick), `bench (--soldiers N | --scenario F) [--ticks T] [--threads] [--json F] [--baseline F] [--strict] [--record-baseline F --machine M --recorded D]` (T1-080; the setup is generated in code: `N / 200` regiments of alternating infantry on `rome:test_field` with a 600-tick move/reform script; T2-111: `--scenario` times a scenario file instead, 1,200 ticks by default, stopping at `Ended`, keyed in the baseline by the file's stem), `replay <file> [--verify] [--threads T] [--content-root D] [--mod DIR]... [--force]` (T2-101, §14), `validate <mods...>`, `bands <dir|file> [--seeds N] [--max-ticks T] [--jobs J] [--json F] [--content-root D] [--mod DIR]...` (T2-110; exit 1 when an active assertion fails; its per-seed `SeedDriver` also drives the determinism test's band corpus, T2-112), `desync-report <log_a> <log_b>`, `autoresolve <scenario.json5> [--ai all|none|1,2] [--max-ticks N] [--threads T] [--json F] [--mod DIR]... [--record-replay F]` (T2-071/T2-082: the engine takes the listed players (default all) at tick 1, the rest run their scripted commands, to Ended or the cap; the `BattleResult` as JSON, exit 2 when it did not end; `--record-replay` writes the battle's replay, T2-101), `genart [--mod-root]` (placeholder sprite sheets, T1-051), `genmap [--mod-root] [--id] [--seed]` (the deterministic Phase 1 test map and its heightmap, T1-030).

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
