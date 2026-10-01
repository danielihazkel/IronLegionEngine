# Editor brush timing (T3-061)

Taken 2026-10-01 on the target machine (`machine.md`), release build:

```
cargo test --release -p il_tests --test editor -- --ignored --nocapture
```

`brush_dab_timing` opens a blank 1600 × 1200 m map (height cell 4 m, zone cell 2 m) and runs 100 dabs of each
brush at a 100 m radius along a line, each dab with the session's patches of the sim view (`LoadedMap`) and the
terrain mesh's block:

| Brush | Mean per dab |
|---|---|
| Height, Raise | 0.061 ms |
| Height, Smooth | 0.083 ms |
| Zone | 0.040 ms |

The budget is 2 ms per stroke frame (TDD §16). Not included: the app's re-upload of the whole terrain mesh on a
frame that changed it (a clone of the CPU mesh into the next frame job, then new GPU buffers on the render thread).
