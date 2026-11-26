// SPDX-License-Identifier: GPL-3.0-or-later
//! The launcher without its window, for a program that drives it, such as a Steam Deck plugin (launcher/README.md): --status,
//! --check-update, --update and --rebuild. They use the saved settings (launcher.json) as the window does. Every line on stdout is
//! one JSON object: any number with "event": "progress", then one with "event": "end" and "state": "done", "error" or "cancelled".
//! Exit code 0 for "done", else 1. A line "cancel" on stdin stops --update or --rebuild.

use crate::{cpu, lock, settings, setup, update, Config};
use serde_json::{json, Value};
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// Goes up when a command or a field a caller relies on changes.
pub const API: u32 = 1;

const GAME_RUNNING: &str = "The game is running. Close it first.";
const TAURI_CONF: &str = include_str!("../tauri.conf.json");

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Command {
    Status,
    CheckUpdate,
    Update,
    Rebuild,
}

pub fn command(args: &[String]) -> Option<Command> {
    args.iter().find_map(|a| match a.as_str() {
        "--status" => Some(Command::Status),
        "--check-update" => Some(Command::CheckUpdate),
        "--update" => Some(Command::Update),
        "--rebuild" => Some(Command::Rebuild),
        _ => None,
    })
}

/// Runs one command, prints its lines and returns the exit code.
pub fn run(cmd: Command) -> i32 {
    let result = saved_config().and_then(|cfg| match cmd {
        Command::Status => Ok(status(&cfg)),
        Command::CheckUpdate => check_update(),
        Command::Update => run_update(&cfg),
        Command::Rebuild => rebuild(&cfg),
    });
    let end = result.unwrap_or_else(|message| json!({ "state": "error", "message": message }));
    let code = if end["state"] == "done" { 0 } else { 1 };
    emit("end", end);
    code
}

/// One line: `fields` (an object) with the event and the interface version.
fn emit(event: &str, mut fields: Value) {
    fields["event"] = event.into();
    fields["api"] = API.into();
    crate::out_line(&fields.to_string());
}

fn conf(key: &str) -> String {
    serde_json::from_str::<Value>(TAURI_CONF).ok().and_then(|v| v[key].as_str().map(String::from)).unwrap_or_default()
}

/// The folder Tauri's app_config_dir() is under, found without a running app.
fn config_base(var: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(windows) {
        var("APPDATA")
    } else {
        var("XDG_CONFIG_HOME").filter(|p| p.is_absolute()).or_else(|| var("HOME").map(|h| h.join(".config")))
    }
}

/// launcher.json, the settings the window saved. None there yet: the defaults. One that cannot be read is an error, not the defaults.
fn saved_config() -> Result<Config, String> {
    let base = config_base(|k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)).ok_or("No settings folder: HOME is not set.")?;
    read_config(&base.join(conf("identifier")).join("launcher.json"))
}

fn read_config(path: &Path) -> Result<Config, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// Whether `a` and `b` are one folder, also through links.
fn same_dir(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    a == b || real(a) == real(b)
}

/// Whether the folder of a Windows path as Wine shows it ("Z:/a/b", "S:/b") is one of `dirs`. Z: is the whole file system. What
/// another letter stands for is the prefix's business, so there the rest of the path has to be the end of the folder's.
fn wine_dir_is(drive: char, rest: &str, dirs: &[PathBuf]) -> bool {
    if drive.eq_ignore_ascii_case(&'z') {
        return dirs.iter().any(|d| same_dir(Path::new(&format!("/{rest}")), d));
    }
    let parts = |s: &str| s.split(['/', '\\']).filter(|x| !x.is_empty()).map(str::to_lowercase).collect::<Vec<_>>();
    let tail = parts(rest);
    dirs.iter().any(|d| {
        let real = fs::canonicalize(d).unwrap_or_else(|_| d.clone());
        [d, &real].iter().any(|d| parts(&d.to_string_lossy()).ends_with(&tail))
    })
}

