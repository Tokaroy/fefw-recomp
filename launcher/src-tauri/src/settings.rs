// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads and edits the runtime's ini files in the run folder: fefw.ini (every key: runtime/docs/CONFIG.md) and controls.ini (the
//! play-mode file the game writes on its first start). The key schema lives in the frontend; this side moves `section.key` values in
//! and out with the careful line editor in ini.rs, so comments and unknown keys survive.

use crate::ini::Ini;
use crate::setup::Paths;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

/// The Chinese table this launcher was built with. It travels inside the executable because an update leaves the table already in
/// the install folder where it is and discards the new one (update.rs).
const SHIPPED_ZH: &str = include_str!("../../../locales/zh.ini");

/// [`table_hash`] of every zh.ini committed before the launcher began recording the one it installed: a file that is one of these was
/// never edited by the player. A closed list; later versions are known from the record.
const EARLIER_ZH: [&str; 25] = [
    "05d8400e675b5c165e390f2c2461c981b84d27d9622594061631ef2b22462b2c",
    "0c21218c7d34426cd575f68595113c13a62c96fb08cbec648f0fdbdff5aaa9f5",
    "13ed1b2ec010264e0b86f91b80e5cd7945a777d40e822d18f6eacc9b433b027a",
    "382eea8637ca1f1065ceb7c2c6a7d894c1540fece55f91336555eb9f6e72510a",
    "3c5b521ba1c3d49a4015c91ef6af226ceaca2241d7ee5d4526198fa9beeaf5b1",
    "417e141c4607b061c32e7943eb916ed387063d4d77993d8d5ab503d6741bae71",
    "42e2126a20750c2e4e40c78bdca7c0fdfcaeed7085d91387e86a5392c3323430",
    "4760023a8b90173258bb4db747758c9576cae9ff7fa1df4cc0623915be6c3ce2",
    "5217c7642e622fb183f4b5297a1350139d0ee89b35f92ef39b88731985c9813f",
    "6259c8a813e30137c6bb2b98a1e93bb335461e14a8d8b228082e51638af8cee9",
    "6697865ad10386a6eea033fbee19edf6efc9c2fa81b1af2bee02fde24bf46245",
    "6c82ec84148083b5ac8cb31f9729ef709553d75f0dcb01cb254a7bfb1d11e230",
    "760bc22ca18ca8ea0132bb93371cdc7baad4d9c32a540b6c531f7378193b645c",
    "763c3cb047102ea7dcf23dd994e7086f1ed2e835572abdae8778778724581bb8",
    "8475147655df8bd251bc83b99d9dae8f1e1ba8cb01df70a5dff490299e523bf8",
    "8a1bf682156c7099037e324335de9dc7cc9d9c60bde62e7e59e8aed241a24038",
    "8ff28b9c63256722d6a5d8a2ced4aacdc6941eebf764e1ce9b1a8d44791add9d",
    "99eae0586b9fd4415493ea84c99d851122db10ae6333a38af475eff0c144e68a",
    "9ac95979289803e8ae24278b9e0477444a29db8348558632505182d92743f53b",
    "9f6c4053bece811aab740f41df4e0e0f41db3f804e616d82f232e8118af8e3a5",
    "a37eddf2bdbab1de39231e37170a212b89271e46c4a9a3407b01bdd48e9d9ce8",
    "adda65f82f7ddc3b2808b25e7a8b7e215ab11baf7121c4b4b89627fdc8630f45",
    "e05d9494d74bc1fd5051d7431b8fd5d9793b1dba1b4351f5155fa774192e28e6",
    "eedd8f0e5988380bb2bcd85fe750cebfb9575f515432f42c9642135c726a55b7",
    "f53b201db631836c79d613ad7d9511b28f1073f4e4a625e2a4ae21701a6811dd",
];

/// SHA-256 of a table, whatever its line endings and with or without a byte-order mark.
fn table_hash(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).replace("\r\n", "\n");
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Brings the run folder's zh.ini up to this launcher's, unless the player edited it. `zh.ini.shipped` beside it records the hash of
/// the table last put or seen there, which is how a later launcher tells an untouched file from an edited one.
fn refresh_zh(tables: &Path) {
    let path = tables.join("zh.ini");
    let record = tables.join("zh.ini.shipped");
    let Ok(bytes) = fs::read(&path) else { return };
    let (have, new) = (table_hash(&bytes), table_hash(SHIPPED_ZH.as_bytes()));
    let recorded = fs::read_to_string(&record).unwrap_or_default();
    if have != new {
        if recorded.trim() != have && !EARLIER_ZH.contains(&have.as_str()) {
            return; // the player's own
        }
        if let Err(e) = write_atomic(&path, SHIPPED_ZH) {
            eprintln!("[locales] {e}");
            return;
        }
    }
    if recorded.trim() != new {
        if let Err(e) = write_atomic(&record, &new) {
            eprintln!("[locales] {e}");
        }
    }
}

/// The shipped language tables (`locales\*.ini`), from the package next to the launcher or from the checkout.
fn locale_sources(repo: &Path, package: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(root) = package.and_then(Path::parent) {
        out.push(root.join("locales"));
    }
    out.push(repo.join("locales"));
    out
}

