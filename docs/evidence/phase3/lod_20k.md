# The 20k fight at every zoom tier (T3-031)

Owner's sweep on 2026-09-14, the target machine of `machine.md`, `cargo run --release -p il_app --
tests/scenarios/large/perf_20k.json5 --threads 8` (the render thread of T3-030 on, the Video tab's
thresholds at their defaults `z1` 24 / `z2` 8), during the melee. The frame rate is the profiler's
presented FPS; `profiler_20k.png` is the screenshot at zoom 2.0 (tick 2,116, 19,938 alive).

| Zoom (px/m) | Tier | FPS |
|---|---|---|
| 96 | Detailed | 60 |
| 24 | Detailed | 60 |
| 12 | Reduced | 60 |
| 8 | Reduced | 60 |
| 4 | Aggregation | 45 |
| 2 | Aggregation | 38 |

The screenshot at zoom 2.0 reads 26 frames presented per second at that moment, with the main thread's
frame at 60.3 ms holding two sim ticks, the sim tick 41.1 ms per tick (40.5 mean, 51.7 max over 60 ticks)
and the render thread at 1.2 ms per frame (94 blocks, 318 soldiers drawn). The render side is no longer
the limit: the sim tick runs on the main thread, and at 41 ms of every 50 ms it leaves the frame nine
milliseconds, and whenever a tick runs over 50 ms the accumulator steps two ticks in one frame. The
Done when's "at least 30 FPS at every zoom and at least 60 at strategic zoom" is therefore not met at the
two far zooms; the owner ticked the task on 2026-09-14 with the shortfall recorded here and in SAD §12
T-16, the sim thread being the next lever.