/// Whether a process's command line (/proc/<pid>/cmdline) names the fefw.exe of one of `dirs`: the game itself, or Wine or Proton
/// about to start it. `cwd`: the process's working folder, for a name without a full path; unknown, such a name counts.
fn names_game(cmdline: &[u8], cwd: Option<&Path>, dirs: &[PathBuf]) -> bool {
    cmdline.split(|&b| b == 0).any(|arg| {
        let arg = String::from_utf8_lossy(arg).replace('\\', "/");
        let (folder, name) = arg.rsplit_once('/').unwrap_or(("", &arg));
        if !name.eq_ignore_ascii_case("fefw.exe") {
            return false;
        }
        let mut chars = folder.chars();
        if let (Some(drive @ ('a'..='z' | 'A'..='Z')), Some(':')) = (chars.next(), chars.next()) {
            return wine_dir_is(drive, chars.as_str().trim_start_matches('/'), dirs);
        }
        let folder = if arg.starts_with('/') {
            Path::new("/").join(folder.trim_start_matches('/'))
        } else {
            match cwd {
                Some(c) => c.join(folder),
                None => return true,
            }
        };
        dirs.iter().any(|d| same_dir(&folder, d))
    })
}

/// Windows: a running program's file cannot be opened for writing (a sharing violation). Nothing is written.
fn in_use(exe: &Path) -> bool {
    const SHARING_VIOLATION: i32 = 32;
    cfg!(windows) && fs::OpenOptions::new().write(true).open(exe).is_err_and(|e| e.raw_os_error() == Some(SHARING_VIOLATION))
}

/// Whether the fefw.exe of one of `dirs` runs, however it was started (Steam, a window of the launcher). Another install's does not
/// count.
fn game_running(dirs: &[PathBuf]) -> bool {
    if cfg!(windows) {
        return dirs.iter().any(|d| in_use(&d.join("fefw.exe")));
    }
    if !cfg!(target_os = "linux") {
        return false;
    }
    let Ok(rd) = fs::read_dir("/proc") else { return false };
    rd.flatten().filter(|e| e.file_name().to_string_lossy().parse::<u32>().is_ok()).any(|e| {
        let cwd = fs::read_link(e.path().join("cwd")).ok();
        fs::read(e.path().join("cmdline")).is_ok_and(|c| names_game(&c, cwd.as_deref(), dirs))
    })
}

/// Where this install's fefw.exe and the files it runs with are: the launcher's folder and the run folder, and in a checkout the
/// build's.
fn install_dirs(cfg: &Config) -> Vec<PathBuf> {
    let mut dirs = vec![crate::exe_dir()];
    if let Some(repo) = crate::repo_of(cfg) {
        dirs.push(crate::game_dir_of(cfg, &repo));
        if crate::package_tools().is_none() {
            dirs.extend(setup::exe_path(&repo).parent().map(Path::to_path_buf));
        }
    }
    dirs
}

fn idle(dirs: &[PathBuf]) -> Result<(), String> {
    if game_running(dirs) {
        Err(GAME_RUNNING.into())
    } else {
        Ok(())
    }
}

