# fefw recomp launcher

Tauri 2 launcher for this repository's `fefw.exe`. It ships no game files and never writes to your dump. It works on a checkout of
the repository: it finds the checkout from its own location (or Settings, Repository folder), checks your dump, runs the repository's
own setup steps on your PC (see docs/BUILDING.md), writes `fefw.ini` in a run folder (`work/play` by default) and starts the game.

- Build: `npm install`, then `npm run tauri build -- --no-bundle`; the exe is `src-tauri/target/release/fefw-launcher.exe`.
- Development: `npm run tauri dev`; `cargo test` in `src-tauri` runs the unit tests.
- Setup state is read from what exists on disk (the copied ExeFS, the lifted libraries, `fefw.exe`, the shader cache and its
  `work/shaders/setup.json`), so a paused setup continues where it stopped. The full log is `work/setup.log`.
- The icon is drawn by `icon/make_icon.py` (no outside artwork); `npx tauri icon icon/icon.png -o src-tauri/icons` makes the sizes.

## Without the window

For a program that drives the launcher, such as a Steam Deck plugin (`src-tauri/src/headless.rs`). No display is needed. They act on
the install the launcher is in, with the settings the window saved (`launcher.json`: the dump, the run folder, the runner).

| Command | Does | Fields of its last line, besides `state` |
| --- | --- | --- |
| `--status` | Reports the install. Changes nothing. | `version`, `package`, `install_dir`, `game_dir`, `dump_dir`, `busy` (`"game"`, `"job"` or null), `exe_exists`, `setup_complete`, `problem` (why setup cannot run, else null), `rebuild` (null when it cannot tell), `rebuild_reason` |
| `--check-update` | Asks GitHub for the latest release. | `current`, `newer`, `package`, `latest` |
| `--update` | Downloads, checks and installs a newer release (release package only). | `updated`, `version`, `rebuild`, `rebuild_reason`, `message` |
| `--rebuild` | The Set up button: builds whatever is not done, such as the game after an update. | `message` |

- Every line on stdout is a JSON object with `event` and `api` (this interface's version, 1). `--update` and `--rebuild` print
  `"event": "progress"` lines (`phase`, `done`, `total`, `message`; `--rebuild` also `fraction`, 0 to 1). Every command ends with one
  `"event": "end"` line: `state` is `done`, `error` or `cancelled`, with a `message` for the last two. The exit code is 0 for `done`,
  else 1. Messages are in English.
- Write `cancel` and a newline to stdin to stop `--update` (until it starts installing) or `--rebuild`. Ending the process instead
  can leave a rebuild's compilers running.
- `--update` and `--rebuild` refuse while this install's `fefw.exe` runs (on Windows the system refuses to replace it), and while
  another setup or update, in the window or not, works on the install (`launcher.lock`).
- On Linux the last step of `--rebuild`, preparing the shaders, runs `fefw.exe`, which needs a display. With neither `DISPLAY` nor
  `WAYLAND_DISPLAY` set (and no runner of the player's own) it is skipped, and the game compiles shaders while it is played instead. Pass
  the session's display to have it done. A display that is set but does not work can still leave the step waiting.
- After `--update`, run the new launcher for `--rebuild`: on Linux `fefw-launcher.AppImage` in the install folder, also when the old
  one was started from an unpacked `AppRun`.
- A launcher from before these commands opens its window instead, so a caller has to know the installed version has them.

`--prepare --game <dump or .nup> [--dlc <folder>]` is RecompLauncher's setup (`recomplauncher.json`). It prints
`progress <0..1> <text>` lines, not JSON, and does not save the dump.

