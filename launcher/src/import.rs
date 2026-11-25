//! An already installed game, reused instead of installed again.
//!
//! A new release of the port (a new Windows folder, an AppImage after the data folder moved) finds
//! no game/ beside it. The player already has one: the folder an earlier version installed. This
//! finds it -- in the folders around the launcher, or where the player points -- checks its EBOOT
//! as Play does, and brings it in:
//!   - game/: moved into this port's folder (a rename: instant on the same drive). On another
//!     drive it stays where it is and this port reads it there (<data>/game_location.txt), so
//!     21 GB are never copied.
//!   - the earlier port's saves (work/gamedata), kenzan.ini, input.toml and mods/: copied, never
//!     over something this port already has.

use std::fs;
use std::path::{Path, PathBuf};

/// The game folder a port reads: <data>/game, or the folder game_location.txt names.
pub fn game_dir(data: &Path) -> PathBuf {
    if let Ok(t) = fs::read_to_string(data.join("game_location.txt")) {
        let p = PathBuf::from(t.trim());
        if p.join("PS3_GAME").is_dir() {
            return p;
        }
    }
    data.join("game")
}

/// What `sel` is: a port's folder (with game/), a game folder (with PS3_GAME) or PS3_GAME itself.
/// Returns (the port folder if any, the game folder).
pub fn locate(sel: &Path) -> Option<(Option<PathBuf>, PathBuf)> {
    let eboot = |g: &Path| g.join("PS3_GAME/USRDIR/EBOOT.elf").is_file();
    if eboot(&sel.join("game")) {
        return Some((Some(sel.to_path_buf()), sel.join("game")));
    }
    if let Ok(t) = fs::read_to_string(sel.join("game_location.txt")) {
        let g = PathBuf::from(t.trim());
        if eboot(&g) {
            return Some((Some(sel.to_path_buf()), g));
        }
    }
    if eboot(sel) {
        let port = sel.parent().filter(|p| p.join("work").is_dir() || p.join("kenzan.ini").is_file()).map(|p| p.to_path_buf());
        return Some((port, sel.to_path_buf()));
    }
    if sel.file_name().map(|n| n.eq_ignore_ascii_case("PS3_GAME")).unwrap_or(false) {
        if let Some(g) = sel.parent() {
            return locate(g);
        }
    }
    None
}

/// Installed games in the folders around `data` (its parent and the parent's subfolders, two
/// levels), other than `data`'s own.
pub fn find_near(data: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let Some(parent) = data.parent() else { return out };
    let mut look = |dir: &Path| {
        if dir == data {
            return;
        }
        if let Some((port, g)) = locate(dir) {
            let p = port.unwrap_or(g);
            if !out.contains(&p) {
                out.push(p);
            }
        }
    };
    if let Ok(rd) = fs::read_dir(parent) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            look(&p);
            if let Ok(rd2) = fs::read_dir(&p) {
                for e2 in rd2.flatten().take(64) {
                    if e2.path().is_dir() {
                        look(&e2.path());
                    }
                }
            }
        }
    }
    out
}

fn copy_tree_missing(src: &Path, dst: &Path, n: &mut usize) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)?.flatten() {
        let (s, d) = (e.path(), dst.join(e.file_name()));
        if s.is_dir() {
            copy_tree_missing(&s, &d, n)?;
        } else if !d.exists() {
            fs::copy(&s, &d)?;
            *n += 1;
        }
    }
    Ok(())
}

/// Bring the game at `sel` into the port at `data`. Ok(what was done) or Err(why not).
pub fn import(sel: &Path, data: &Path) -> Result<String, String> {
    let (port, game) = locate(sel).ok_or("No installed game there: pick the folder of an earlier version of the port, or its game folder.")?;
    let st = crate::checks::game_at(&game);
    if !st.ok {
        return Err(format!("That game cannot be used: {}", st.text));
    }
    let here = data.join("game");
    if fs::canonicalize(&game).ok() == fs::canonicalize(&here).ok() {
        return Err("That is already this port's game.".into());
    }
    if here.join("PS3_GAME").exists() {
        return Err("This port already has a game installed.".into());
    }
    let mut done = vec![];
    // the saves and settings first: a game moved away must not leave them behind
    if let Some(port) = &port {
        let mut n = 0;
        let src = port.join("work/gamedata");
        if src.is_dir() {
            copy_tree_missing(&src, &data.join("work/gamedata"), &mut n).map_err(|e| format!("Copying the saves: {e}"))?;
            done.push(format!("saves ({n} file(s))"));
        }
        for f in ["kenzan.ini", "input.toml"] {
            if port.join(f).is_file() && !data.join(f).exists() {
                fs::copy(port.join(f), data.join(f)).map_err(|e| e.to_string())?;
                done.push(f.to_string());
            }
        }
        if port.join("mods").is_dir() {
            let mut m = 0;
            let _ = copy_tree_missing(&port.join("mods"), &data.join("mods"), &mut m);
            if m > 0 {
                done.push(format!("mods ({m} file(s))"));
            }
        }
    }
    let _ = fs::remove_dir_all(&here); // an empty or broken leftover (checked above: no PS3_GAME)
    let _ = fs::remove_file(data.join("game_location.txt"));
    match fs::rename(&game, &here) {
        Ok(()) => {
            if let Some(port) = &port {
                // the earlier port reads its game from here now, if it is ever started again
                let _ = fs::write(port.join("game_location.txt"), here.display().to_string());
            }
            done.insert(0, "game moved here".into());
        }
        Err(_) => {
            fs::write(data.join("game_location.txt"), game.display().to_string()).map_err(|e| e.to_string())?;
            done.insert(0, format!("game used where it is ({})", game.display()));
        }
    }
    Ok(format!("Done: {}.", done.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// KZ_TEST_EBOOT=<a real EBOOT.elf>: an old port folder beside a new one, imported.
    #[test]
    fn import_from_old_port() {
        let Ok(eb) = std::env::var("KZ_TEST_EBOOT") else { return };
        let t = std::env::temp_dir().join(format!("kz-import-{}", std::process::id()));
        let (old, new) = (t.join("Kenzan-old"), t.join("Kenzan-new"));
        fs::create_dir_all(old.join("game/PS3_GAME/USRDIR")).unwrap();
        fs::copy(&eb, old.join("game/PS3_GAME/USRDIR/EBOOT.elf")).unwrap();
        fs::create_dir_all(old.join("work/gamedata/dev_hdd0/home/00000001/savedata/S1")).unwrap();
        fs::write(old.join("work/gamedata/dev_hdd0/home/00000001/savedata/S1/DATA.BIN"), b"save").unwrap();
        fs::write(old.join("kenzan.ini"), "fps=120\n").unwrap();
        fs::create_dir_all(new.join("work")).unwrap();
        assert_eq!(find_near(&new), vec![old.clone()]);
        let m = import(&old, &new).unwrap();
        println!("{m}");
        assert!(new.join("game/PS3_GAME/USRDIR/EBOOT.elf").is_file());
        assert!(!old.join("game").exists());
        assert!(new.join("work/gamedata/dev_hdd0/home/00000001/savedata/S1/DATA.BIN").is_file());
        assert_eq!(fs::read_to_string(new.join("kenzan.ini")).unwrap(), "fps=120\n");
        assert_eq!(game_dir(&old), new.join("game"));
        assert!(import(&old, &new).is_err());   // nothing left to bring in
        fs::remove_dir_all(&t).unwrap();
    }
}