/// Calls `stop` when the caller writes "cancel" on stdin. The end of stdin is not a cancel: a caller may give none.
fn on_cancel(stop: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(l) if l.trim() == "cancel" => stop(),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

fn status(cfg: &Config) -> Value {
    let install = crate::exe_dir();
    let package = crate::package_tools().is_some();
    let busy = if game_running(&install_dirs(cfg)) {
        Some("game")
    } else if lock::held(&install) {
        Some("job")
    } else {
        None
    };
    let game_dir = crate::repo_of(cfg).map(|r| crate::game_dir_of(cfg, &r));
    let paths = cpu::check().map_or_else(|| crate::paths(cfg), Err);
    let exe = match &paths {
        Ok(p) => Some(p.exe()),
        // without a usable dump the package's exe is still where it is built
        Err(_) => game_dir.as_ref().filter(|_| package).map(|d| d.join("fefw.exe")),
    };
    let (rebuild, reason) = match &game_dir {
        Some(d) if package => update::rebuild_needed(&install, d),
        _ => (None, String::new()),
    };
    let shown = |p: &Path| crate::dump::fwd(p);
    json!({
        "state": "done",
        "version": crate::current_version(conf("version")),
        "package": package,
        "install_dir": shown(&install),
        "game_dir": game_dir.as_deref().map(shown),
        "dump_dir": cfg.dump_dir,
        "busy": busy,
        "exe_exists": exe.is_some_and(|e| e.is_file()),
        "setup_complete": paths.as_ref().is_ok_and(setup::complete),
        "problem": paths.err(),
        "rebuild": rebuild,
        "rebuild_reason": reason,
    })
}

fn check_update() -> Result<Value, String> {
    let c = update::check(&crate::current_version(conf("version")), crate::package_tools().is_some())?;
    Ok(json!({ "state": "done", "current": c.current, "newer": c.newer, "package": c.package, "latest": c.latest }))
}

fn run_update(cfg: &Config) -> Result<Value, String> {
    if crate::package_tools().is_none() {
        return Err("Updates are for the release package. In a checkout, use git.".into());
    }
    let install = crate::exe_dir();
    let _lock = lock::acquire(&install)?;
    let dirs = install_dirs(cfg);
    idle(&dirs)?;
    let game_dir = crate::repo_of(cfg).map(|r| crate::game_dir_of(cfg, &r)).unwrap_or_else(|| install.clone());
    let check = update::check(&crate::current_version(conf("version")), true)?;
    let release = check.latest.filter(|_| check.newer);
    let Some(release) = release else {
        // an earlier update may still wait for its rebuild
        let (rebuild, reason) = update::rebuild_needed(&install, &game_dir);
        return Ok(json!({
            "state": "done",
            "updated": false,
            "version": check.current,
            "rebuild": rebuild,
            "rebuild_reason": reason,
            "message": "This is the latest version.",
        }));
    };
    if release.asset_url.is_empty() {
        return Err(format!("Release {} has no package for this system yet.", release.version));
    }
    let shared = Arc::new(update::Shared::default());
    let s = shared.clone();
    on_cancel(move || s.cancel.store(true, Ordering::SeqCst));
    let progress = |s: update::Status| {
        emit("progress", json!({ "phase": s.phase, "done": s.done, "total": s.total, "message": s.message }));
    };
    // the download takes a while: the game may have been started meanwhile
    let ready = || idle(&dirs);
    let job = update::Run { shared: &shared, install, game_dir, emit: &progress, ready: &ready };
    match job.run(&release) {
        Ok(s) => Ok(json!({
            "state": "done",
            "updated": true,
            "version": s.version,
            "rebuild": s.rebuild,
            "rebuild_reason": s.rebuild_reason,
            "message": format!("Version {} is installed.", release.version),
        })),
        Err(message) if message == update::CANCELLED => Ok(json!({ "state": "cancelled", "message": message })),
        Err(message) => Err(message),
    }
}

/// How far a whole setup is, from the progress of the step that runs.
pub struct Overall {
    base: f64,
    steps: f64,
    /// the step last seen and how many were
    seen: Mutex<(String, usize)>,
}

impl Overall {
    /// `base`: the share already behind (an unpacked .nup).
    pub fn new(p: &setup::Paths, base: f64) -> Overall {
        Overall::of(counted(&setup::steps(p, None)), base)
    }

    fn of(steps: usize, base: f64) -> Overall {
        Overall { base, steps: steps as f64, seen: Mutex::new((String::new(), 0)) }
    }

    /// 0..1. It goes back when a step starts its next part, which counts from the start again.
    pub fn at(&self, pr: &setup::Progress) -> f64 {
        let mut g = self.seen.lock().unwrap();
        if g.0 != pr.step {
            g.0 = pr.step.clone();
            g.1 += 1;
        }
        let within = if pr.total > 0 { (pr.done as f64 / pr.total as f64).min(1.0) } else { 0.0 };
        (self.base + (1.0 - self.base) * (g.1.saturating_sub(1) as f64 + within) / self.steps).clamp(0.0, 1.0)
    }
}

/// The steps a setup will report: those not done, and the two it runs every time (writing fefw.ini, and the shader preparation,
/// which checks the GPU and driver itself).
fn counted(steps: &[setup::StepState]) -> usize {
    steps.iter().filter(|s| s.state != "done" || matches!(s.id.as_str(), "ini" | "prepare")).count().max(1)
}

/// The window's Set up with the saved settings: after an update it builds what the new version changed.
fn rebuild(cfg: &Config) -> Result<Value, String> {
    if let Some(why) = cpu::check() {
        return Err(why);
    }
    let install = crate::exe_dir();
    let _lock = lock::acquire(&install)?;
    idle(&install_dirs(cfg))?;
    // as the window does when it opens
    if crate::package_tools().is_some() {
        update::cleanup(&install);
    }
    crate::refresh_package_locales();
    if let Err(e) = crate::sync_run_folder(cfg) {
        eprintln!("[run folder] {e}");
    }
    let p = crate::paths(cfg)?;
    let shared = Arc::new(setup::Shared::default());
    *shared.progress.lock().unwrap() =
        setup::Progress { state: "running".into(), step: "python".into(), message: "Starting...".into(), ..Default::default() };
    let s = shared.clone();
    on_cancel(move || {
        s.cancel.store(true, Ordering::SeqCst);
        setup::kill_tree(s.pid.load(Ordering::SeqCst));
    });
    let overall = Overall::new(&p, 0.0);
    let progress = |pr: setup::Progress| {
        let fraction = overall.at(&pr);
        emit("progress", json!({ "phase": pr.step, "done": pr.done, "total": pr.total, "fraction": fraction, "message": pr.message }));
    };
    let end = setup::run(&shared, &p, crate::jobs_of(cfg), &cfg.runner, &progress, &|_| {});
    if end.state == "cancelled" {
        return Ok(json!({ "state": "cancelled", "message": end.message }));
    }
    if end.state != "done" {
        return Err(if end.message.trim().is_empty() { "Setup did not finish.".into() } else { end.message });
    }
    if !setup::complete(&p) {
        return Err("Setup ended, but the game is not ready. See setup.log.".into());
    }
    settings::ensure_play_paths(&p.game_dir, &p)?;
    // a step skipped at the end (the shader preparation) reports nothing: finish the bar
    emit("progress", json!({ "phase": "done", "done": 1, "total": 1, "fraction": 1.0, "message": "Ready to play." }));
    Ok(json!({ "state": "done", "message": "Ready to play." }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn commands() {
        assert_eq!(command(&args(&["--status"])), Some(Command::Status));
        assert_eq!(command(&args(&["--check-update"])), Some(Command::CheckUpdate));
        assert_eq!(command(&args(&["--update"])), Some(Command::Update));
        assert_eq!(command(&args(&["--rebuild"])), Some(Command::Rebuild));
        // the window, and the setup RecompLauncher runs
        assert_eq!(command(&args(&[])), None);
        assert_eq!(command(&args(&["--game", "/dump"])), None);
        assert_eq!(command(&args(&["--prepare", "--game", "/dump", "--dlc="])), None);
    }

    #[test]
    fn overall_progress_follows_the_part_that_runs() {
        let o = Overall::of(2, 0.0);
        let at = |step: &str, done, total| o.at(&setup::Progress { step: step.into(), done, total, ..Default::default() });
        assert_eq!(at("build", 3, 6), 0.25);
        // the step's next part: not held at the furthest point of the one before
        assert_eq!(at("build", 0, 100), 0.0);
        assert_eq!(at("build", 100, 100), 0.5);
        assert_eq!(at("prepare", 0, 0), 0.5);
        assert_eq!(at("prepare", 10, 10), 1.0);
    }

    #[test]
    fn the_steps_that_always_run_are_counted() {
        let step = |id: &str, state: &str| setup::StepState { id: id.into(), state: state.into(), ..Default::default() };
        let done: Vec<_> = ["build", "shaders", "ini", "prepare"].iter().map(|id| step(id, "done")).collect();
        assert_eq!(counted(&done), 2);
        assert_eq!(counted(&[step("build", "todo"), step("shaders", "done"), step("ini", "done"), step("prepare", "done")]), 3);
        assert_eq!(counted(&[]), 1);
    }

    #[cfg(windows)]
    #[test]
    fn a_running_program_is_in_use() {
        assert!(in_use(&std::env::current_exe().unwrap()));
        assert!(!in_use(Path::new("no-such-folder/fefw.exe")));
    }

    #[cfg(not(windows))]
    #[test]
    fn the_game_is_found_however_it_was_started() {
        let dirs = [PathBuf::from("/home/p/games/fefw")];
        let runs = |cmdline: &str, cwd: Option<&str>| names_game(cmdline.as_bytes(), cwd.map(Path::new), &dirs);
        // Proton's script and its steam.exe, then the game under the drives Wine shows
        assert!(runs("python3\0/p/proton\0waitforexitandrun\0/home/p/games/fefw/fefw.exe\0", None));
        assert!(runs("c:\\windows\\system32\\steam.exe\0/home/p/games/fefw/fefw.exe\0", None));
        assert!(runs("Z:\\home\\p\\games\\fefw\\fefw.exe\0--prepare-shaders\0", None));
        assert!(runs("S:\\p\\games\\fefw\\FEFW.EXE\0", None));
        assert!(runs("G:\\fefw.exe\0", None));
        // no full path: the working folder says which install
        assert!(runs("umu-run\0./fefw.exe\0", Some("/home/p/games/fefw")));
        assert!(runs("wine\0fefw.exe\0", Some("/home/p/games/fefw")));
        assert!(runs("wine\0fefw.exe\0", None));
        // another install
        assert!(!runs("/home/p/games/other/fefw.exe\0", None));
        assert!(!runs("Z:\\home\\p\\games\\fefw-old\\fefw.exe\0", None));
        assert!(!runs("S:\\p\\other\\fefw\\fefw.exe\0", None));
        assert!(!runs("Z:\\games\\fefw\\fefw.exe\0", None)); // the end of the path, but Z: says where it starts
        assert!(!runs("wine\0./fefw.exe\0", Some("/home/p/games/other")));
        // not the game
        assert!(!runs("/home/p/games/fefw/fefw-launcher.AppImage\0--update\0", None));
        assert!(!runs("less\0/home/p/games/fefw/fefw.log\0", None));
        assert!(!runs("cmake\0--target\0not-fefw.exe\0", None));
        assert!(!runs("", None));
    }

    #[cfg(unix)]
    #[test]
    fn an_install_reached_through_a_link_is_the_same_one() {
        let d = std::env::temp_dir().join(format!("fefw-headless-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("real")).unwrap();
        std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
        let linked = format!("{}/fefw.exe", d.join("link").display());
        let dirs = [d.join("real")];
        assert!(names_game(linked.as_bytes(), None, &dirs));
        assert!(names_game(format!("Z:{}", linked.replace('/', "\\")).as_bytes(), None, &dirs));
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(not(windows))]
    #[test]
    fn settings_are_where_the_window_saves_them() {
        let env = |xdg: &'static str| {
            move |k: &str| match k {
                "XDG_CONFIG_HOME" => (!xdg.is_empty()).then(|| PathBuf::from(xdg)),
                "HOME" => Some(PathBuf::from("/home/p")),
                _ => None,
            }
        };
        assert_eq!(config_base(env("")), Some(PathBuf::from("/home/p/.config")));
        assert_eq!(config_base(env("/x")), Some(PathBuf::from("/x")));
        assert_eq!(config_base(env("relative")), Some(PathBuf::from("/home/p/.config")));
        assert_eq!(conf("identifier"), "io.github.goldoire.fefw-launcher");
        assert!(update::parse_version(&conf("version")).is_some());
    }

    #[test]
    fn a_damaged_settings_file_is_not_the_defaults() {
        let d = std::env::temp_dir().join(format!("fefw-headless-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let path = d.join("launcher.json");
        assert!(read_config(&path).is_ok_and(|c| c.dump_dir.is_empty()));
        fs::write(&path, r#"{"dump_dir": "/dump", "jobs": 3}"#).unwrap();
        assert!(read_config(&path).is_ok_and(|c| c.dump_dir == "/dump" && c.jobs == 3));
        fs::write(&path, "{").unwrap();
        assert!(read_config(&path).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
