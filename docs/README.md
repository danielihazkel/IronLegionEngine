# Iron Legion Engine — Documentation

Design documents for the Iron Legion Engine, a Rust 2D strategy engine for turn-based grand campaigns with real-time battles of tens of thousands of individually simulated soldiers.

## Document map

| # | Document | Owns | Status |
|---|---|---|---|
| 00 | [Glossary](00-glossary.md) | Every shared term. Other docs link here, never redefine. | v0.2 |
| 01 | [Product Requirements Document](01-prd.md) | *What* and *why*. All `REQ-*` IDs, priorities, phases, roadmap, risks, open questions. All numbers (tick rate, caps, performance ladder) are stated here once. | v0.2 |
| 02 | [Software Architecture Document](02-sad.md) | *Shape*. Crates, dependency rules, runtime loops, data flow, concurrency model, determinism rules, decision log. | v0.1 |
| 03 | [Simulation Design Spec](03-simulation-spec.md) | *Rules*. Every battle and campaign rule as a numbered `SIM-*` item with formulas and named tunables. | v0.1 |
| 04 | [Technical Design Document](04-tdd.md) | *How*. Rust types, traits, components, systems, schedule, schemas, budgets, tests, per subsystem. | v0.1 |
| 05 | [Networking Architecture Spec](05-networking-spec.md) | Future lockstep multiplayer and what the single-player code must already do for it. | v0.1 |
| 06 | [Modding SDK Spec](06-modding-sdk-spec.md) | Mod packages, manifests, override semantics, content reference, Lua API, editors. | v0.1 |
| 07 | [Task List, Phases 0–2](07-tasks-phase-0-2.md) | Implementation tasks with done-criteria, sizes, dependencies, and per-phase exit checklists. | active |
| — | [schemas/](schemas/) | JSON Schema drafts for content files referenced by the TDD and Modding SDK. | draft |

The original LaTeX PRD (`../iron_legion_engine_prd.tex`) is v0.1 and is kept for history only.

## Reading order

1. New to the project: Glossary → PRD §1–3 → SAD §1–6.
2. Implementing a battle system: PRD section for the area → Simulation Spec section → TDD section.
3. Writing content or a mod: Modding SDK → schemas.
4. Preparing multiplayer: Networking Spec §9 checklist first.

## Ownership rules

- A fact lives in exactly one document. Others link to it.
- Requirement IDs (`REQ-<AREA>-nnn`) are defined only in the PRD. Simulation rule IDs (`SIM-<AREA>-nnn`) only in the Simulation Spec. Architecture decisions (`ADR-nnn`) only in the SAD.
- Data field names used in Simulation Spec formulas are the same identifiers used in TDD structs and Modding SDK schemas.
- Every SAD component, Simulation Spec section, and TDD section lists the REQ IDs it satisfies.

## Traceability matrix (PRD area → owning sections)

| PRD area | REQ prefix | SAD | Simulation Spec | TDD | Networking | Modding |
|---|---|---|---|---|---|---|
| Vision, boundary | VIS | §2, §5 | — | §1 | — | §1 |
| Platforms | PLAT | §9 | §2 | §1, §2 | §3 | — |
| Performance | PERF | §2, §10 | §1 | every section's budget | — | — |
| Technology | TECH | §5, §11 | — | §1 | — | §5 |
| Determinism, sim core | SIM 001–009 | §3, §8, §9 | §2 | §2, §4 | §5, §9 | — |
| World, battle flow, terrain, visibility | SIM 020–053 | §6 | §1, §5, §11, §12 | §4, §6 | — | §4 (maps) |
| Campaign ↔ battle | SIM 060–064 | §6.4 | §12, §14 | §4, §9 | §6 | — |
| Formations | FORM | §5 | §4 | §7 | — | §4 |
| Pathfinding, spatial | PATH | §5 | §5 | §5, §6 | — | — |
| Combat, commanders | CMBT | §5 | §6, §9 | §8 | — | §4 |
| Abilities | ABIL | §5 | §10 | §8 | — | §4 |
| Morale | MOR | §5 | §7 | §8 | — | §4 |
| Fatigue | FAT | §5 | §8 | §8 | — | §4 |
| Campaign | CAMP | §5, §6.3 | §14 | §9 | §6 | §4 |
| AI | AI | §5 | §13, §14 | §8, §9 | — | §4 |
| Rendering | RNDR | §5, §6.1 | — | §10 | — | — |
| UI | UI | §5 | — | §11 | — | §5 |
| Input | INP | §6.1 | §3 | §11 | — | — |
| Audio | AUD | §5 | — | §12 | — | §4 |
| Localisation | LOC | §7 | — | §3 | — | §7 |
| Modding | MOD | §7 | — | §3, §13 | §4 | all |
| Saves | SAVE | §7 | §2 | §14 | §5 | §8 |
| Networking | NET | §6.1, §11 | §2, §3 | §4 | all | — |
| Tooling | TOOL | §5 | — | §16, §17 | §5 | §6 |
| Testing | TEST | §9 | §15 | §17 | §5 | §3 |

## Status legend

`draft` = being written · `v0.x` = reviewable · `accepted` = implementation may start against it.
