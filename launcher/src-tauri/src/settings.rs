// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads and edits the runtime's ini files in the run folder: fefw.ini (every key: runtime/docs/CONFIG.md) and controls.ini (the
//! play-mode file the game writes on its first start). The key schema lives in the frontend; this side moves `section.key` values in
//! and out with the careful line editor in ini.rs, so comments and unknown keys survive.

use crate::ini::Ini;
use crate::setup::Paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const GAME_INI: &str = "fefw.ini";
pub const CONTROLS_INI: &str = "controls.ini";

#[derive(Serialize)]
pub struct SettingsDump {
    pub game_dir: String,
    pub game_exists: bool,
    pub controls_exists: bool,
    pub game: HashMap<String, String>,
    pub controls: HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct Edit {
    /// "game" (fefw.ini) or "controls" (controls.ini)
    pub file: String,
    pub section: String,
    pub key: String,
    pub value: String,
}

fn load(path: &Path) -> Option<Ini> {
    let bytes = fs::read(path).ok()?;
    Some(Ini::parse(&String::from_utf8_lossy(&bytes)))
}

fn map_of(ini: &Option<Ini>) -> HashMap<String, String> {
    ini.as_ref().map(|i| i.all().into_iter().collect()).unwrap_or_default()
}

fn file_for(dir: &Path, which: &str) -> Result<PathBuf, String> {
    match which {
        "game" => Ok(dir.join(GAME_INI)),
        "controls" => Ok(dir.join(CONTROLS_INI)),
        other => Err(format!("unknown settings file '{other}'")),
    }
}

pub fn read(dir: &Path) -> SettingsDump {
    let game = load(&dir.join(GAME_INI));
    let controls = load(&dir.join(CONTROLS_INI));
    SettingsDump {
        game_dir: crate::dump::fwd(dir),
        game_exists: game.is_some(),
        controls_exists: controls.is_some(),
        game: map_of(&game),
        controls: map_of(&controls),
    }
}

pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {}", d.display(), e))?;
    }
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("cannot replace {}: {}", path.display(), e)
    })
}

/// Applies edits grouped per file. controls.ini is never created here: the game writes it with its factory bindings on first run.
pub fn apply(dir: &Path, edits: &[Edit]) -> Result<(), String> {
    for which in ["game", "controls"] {
        let mine: Vec<&Edit> = edits.iter().filter(|e| e.file == which).collect();
        if mine.is_empty() {
            continue;
        }
        let path = file_for(dir, which)?;
        let mut ini = match load(&path) {
            Some(i) => i,
            None if which == "game" => Ini::empty(),
            None => return Err(format!("{CONTROLS_INI} does not exist yet. Start the game once so it creates it.")),
        };
        for e in mine {
            ini.set(&e.section, &e.key, &e.value);
        }
        write_atomic(&path, &ini.to_text())?;
    }
    Ok(())
}

/// The ini values setup owns: where the game code, the romfs, the dumped add-on content, the mods, the shader cache and the texture
/// cache are. With no dlc/ in the dump, [aoc] dir is empty: no add-on content.
fn owned(p: &Paths) -> Vec<(&'static str, &'static str, String)> {
    vec![
        ("fs", "exefs", crate::dump::game_path(&p.plain_exefs())),
        ("fs", "main", String::new()),
        ("fs", "romfs", crate::dump::game_path(&p.romfs)),
        ("fs", "mods", crate::dump::game_path(&p.mods())),
        ("aoc", "dir", p.dlc.as_deref().map(crate::dump::game_path).unwrap_or_default()),
        ("nvn", "shader_cache", crate::dump::game_path(&p.spirv())),
        ("nvn", "bc7_cache", crate::dump::game_path(&p.bc7())),
    ]
}

pub fn play_paths_ok(dir: &Path, p: &Paths) -> bool {
    let Some(ini) = load(&dir.join(GAME_INI)) else { return false };
    owned(p).iter().all(|(s, k, v)| ini.get(s, k).unwrap_or_default().replace('\\', "/") == *v)
}

/// The shipped language tables (`locales\*.ini`), from the package next to the launcher or from the checkout.
fn locale_sources(p: &Paths) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(tools) = &p.package {
        if let Some(root) = tools.parent() {
            out.push(root.join("locales"));
        }
    }
    out.push(p.repo.join("locales"));
    out
}

/// Copies the language tables the package ships into the run folder, where fefw_rt.dll reads them (`locales\<lang>.ini` next to
/// the DLL). A table already there is left alone: it may be the player's own translation of that language. Only `.ini` files are
/// copied, and only the ones the run folder does not have yet.
pub fn install_locales(dir: &Path, p: &Paths) {
    let target = dir.join("locales");
    for src in locale_sources(p) {
        let Ok(rd) = fs::read_dir(&src) else { continue };
        for e in rd.flatten() {
            let from = e.path();
            let is_ini = from.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("ini"));
            if !is_ini || !from.is_file() {
                continue;
            }
            let Some(name) = from.file_name() else { continue };
            let to = target.join(name);
            if to.exists() {
                continue; // the player's copy, or one this already put there
            }
            let _ = fs::create_dir_all(&target);
            if let Err(e) = fs::copy(&from, &to) {
                eprintln!("[locales] could not copy {} to {}: {e}", from.display(), to.display());
            }
        }
    }
}

/// Writes fefw.ini's paths (kept: every other key, and a save folder the player chose). Used by setup and before every Play.
/// The first time, it also picks an output size for the screen; the answer is then a line for the setup log.
pub fn ensure_play_paths(dir: &Path, p: &Paths) -> Result<Option<String>, String> {
    ensure_play_paths_on(dir, p, crate::screen::size)
}

