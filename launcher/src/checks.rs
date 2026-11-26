//! What the Home page confirms before Play: the game's executable and its version, the port's
//! own files for the chosen frame rate, and the player's saves.

use std::fs;
use std::io::Read;
use std::path::Path;

/// Executables the port was recompiled from (SHA-1 of the decrypted EBOOT.elf).
const KNOWN_EBOOTS: &[(&str, &str)] = &[
    ("61669b3de79f71dfcf6ff3e6c4441b3df3d9da43", "BLJM60064 · v1.00 · English translation"),
    ("d710038a7fe465a4", "BLJM60064 · v1.00 · Japanese"),
];

pub struct Status {
    pub ok: bool,
    pub text: String,
}

fn sha1_file(p: &Path) -> Option<String> {
    let mut f = fs::File::open(p).ok()?;
    let mut h = sha1_smol::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.digest().to_string())
}

pub fn game(root: &Path) -> Status {
    game_at(&crate::import::game_dir(root))
}

/// The game folder `game` (the one holding PS3_GAME): its EBOOT, a version this port runs?
pub fn game_at(game: &Path) -> Status {
    let eboot = game.join("PS3_GAME/USRDIR/EBOOT.elf");
    if !eboot.exists() {
        // the disc as it is (V1.1 layout: the translation is a layer beside it, lang.rs)
        let sfo = fs::read(game.join("PS3_GAME/PARAM.SFO")).unwrap_or_default();
        if game.join("PS3_GAME/USRDIR/EBOOT.BIN").is_file() && sfo.windows(9).any(|w| w == b"BLJM60064") {
            return Status { ok: true, text: "BLJM60064 · v1.00".into() };
        }
        return Status { ok: false, text: "Game not found (game/PS3_GAME)".into() };
    }
    match sha1_file(&eboot) {
        Some(h) => match KNOWN_EBOOTS.iter().find(|(k, _)| h.starts_with(k)) {
            Some((_, name)) => Status { ok: true, text: (*name).into() },
            None => Status { ok: false, text: format!("Unsupported game version ({})", &h[..12]) },
        },
        None => Status { ok: false, text: "EBOOT unreadable".into() },
    }
}

/// The port's executable and the game module for the chosen frame rate.
pub fn port(lay: &crate::layout::Layout, fps: &str) -> Status {
    if !lay.exe.exists() {
        return Status { ok: false, text: "Port not found (KenzanRecomp)".into() };
    }
    let lang = crate::lang::get(&lay.data);
    if lang == crate::lang::Lang::Jp && !crate::lang::jp_ready(lay) {
        return Status { ok: false, text: "The package's Japanese files are missing".into() };
    }
    let (module, eboot) = lay.mode(fps, lang);
    if !module.exists() || !eboot.exists() {
        return Status { ok: false, text: format!("Files for {fps} FPS mode are missing") };
    }
    Status { ok: true, text: format!("Port ready · {fps} FPS") }
}

/// The language the game will be played in, and whether it can be.
pub fn language(lay: &crate::layout::Layout) -> Status {
    use crate::lang::*;
    match get(&lay.data) {
        Lang::En if english_ready(&lay.data) => Status { ok: true, text: "English (fan translation)".into() },
        Lang::En => Status { ok: false, text: "English: the translation is not installed".into() },
        Lang::Jp if !japanese_game_ready(&lay.data) => {
            Status { ok: false, text: "Japanese: the game still has the translation in it".into() }
        }
        Lang::Jp => Status { ok: true, text: "Japanese (original)".into() },
    }
}

pub fn saves(lay: &crate::layout::Layout) -> Status {
    let dir = lay.saves();
    let n = fs::read_dir(&dir)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("BLJM60064-LIST"))
                .count()
        })
        .unwrap_or(0);
    if n > 0 {
        Status { ok: true, text: format!("{n} save(s) found") }
    } else {
        Status { ok: true, text: "No saves yet".into() }
    }
}
