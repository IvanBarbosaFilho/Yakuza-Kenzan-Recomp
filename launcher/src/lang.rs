//! The game's language: the community English translation as a layer over the untouched game, or
//! the Japanese original.
//!
//! Layout (from V1.1): `game/` is the disc as it is; `translation/` beside it holds the translation's
//! files (USRDIR/<path>, PARAM.SFO) and its EBOOT.elf. The engine reads a translated file instead
//! of the game's when PS3_TRANSLATION_DIR names that folder (ppu_fs_loose_mods.h), so switching
//! language copies nothing. `language.txt` keeps the choice.
//!
//! Earlier versions installed the translation INTO game/ ("merged": game/ has "EBOOT RPCS3.BIN").
//! English keeps working there as before; Japanese needs the disc's own versions of the 1040 files
//! the translation replaced: `convert` takes them from the ISO once and moves the translated ones
//! to translation/ (every move journaled in convert.journal, undone if anything fails, and undone
//! at the next start if the launcher was closed half way).
//!
//! The Japanese executables and game modules come in the package (AppImage usr/share/kenzan/japanese,
//! Windows engine/japanese), so the language is one switch with nothing to download.

use crate::install::{self, Shared};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The translation's files, USRDIR-relative ("EBOOT RPCS3.BIN" aside): what `convert` moves.
const PATCH_FILES: &str = include_str!("patch_files.txt");


#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    En,
    Jp,
}

pub fn get(data: &Path) -> Lang {
    match fs::read_to_string(data.join("language.txt")).map(|s| s.trim().to_lowercase()) {
        Ok(s) if s == "jp" || s == "ja" => Lang::Jp,
        _ => Lang::En,
    }
}

pub fn set(data: &Path, l: Lang) -> std::io::Result<()> {
    fs::write(data.join("language.txt"), if l == Lang::Jp { "jp\n" } else { "en\n" })
}

/// translation/: beside the game folder (which game_location.txt may put on another drive).
pub fn translation_dir(data: &Path) -> PathBuf {
    let g = crate::import::game_dir(data);
    g.parent().map(|p| p.join("translation")).unwrap_or_else(|| data.join("translation"))
}

pub fn has_translation(data: &Path) -> bool {
    translation_dir(data).join("USRDIR/EBOOT.elf").is_file()
}

/// game/ has the translation installed into it (V1.0.x).
pub fn merged(data: &Path) -> bool {
    crate::import::game_dir(data).join("PS3_GAME/USRDIR/EBOOT RPCS3.BIN").is_file()
}

/// English can be played: a translation layer, or a merged install.
pub fn english_ready(data: &Path) -> bool {
    has_translation(data) || merged(data)
}

/// Japanese can be played without the ISO: the game files are the disc's own.
pub fn japanese_game_ready(data: &Path) -> bool {
    let g = crate::import::game_dir(data);
    g.join("PS3_GAME/USRDIR/EBOOT.BIN").is_file() && !merged(data)
}

/// The Japanese executables and modules: japanese/ beside the package's own modules, or work/jp
/// (its modules in build/) in a dev tree.
pub fn jp_dir(lay: &crate::layout::Layout) -> PathBuf {
    if lay.app.is_some() { lay.modules.join("japanese") } else { lay.work.join("jp") }
}

/// The package has them (always, unless files were deleted).
pub fn jp_ready(lay: &crate::layout::Layout) -> bool {
    jp_dir(lay).join("EBOOT.elf").is_file()
}

fn set_p(p: &Shared, step: &str, frac: f32) {
    if let Ok(mut g) = p.lock() {
        g.step = step.to_string();
        g.frac = frac;
        g.log.push(step.to_string());
    }
}