/// Copies the language tables the package ships into the run folder, where fefw_rt.dll reads them (`locales\<lang>.ini` next to
/// the DLL). Only `.ini` files are copied, and only the ones the run folder does not have yet: a table already there may be the
/// player's own. The one exception is a zh.ini the player never edited, which [`refresh_zh`] keeps current.
pub fn install_locales(dir: &Path, repo: &Path, package: Option<&Path>) {
    let target = dir.join("locales");
    for src in locale_sources(repo, package) {
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
    refresh_zh(&target);
}

/// Brings the F1 menu's [gfx] ui_language in step with the launcher's picked language (lang.rs). True when `ini` changed.
fn sync_menu_language(dir: &Path, ini: &mut Ini, lang: &str) -> bool {
    let current = ini.get("gfx", "ui_language").unwrap_or_default();
    let has_table = |code: &str| dir.join("locales").join(format!("{code}.ini")).is_file();
    match crate::lang::f1_value(lang, &current, &crate::lang::available(), has_table) {
        Some(v) => {
            ini.set("gfx", "ui_language", &v);
            true
        }
        None => false,
    }
}

/// The same when the launcher opens and on a switch, tables first: the game may be started without Play. A fefw.ini that is not
/// there yet stays so: Set up writes it, with its first defaults.
pub fn apply_menu_language(dir: &Path, repo: &Path, package: Option<&Path>, lang: &str) -> Result<(), String> {
    let path = dir.join(GAME_INI);
    let Some(mut ini) = load(&path) else { return Ok(()) };
    install_locales(dir, repo, package);
    if sync_menu_language(dir, &mut ini, lang) {
        write_atomic(&path, &ini.to_text())?;
    }
    Ok(())
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
    install_locales(dir, &p.repo, p.package.as_deref());
    changed |= sync_menu_language(dir, &mut ini, &p.lang);
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
            lang: String::new(),
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
    fn play_brings_the_menu_language_in_step() {
        let repo = tmp("repo-lang");
        let game = tmp("game-lang");
        let mut p = paths_with(&repo, &game);
        p.lang = "zh".into();
        let menu = |game: &Path| load(&game.join(GAME_INI)).unwrap().get("gfx", "ui_language");

        // a switch before Set up does not create the file
        apply_menu_language(&game, &repo, None, "zh").unwrap();
        assert!(!game.join(GAME_INI).exists());

        // an ini that lost the key, or never had it
        fs::write(game.join(GAME_INI), "[fs]\nexefs = old\n").unwrap();
        ensure_play_paths_on(&game, &p, || panic!("asked")).unwrap();
        assert_eq!(menu(&game).as_deref(), Some("zh"));

        apply_menu_language(&game, &repo, None, "en").unwrap();
        assert_eq!(menu(&game).as_deref(), Some("en"));

        // a table the player named stays
        fs::write(game.join(GAME_INI), "[gfx]\nui_language = mine\n").unwrap();
        ensure_play_paths_on(&game, &p, || Some((1920, 1080))).unwrap();
        assert_eq!(menu(&game).as_deref(), Some("mine"));
    }

    #[test]
    fn a_switch_brings_the_tables_to_a_run_folder_of_its_own() {
        let repo = tmp("repo-switch");
        let game = tmp("game-switch");
        fs::create_dir_all(repo.join("locales")).unwrap();
        fs::write(repo.join("locales").join("de.ini"), "Close = Schliessen\n").unwrap();
        fs::write(game.join(GAME_INI), "[gfx]\nui_language = auto\n").unwrap();

        apply_menu_language(&game, &repo, None, "de").unwrap();

        assert!(game.join("locales").join("de.ini").is_file());
        assert_eq!(load(&game.join(GAME_INI)).unwrap().get("gfx", "ui_language").as_deref(), Some("de"));
    }

    #[test]
    fn an_untouched_zh_table_is_brought_up_to_date_and_an_edited_one_is_not() {
        let game = tmp("game-zh");
        let table = game.join("zh.ini");
        let record = game.join("zh.ini.shipped");
        let read = || fs::read_to_string(&table).unwrap();

        // the table an earlier launcher recorded, since untouched (Windows line endings or not)
        fs::write(&table, "Close = old\r\n").unwrap();
        fs::write(&record, table_hash(b"Close = old\n")).unwrap();
        refresh_zh(&game);
        assert_eq!(read(), SHIPPED_ZH);
        assert_eq!(fs::read_to_string(&record).unwrap(), table_hash(SHIPPED_ZH.as_bytes()));

        // the player's edit stays, now and on the next start
        fs::write(&table, "Close = mine\n").unwrap();
        refresh_zh(&game);
        refresh_zh(&game);
        assert_eq!(read(), "Close = mine\n");

        // no table: nothing is made here (install_locales copies one first)
        fs::remove_file(&table).unwrap();
        refresh_zh(&game);
        assert!(!table.exists());
    }

    #[test]
    fn install_locales_copies_the_shipped_tables_only_once() {
        let repo = tmp("repo");
        let game = tmp("game");
        fs::create_dir_all(repo.join("locales")).unwrap();
        fs::write(repo.join("locales").join("zh.ini"), "shipped zh").unwrap();
        fs::write(repo.join("locales").join("README.md"), "docs").unwrap();
        let p = paths_with(&repo, &game);

        install_locales(&game, &p.repo, None);
        assert_eq!(fs::read_to_string(game.join("locales").join("zh.ini")).unwrap(), "shipped zh");
        assert!(!game.join("locales").join("README.md").exists(), "only the tables are copied");

        // the player's own copy is never replaced by a later run
        fs::write(game.join("locales").join("zh.ini"), "the player's own").unwrap();
        install_locales(&game, &p.repo, None);
        assert_eq!(fs::read_to_string(game.join("locales").join("zh.ini")).unwrap(), "the player's own");
    }
}
