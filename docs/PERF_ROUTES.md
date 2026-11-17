# Performance routes and A/B runs

Hand-played comparisons are noisy: a different spot on the map, a camera turn or a load in one run and not the other can move the
numbers more than the change being tested. A route is a script that plays the same thing every time, unattended, and marks the
stretches to measure ("scenes"). `tools/perf/ab_run.ps1` runs it several times for each of two settings and
`tools/perf/perf_report.py` compares the logs.

This needs the developer build (docs/BUILDING.md) and a save to load. Python comes from `tools\.venv` (or `python` on PATH).

## 1. The route

`tools/perf/routes/fefw_map.tt` boots the game, presses through the title menu into save slot 000, waits for the map to settle,
then measures two scenes:

- `map_still`: 20 s with nothing moving: the cost of drawing the scene.
- `map_pan`: 11 s with the cursor panning the camera right, up, left and down: camera turns, texture streaming, culling.

Waits are in host time (`@sleep`), not frames, so the route covers the same game time at any frame rate.

**Make it fit your save once.** Put the save you want to measure in slot 000 (a battle map with the units on it works well), then
run the route by hand and watch it:

```
set NXRT_TEST_SCRIPT=C:\ObeliskChamber-src\tools\perf\routes\fefw_map.tt
set NXRT_TEST_SHOT_DIR=C:\ObeliskChamber-src\work\perf\shots
game-build\fefw\build\release\ObeliskChamber.exe --ini C:\ObeliskChamber-run\ObeliskChamber.ini
set NXRT_TEST_SCRIPT=
```

The screenshots `01_title` … `05_end` show where each press landed. If the title needs another press, or the slot screen sits
elsewhere, change the `press` and `@sleep` lines (the grammar is in `runtime/include/nxrt/tt_script.h`). The run ends by itself at
`@pass`; the log says `tt: run ended: pass`.

Other routes are new `.tt` files: any number of `@log SCENE start <name>` / `@log SCENE end <name>` pairs.

## 2. What a scene measures

At `SCENE end` the log gets one line for exactly the frames between the two marks:

```
hitch: scene 'map_still': 2340 frames in 20.0 s: mean 117.0 fps, 1% low 92.3 fps, worst 14.1 ms, 0 over 25 ms, 0 over 50 ms,
       0 over 2x the median (8.40 ms); render thread ms/frame: replay 5.62, present 2.10 (limiter 1.90), idle 0.40
```

`[nvn] replay_stats = 600` (render-thread cost per draw) and `[prof] gpu_stats = 600` (GPU time per frame) add their own lines; the
report uses the ones that fall inside the run.

## 3. A/B runs

```
pwsh -File tools\perf\ab_run.ps1 -B "NXRT_NVN_REAPER=false" -Names on,off
```

- `-A` and `-B` are environment variables, `NAME=value` separated by `;`. Every ini key has one: `NXRT_<SECTION>_<KEY>`
  (runtime/docs/CONFIG.md); it overrides the ini for that run only. Leave one side empty for "the ini as it is".
- `-Runs 3` (default) runs each side three times, alternating A B, B A, A B, so slow drift over the session falls on both.
- Logs, screenshots and `report.md` go to `work\perf\<date-time>`. Each run takes about two minutes; don't use the PC meanwhile.
- `-Exe`, `-Ini`, `-Route`, `-Out` point elsewhere; `-TimeoutS` (600) stops a run whose route got stuck.

## 4. The report

`report.md` has one table per scene: each metric's median over the runs of each side, the spread (min..max) when there were
several runs, and the change from A to B (marked better or worse past 3%). Above it, each run's setup (output size, DLSS mode and
model, HDR, which switches were set by the environment) and warnings: a run whose setup differs from the others, Streamline missing,
a build without the newest log lines, a route that failed.

The report also works on any logs, scripted or not:

```
python -I tools\perf\perf_report.py ObeliskChamber.log
python -I tools\perf\perf_report.py --a a1.log a2.log --b b1.log b2.log --names off on
```

Without scenes it uses the steady 10 s windows of the hitch monitor (all but the first, worst frame under 50 ms).