fn finish(p: &Shared, r: Result<String, String>) {
    if let Ok(mut g) = p.lock() {
        g.done = true;
        match r {
            Ok(m) => {
                g.frac = 1.0;
                g.step = m.clone();
                g.log.push(m);
            }
            Err(e) => {
                g.log.push(format!("ERROR: {e}"));
                g.error = Some(e);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// A merged install (V1.0.x) -> the game as on the disc + translation/

fn journal(data: &Path) -> PathBuf {
    data.join("convert.journal")
}

/// Undo a conversion the launcher did not finish (it was closed or crashed half way).
pub fn recover(data: &Path) -> Option<String> {
    let j = journal(data);
    let text = fs::read_to_string(&j).ok()?;
    let mut undone = 0;
    for line in text.lines().rev() {
        if let Some((a, b)) = line.split_once('\t') {
            if Path::new(b).exists() && !Path::new(a).exists() {
                if let Some(d) = Path::new(a).parent() {
                    let _ = fs::create_dir_all(d);
                }
                if fs::rename(b, a).is_ok() {
                    undone += 1;
                }
            }
        }
    }
    let _ = fs::remove_file(&j);
    Some(format!("An unfinished language conversion was undone ({undone} file(s) put back)."))
}

struct Moves {
    file: fs::File,
    done: Vec<(PathBuf, PathBuf)>,
}

impl Moves {
    fn mv(&mut self, a: &Path, b: &Path) -> Result<(), String> {
        if let Some(d) = b.parent() {
            fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        // journaled before the move: recover() only moves back what has really moved
        writeln!(self.file, "{}\t{}", a.display(), b.display()).map_err(|e| e.to_string())?;
        self.file.sync_data().map_err(|e| e.to_string())?;
        fs::rename(a, b).map_err(|e| format!("{} -> {}: {e}", a.display(), b.display()))?;
        self.done.push((a.to_path_buf(), b.to_path_buf()));
        Ok(())
    }
    fn undo(&mut self) {
        for (a, b) in self.done.drain(..).rev() {
            let _ = fs::rename(&b, &a);
        }
    }
}

/// `disc`: the ISO (or a folder with PS3_GAME) the game was installed from.
pub fn convert(data: PathBuf, disc: PathBuf, p: Shared) {
    let r = convert_inner(&data, &disc, &p);
    finish(&p, r);
    install::save_log(&data, &p, None);
}

fn convert_inner(data: &Path, disc: &Path, p: &Shared) -> Result<String, String> {
    if !merged(data) {
        return Err("Nothing to convert: the game already is as on the disc.".into());
    }
    let game = crate::import::game_dir(data);
    let base = game.parent().ok_or("the game folder has no parent")?.to_path_buf();
    let usr = game.join("PS3_GAME/USRDIR");
    let files: Vec<&str> = PATCH_FILES.lines().filter(|l| !l.is_empty()).collect();
    let orig = base.join("original.installing");
    let tr_new = base.join("translation.installing");
    for d in [&orig, &tr_new] {
        let _ = fs::remove_dir_all(d);
    }
    // 1. the disc's own versions of the translated files (4.4 GB)
    set_p(p, "Reading the original files from the disc…", 0.02);
    let free = fs4::available_space(&base).map_err(|e| e.to_string())?;
    if free < 5_000_000_000 {
        return Err(format!("not enough space: {:.1} GB free, about 5 GB needed", free as f64 / 1e9));
    }
    if disc.is_dir() {
        let src = if disc.join("PS3_GAME").is_dir() { disc.join("PS3_GAME") } else { disc.to_path_buf() };
        for (i, f) in files.iter().enumerate().chain([(files.len(), &"../PARAM.SFO")]) {
            let (s, d) = (src.join("USRDIR").join(f), orig.join("PS3_GAME/USRDIR").join(f));
            fs::create_dir_all(d.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::copy(&s, &d).map_err(|e| format!("{}: {e}", s.display()))?;
            if i % 20 == 0 {
                if let Ok(mut g) = p.lock() {
                    g.frac = 0.02 + 0.8 * i as f32 / files.len() as f32;
                }
            }
        }
    } else {
        if !disc.is_file() {
            return Err("the game ISO does not exist".into());
        }
        let list = base.join("original.list");
        let mut t = String::from("PS3_GAME/PARAM.SFO\n");
        for f in &files {
            t.push_str("PS3_GAME/USRDIR/");
            t.push_str(f);
            t.push('\n');
        }
        fs::write(&list, t).map_err(|e| e.to_string())?;
        let mut out = std::ffi::OsString::from("-o");
        out.push(orig.as_os_str());
        let mut l = std::ffi::OsString::from("@");
        l.push(list.as_os_str());
        let r = install::run_7z(p, &["x".as_ref(), disc.as_os_str(), out.as_os_str(), l.as_os_str()], 0.02, 0.82);
        let _ = fs::remove_file(&list);
        r?;
    }
    let ousr = orig.join("PS3_GAME/USRDIR");
    let sfo = fs::read(orig.join("PS3_GAME/PARAM.SFO")).unwrap_or_default();
    if !sfo.windows(9).any(|w| w == b"BLJM60064") {
        let _ = fs::remove_dir_all(&orig);
        return Err("that disc is not BLJM60064 (Ryu ga Gotoku Kenzan!)".into());
    }
    if let Some(f) = files.iter().find(|f| !ousr.join(f).is_file()) {
        let _ = fs::remove_dir_all(&orig);
        return Err(format!("the disc has no {f}: is this the original, untouched ISO?"));
    }

    // 2. the swap, journaled
    set_p(p, "Separating the translation from the game…", 0.84);
    let mut mv = Moves {
        file: fs::OpenOptions::new().create(true).append(true).open(journal(data)).map_err(|e| e.to_string())?,
        done: vec![],
    };
    let tusr = tr_new.join("USRDIR");
    let r = (|| -> Result<(), String> {
        for f in &files {
            mv.mv(&usr.join(f), &tusr.join(f))?;
            mv.mv(&ousr.join(f), &usr.join(f))?;
        }
        mv.mv(&game.join("PS3_GAME/PARAM.SFO"), &tr_new.join("PARAM.SFO"))?;
        mv.mv(&orig.join("PS3_GAME/PARAM.SFO"), &game.join("PS3_GAME/PARAM.SFO"))?;
        for f in ["EBOOT.elf", "EBOOT RPCS3.BIN"] {
            if usr.join(f).is_file() {
                mv.mv(&usr.join(f), &tusr.join(f))?;
            }
        }
        if install::sha1_file(&tusr.join("EBOOT.elf")).as_deref() != Some(install::EN_EBOOT_SHA1) {
            return Err("the installed English executable is not the expected one".into());
        }
        let tr = base.join("translation");
        if tr.exists() {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            mv.mv(&tr, &base.join(format!("translation.old-{secs}")))?;
        }
        mv.mv(&tr_new, &tr)?;
        Ok(())
    })();
    if let Err(e) = r {
        mv.undo();
        drop(mv);
        let _ = fs::remove_file(journal(data));
        let _ = fs::remove_dir_all(&orig);
        let _ = fs::remove_dir_all(&tr_new);
        return Err(format!("{e} -- nothing was changed"));
    }
    drop(mv);
    let _ = fs::remove_file(journal(data));
    let _ = fs::remove_dir_all(&orig);
    Ok("Done: the game is as on the disc, the translation is a layer over it.".into())
}

// ---------------------------------------------------------------------------------------------
// A Japanese-only install that wants English: the translation archive -> translation/

pub fn add_translation(data: PathBuf, patch: PathBuf, p: Shared) {
    let r = (|| -> Result<String, String> {
        let base = crate::import::game_dir(&data).parent().ok_or("no game folder")?.to_path_buf();
        let tmp = base.join("patch.installing");
        let tr_new = base.join("translation.installing");
        install::make_translation(&patch, &tmp, &tr_new, &p)?;
        let tr = base.join("translation");
        let _ = fs::remove_dir_all(&tr);
        fs::rename(&tr_new, &tr).map_err(|e| e.to_string())?;
        Ok("The English translation is installed.".into())
    })();
    finish(&p, r);
    install::save_log(&data, &p, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn sh(p: &Path) -> String {
        install::sha1_file(p).unwrap_or_default()
    }

    /// KZ_TEST_GAME_JP=<an untouched game folder with PS3_GAME> KZ_TEST_PATCH=<the translation folder>
    /// KZ_TEST_DIR=<scratch dir on the same drive (hard links)>: a split install, a merged (V1.0.x)
    /// install converted, an interrupted conversion recovered.
    #[test]
    fn split_install_convert_recover() {
        let (Ok(jp), Ok(patch), Ok(t)) =
            (std::env::var("KZ_TEST_GAME_JP"), std::env::var("KZ_TEST_PATCH"), std::env::var("KZ_TEST_DIR")) else { return };
        let (jp, patch, t) = (PathBuf::from(jp), PathBuf::from(patch), PathBuf::from(t).join("split"));   // its own folder
        let _ = fs::remove_dir_all(&t);
        let p: Shared = Arc::new(Mutex::new(Default::default()));
        let probe = "bin/scenario/scenario.bin";
        let jp_sha = sh(&jp.join("PS3_GAME/USRDIR").join(probe));
        let en_sha = sh(&patch.join("USRDIR").join(probe));
        assert_ne!(jp_sha, en_sha);

        // 1. a new install: the disc untouched + translation/
        let a = t.join("new");
        fs::create_dir_all(&a).unwrap();
        install::run(a.clone(), jp.clone(), Some(patch.clone()), p.clone());
        assert!(p.lock().unwrap().error.is_none(), "{:?}", p.lock().unwrap().log);
        assert_eq!(sh(&a.join("game/PS3_GAME/USRDIR").join(probe)), jp_sha);
        assert_eq!(sh(&a.join("translation/USRDIR").join(probe)), en_sha);
        assert_eq!(sh(&a.join("translation/USRDIR/EBOOT.elf")), install::EN_EBOOT_SHA1);
        assert!(!a.join("translation/USRDIR/EBOOT RPCS3.BIN").exists());
        assert!(has_translation(&a) && !merged(&a) && japanese_game_ready(&a) && get(&a) == Lang::En);

        // 2. a V1.0.x install (the translation copied into game/), converted
        let b = t.join("old");
        install::link_or_copy(&jp.join("PS3_GAME"), &b.join("game/PS3_GAME")).unwrap();
        let usr = b.join("game/PS3_GAME/USRDIR");
        for f in PATCH_FILES.lines().chain(["EBOOT RPCS3.BIN"]) {
            let d = usr.join(f);
            let _ = fs::remove_file(&d);   // a hard link into the real game: never written through
            fs::copy(patch.join("USRDIR").join(f), &d).unwrap();
        }
        let _ = fs::remove_file(b.join("game/PS3_GAME/PARAM.SFO"));
        fs::copy(patch.join("PARAM.SFO"), b.join("game/PS3_GAME/PARAM.SFO")).unwrap();
        fs::copy(a.join("translation/USRDIR/EBOOT.elf"), usr.join("EBOOT.elf")).unwrap();
        assert!(merged(&b) && english_ready(&b) && !japanese_game_ready(&b));
        assert_eq!(sh(&jp.join("PS3_GAME/USRDIR").join(probe)), jp_sha, "the real game was written");

        // 2a. a disc without one of the files: refused, nothing moved
        let bad = t.join("baddisc");
        install::link_or_copy(&jp.join("PS3_GAME"), &bad.join("PS3_GAME")).unwrap();
        fs::remove_file(bad.join("PS3_GAME/USRDIR").join(probe)).unwrap();
        let q: Shared = Arc::new(Mutex::new(Default::default()));
        convert(b.clone(), bad.clone(), q.clone());
        assert!(q.lock().unwrap().error.is_some());
        assert!(merged(&b) && sh(&usr.join(probe)) == en_sha && !journal(&b).exists());

        // 2b. the real conversion
        let q: Shared = Arc::new(Mutex::new(Default::default()));
        // KZ_TEST_ISO: from an ISO (7-Zip and its list of files) instead of the folder
        let src = std::env::var("KZ_TEST_ISO").map(PathBuf::from).unwrap_or(jp.clone());
        convert(b.clone(), src, q.clone());
        assert!(q.lock().unwrap().error.is_none(), "{:?}", q.lock().unwrap().log);
        assert!(!merged(&b) && has_translation(&b) && japanese_game_ready(&b) && !journal(&b).exists());
        for f in PATCH_FILES.lines().step_by(97) {
            assert_eq!(sh(&usr.join(f)), sh(&jp.join("PS3_GAME/USRDIR").join(f)), "{f}");
            assert_eq!(sh(&b.join("translation/USRDIR").join(f)), sh(&patch.join("USRDIR").join(f)), "{f}");
        }
        assert_eq!(sh(&b.join("translation/USRDIR/EBOOT.elf")), install::EN_EBOOT_SHA1);
        assert!(!usr.join("EBOOT.elf").exists() && !usr.join("EBOOT RPCS3.BIN").exists());

        // 3. an interrupted conversion is put back at the next start
        let x = usr.join(probe);
        let y = t.join("moved.bin");
        fs::write(journal(&b), format!("{}\t{}\n", x.display(), y.display())).unwrap();
        fs::rename(&x, &y).unwrap();
        assert!(recover(&b).is_some());
        assert!(x.is_file() && !y.exists() && !journal(&b).exists());

        fs::remove_dir_all(&t).unwrap();
        assert_eq!(sh(&jp.join("PS3_GAME/USRDIR").join(probe)), jp_sha, "the real game was written");
    }
}
