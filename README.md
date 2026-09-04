# Iron Legion Engine

A Rust 2D strategy engine: turn-based campaigns and real-time battles of tens of thousands of individually simulated soldiers. Phase 1 (a rendered battlefield with movement and formations) is complete; combat is Phase 2.

- **Run it:** [docs/08-how-to-run.md](docs/08-how-to-run.md)
- **Read the design:** [docs/README.md](docs/README.md) is the map of the requirements, architecture, simulation rules, technical design and modding SDK.
- **What is being built next:** [docs/07-tasks-phase-0-2.md](docs/07-tasks-phase-0-2.md)

```
cargo run --release -p il_app -- tests/scenarios/move_reform_2000.json5 --threads 8
```
