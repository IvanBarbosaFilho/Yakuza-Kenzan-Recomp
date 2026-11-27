//! Checks the installed game's files against the disc's (V1.0.3). A damaged dump or a bad ISO extraction
//! can leave a file with the right size and the wrong bytes -- one player's `bin/scenario/player_pos.bin`
//! hung the game while loading, with a looping sound and a black screen. Every file is compared with
//! game_files.txt (made from a known good copy: size and SHA-1 prefix of the disc's file and, for the
//! files the English translation replaces, of the translated one), so the player is told which file is
//! wrong instead of the game hanging.
//!
//! The game folder may hold the disc's files with the translation beside it (V1.0.2+: translation/USRDIR)
//! or, in installs made before that, the translated files merged in: either version is accepted there.

use crate::install::Shared;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("game_files.txt");

struct Entry<'a> {
    rel: &'a str,
    size_jp: u64,
    sha_jp: &'a str,
    size_en: Option<u64>,
    sha_en: Option<&'a str>,
}

fn entries() -> Vec<Entry<'static>> {
    MANIFEST
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut c = l.split('\t');
            let rel = c.next()?;
            let size_jp = c.next()?.parse().ok()?;
            let sha_jp = c.next()?;
            let se = c.next()?;
            let he = c.next()?;
            Some(Entry {
                rel,
                size_jp,
                sha_jp,
                size_en: if se == "-" { None } else { se.parse().ok() },
                sha_en: if he == "-" { None } else { Some(he) },
            })
        })
        .collect()
}

/// SHA-1 of a file as 16 hex digits (the manifest's prefix); `done` gets the bytes read.
fn sha16(p: &Path, done: &mut dyn FnMut(u64)) -> Option<String> {
    let mut f = fs::File::open(p).ok()?;
    let mut h = sha1_smol::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        done(n as u64);
    }
    Some(h.digest().to_string()[..16].to_string())
}

/// The file at `rel` under `base`, matching each name without regard to case when the exact one is not
/// there (an extraction tool wrote bin/light_anim/dummy.txt for the disc's DUMMY.TXT; Windows and the
/// engine's file layer do not tell them apart either).
fn find_ci(base: &Path, rel: &str) -> PathBuf {
    let exact = base.join(rel);
    if exact.exists() {
        return exact;
    }
    let mut cur = base.to_path_buf();
    for part in rel.split('/') {
        let direct = cur.join(part);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let found = fs::read_dir(&cur).ok().and_then(|rd| {
            rd.flatten().map(|e| e.path()).find(|q| {
                q.file_name().and_then(|n| n.to_str()).map(|n| n.eq_ignore_ascii_case(part)).unwrap_or(false)
            })
        });
        match found {
            Some(q) => cur = q,
            None => return exact,
        }
    }
    cur
}

fn set(p: &Shared, step: &str, frac: f32) {
    if let Ok(mut g) = p.lock() {
        g.step = step.to_string();
        g.frac = frac;
    }
}

fn note(p: &Shared, line: String) {
    if let Ok(mut g) = p.lock() {
        g.log.push(line);
    }
}

/// Check every file; the result goes to the progress (done, error = the summary when files are bad)
/// and to `<data>/work/logs/game_check.txt`.
pub fn run(data: PathBuf, p: Shared) {
    let game = crate::import::game_dir(&data);
    let usr = game.join("PS3_GAME/USRDIR");
    let tr = crate::lang::translation_dir(&data).join("USRDIR");
    let layer = tr.is_dir();
    let list = entries();
    let total: u64 = list.iter().map(|e| e.size_jp.max(e.size_en.unwrap_or(0))).sum::<u64>().max(1);
    let mut read = 0u64;
    let mut bad: Vec<String> = Vec::new();
    set(&p, "Checking the game files…", 0.0);
    note(&p, format!("{} files to check ({:.1} GB)", list.len(), total as f64 / 1e9));
    let mut last = std::time::Instant::now();
    for (i, e) in list.iter().enumerate() {
        let mut tick = |n: u64| {
            read += n;
            if last.elapsed().as_millis() > 150 {
                last = std::time::Instant::now();
                if let Ok(mut g) = p.lock() {
                    g.frac = (read as f64 / total as f64).min(1.0) as f32;
                }
            }
        };
        // the game folder's copy: the disc's file, or (an older install) the translated one
        let f = find_ci(&usr, e.rel);
        match fs::metadata(&f) {
            Err(_) => bad.push(format!("missing: {}", e.rel)),
            Ok(m) => {
                let size_ok = m.len() == e.size_jp || Some(m.len()) == e.size_en;
                if !size_ok {
                    bad.push(format!("damaged (size {} instead of {}): {}", m.len(), e.size_jp, e.rel));
                } else {
                    match sha16(&f, &mut tick) {
                        Some(h) if h == e.sha_jp || Some(h.as_str()) == e.sha_en => {}
                        Some(_) => bad.push(format!("damaged (wrong contents): {}", e.rel)),
                        None => bad.push(format!("unreadable: {}", e.rel)),
                    }
                }
            }
        }
        // the translation's copy, beside the game (V1.0.2+)
        if layer {
            if let (Some(se), Some(he)) = (e.size_en, e.sha_en) {
                let t = find_ci(&tr, e.rel);
                match fs::metadata(&t) {
                    Err(_) => bad.push(format!("missing from the translation: {}", e.rel)),
                    Ok(m) if m.len() != se => bad.push(format!("damaged in the translation (size): {}", e.rel)),
                    Ok(_) => match sha16(&t, &mut tick) {
                        Some(h) if h == he => {}
                        _ => bad.push(format!("damaged in the translation (wrong contents): {}", e.rel)),
                    },
                }
            }
        }
        if i % 200 == 0 {
            if let Ok(mut g) = p.lock() {
                g.step = format!("Checking the game files… {} of {}", i, list.len());
            }
        }
    }
    let summary = if bad.is_empty() {
        format!("All {} game files are intact.", list.len())
    } else {
        format!("{} damaged or missing game file(s): extract the game again from your disc/ISO and reinstall.", bad.len())
    };
    for b in bad.iter().take(30) {
        note(&p, b.clone());
    }
    if bad.len() > 30 {
        note(&p, format!("… and {} more (see work/logs/game_check.txt)", bad.len() - 30));
    }
    // the report beside the other logs
    let logs = data.join("work/logs");
    let _ = fs::create_dir_all(&logs);
    if let Ok(mut f) = fs::File::create(logs.join("game_check.txt")) {
        let _ = writeln!(f, "{summary}");
        let _ = writeln!(f, "game: {}{}", game.display(), if layer { " (translation beside it)" } else { "" });
        for b in &bad {
            let _ = writeln!(f, "{b}");
        }
    }
    if let Ok(mut g) = p.lock() {
        g.frac = 1.0;
        g.step = summary.clone();
        g.log.push(summary.clone());
        if !bad.is_empty() {
            g.error = Some(summary);
        }
        g.done = true;
    }
}

/// The last check's result, for the Home page: None = never checked, Some(true) = all good.
pub fn last_result(data: &Path) -> Option<(bool, String)> {
    let t = fs::read_to_string(data.join("work/logs/game_check.txt")).ok()?;
    let first = t.lines().next()?.to_string();
    Some((first.starts_with("All "), first))
}
