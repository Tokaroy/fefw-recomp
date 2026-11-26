// SPDX-License-Identifier: GPL-3.0-or-later
//! The game's Steam library art (steam-art/, drawn from the project's own icon by icon/make_steam_art.py), put beside the shortcut
//! Add to Steam makes: Steam reads a shortcut's pictures from the account's config/grid, by the shortcut's id.

use std::fs;
use std::path::{Path, PathBuf};

/// What follows the id in the file's name, and the picture.
const ART: [(&str, &[u8]); 5] = [
    ("p", include_bytes!("../steam-art/portrait.png")),
    ("", include_bytes!("../steam-art/wide.png")),
    ("_hero", include_bytes!("../steam-art/hero.png")),
    ("_logo", include_bytes!("../steam-art/logo.png")),
    ("_icon", include_bytes!("../steam-art/icon.png")),
];
/// The picture formats a player's own art may have.
const FORMATS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "ico"];

/// Gives the shortcut `id` of the account whose config folder this is each picture it has none of. A picture that is there, in any
/// format, stays: it may be the player's choice. Returns the icon, when it is the launcher's, for the shortcut to name. The art is
/// a nicety: a file that cannot be written is left out without an error.
pub fn apply(config: &Path, id: u32) -> Option<PathBuf> {
    let grid = config.join("grid");
    fs::create_dir_all(&grid).ok()?;
    let mut icon = None;
    for (suffix, picture) in ART {
        let stem = format!("{id}{suffix}");
        let ours = grid.join(format!("{stem}.png"));
        let there = FORMATS.iter().any(|f| grid.join(format!("{stem}.{f}")).exists());
        let written = !there && fs::write(&ours, picture).is_ok();
        if suffix == "_icon" && (written || fs::read(&ours).is_ok_and(|d| d == picture)) {
            icon = Some(ours);
        }
    }
    icon
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fefw-art-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_new_shortcut_gets_every_picture() {
        let d = tmp("new");
        let icon = apply(&d, 2_242_939_390).unwrap();
        assert_eq!(icon, d.join("grid").join("2242939390_icon.png"));
        for name in ["2242939390p.png", "2242939390.png", "2242939390_hero.png", "2242939390_logo.png", "2242939390_icon.png"] {
            assert!(fs::read(d.join("grid").join(name)).unwrap().starts_with(b"\x89PNG"), "{name}");
        }
        // again: the same files, and the icon is still known as the launcher's
        assert_eq!(apply(&d, 2_242_939_390), Some(icon));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_players_pictures_stay() {
        let d = tmp("own");
        let grid = d.join("grid");
        fs::create_dir_all(&grid).unwrap();
        fs::write(grid.join("7p.jpg"), "mine").unwrap();
        fs::write(grid.join("7_hero.png"), "mine").unwrap();
        fs::write(grid.join("7_icon.ico"), "mine").unwrap();
        fs::write(grid.join("7.json"), "logo position").unwrap();
        assert_eq!(apply(&d, 7), None);
        assert!(!grid.join("7p.png").exists() && !grid.join("7_icon.png").exists());
        assert_eq!(fs::read_to_string(grid.join("7_hero.png")).unwrap(), "mine");
        assert!(grid.join("7.png").is_file() && grid.join("7_logo.png").is_file(), "a .json is not a picture");
        let _ = fs::remove_dir_all(&d);
    }
}