fn ensure_play_paths_on(dir: &Path, p: &Paths, screen: impl FnOnce() -> Option<(u32, u32)>) -> Result<Option<String>, String> {
    let path = dir.join(GAME_INI);
    let new = !path.exists();
    let mut ini = load(&path).unwrap_or_else(|| {
        Ini::parse(
            "; fefw.ini: settings of fefw.exe. Every key is described in runtime/docs/CONFIG.md. The launcher writes [fs], [aoc] and [nvn] paths.\n",
        )
    });
    let mut changed = new;
    // no [fs] paths yet: Set up has not written here before (Settings saved before it leaves a file with only what was changed)
    let first = ini.get("fs", "exefs").is_none();
    let mut note = None;
    let _ = fs::create_dir_all(p.mods());
    install_locales(dir, p);
    for (s, k, v) in owned(p) {
        if ini.get(s, k).unwrap_or_default().replace('\\', "/") != v {
            ini.set(s, k, &v);
            changed = true;
        }
    }
    if ini.get("fs", "save_dir").unwrap_or_default().trim().is_empty() {
        let _ = fs::create_dir_all(dir.join("save"));
        ini.set("fs", "save_dir", "save"); // relative to the run folder
        changed = true;
    }
    if new {
        // a first ini: the log next to it, the built-in temporal upscaler, the game's own frame rate
        ini.set("log", "path", "fefw.log");
        ini.set("log", "level", "info");
        ini.set("boot", "gfx", "window");
        ini.set("dlss", "upscaler", "taau");
        ini.set("fefw", "fps60", "0");
    }
    if first && ini.get("fefw", "output").is_none() {
        // once only, and never over a choice: a screen smaller than the console's 2560x1440 starts at its own class of size
        let screen = screen();
        let output = crate::screen::output_for(screen);
        ini.set("fefw", "output", output);
        changed = true;
        if let Some((w, h)) = screen.filter(|_| output != "console") {
            note = Some(format!("Output resolution set to {output} for the {w}x{h} screen (Settings changes it)"));
        }
    }
    if changed {
        write_atomic(&path, &ini.to_text())?;
    }
    Ok(note)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fefw-loc-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn paths_with(repo: &Path, game: &Path) -> Paths {
        Paths {
            repo: repo.to_path_buf(),
            exefs: repo.join("exefs"),
            romfs: repo.join("romfs"),
            dlc: None,
            game_dir: game.to_path_buf(),
            package: None,
        }
    }

    fn output_of(game: &Path) -> Option<String> {
        load(&game.join(GAME_INI)).unwrap().get("fefw", "output")
    }

    #[test]
    fn the_first_set_up_fits_the_output_to_a_small_screen_once() {
        let repo = tmp("repo-output");
        let game = tmp("game-output");
        let p = paths_with(&repo, &game);

        let note = ensure_play_paths_on(&game, &p, || Some((800, 1280))).unwrap();
        assert_eq!(output_of(&game).as_deref(), Some("720p"));
        assert!(note.unwrap().contains("720p"));

        // the player's later choice stays, and the screen is not asked again
        apply(&game, &[Edit { file: "game".into(), section: "fefw".into(), key: "output".into(), value: "console".into() }]).unwrap();
        assert_eq!(ensure_play_paths_on(&game, &p, || panic!("asked again")).unwrap(), None);
        assert_eq!(output_of(&game).as_deref(), Some("console"));
    }

    #[test]
    fn settings_saved_before_set_up_do_not_stop_the_first_defaults() {
        let repo = tmp("repo-early");
        let game = tmp("game-early");
        let p = paths_with(&repo, &game);
        fs::write(game.join(GAME_INI), "[gfx]\nui_language = zh\n").unwrap();
        ensure_play_paths_on(&game, &p, || Some((1920, 1080))).unwrap();
        assert_eq!(output_of(&game).as_deref(), Some("1080p"));

        // an output picked there is the player's
        fs::write(game.join(GAME_INI), "[fefw]\noutput = 1440p\n").unwrap();
        assert_eq!(ensure_play_paths_on(&game, &p, || panic!("asked")).unwrap(), None);
        assert_eq!(output_of(&game).as_deref(), Some("1440p"));
    }

    #[test]
    fn an_ini_set_up_wrote_before_keeps_its_output() {
        let repo = tmp("repo-old");
        let game = tmp("game-old");
        let p = paths_with(&repo, &game);
        fs::write(game.join(GAME_INI), "[fs]\nexefs = old\n").unwrap();
        assert_eq!(ensure_play_paths_on(&game, &p, || panic!("asked")).unwrap(), None);
        assert_eq!(output_of(&game), None);
    }

    #[test]
    fn install_locales_copies_the_shipped_tables_only_once() {
        let repo = tmp("repo");
        let game = tmp("game");
        fs::create_dir_all(repo.join("locales")).unwrap();
        fs::write(repo.join("locales").join("zh.ini"), "shipped zh").unwrap();
        fs::write(repo.join("locales").join("README.md"), "docs").unwrap();
        let p = paths_with(&repo, &game);

        install_locales(&game, &p);
        assert_eq!(fs::read_to_string(game.join("locales").join("zh.ini")).unwrap(), "shipped zh");
        assert!(!game.join("locales").join("README.md").exists(), "only the tables are copied");

        // the player's own copy is never replaced by a later run
        fs::write(game.join("locales").join("zh.ini"), "the player's own").unwrap();
        install_locales(&game, &p);
        assert_eq!(fs::read_to_string(game.join("locales").join("zh.ini")).unwrap(), "the player's own");
    }
}
