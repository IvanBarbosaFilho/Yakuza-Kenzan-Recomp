//! Game installer: the player's own disc dump (an ISO or an extracted folder) becomes `game/`, as
//! on the disc, and the community English translation (optional: without it the game is the
//! Japanese original) becomes `translation/` beside it -- a layer the engine reads over the game
//! (lang.rs). The translation's executable is checked against the one the port was recompiled from.
//!
//! Steps: free-space check -> the disc's PS3_GAME (7-Zip reads the ISO's UDF tree: right names,
//! multi-extent files whole) -> the patch's USRDIR and PARAM.SFO into translation/ -> "EBOOT RPCS3.BIN"
//! (a fake-signed SELF: plain or zlib segments, no keys) unwrapped to EBOOT.elf -> SHA-1 check
//! -> only then the old game/ (and translation/) are moved aside (renamed, never deleted) and the
//! new ones put in place.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

pub const EN_EBOOT_SHA1: &str = "61669b3de79f71dfcf6ff3e6c4441b3df3d9da43";
const NEED_BYTES: u64 = 23 * 1024 * 1024 * 1024; // the disc's 21 GB plus room for the patch

#[derive(Default)]
pub struct Progress {
    pub step: String,
    pub frac: f32,
    pub log: Vec<String>,
    pub done: bool,
    pub error: Option<String>,
}

pub type Shared = Arc<Mutex<Progress>>;

fn set(p: &Shared, step: &str, frac: f32) {
    if let Ok(mut g) = p.lock() {
        g.step = step.to_string();
        g.frac = frac;
        g.log.push(step.to_string());
    }
}

fn note(p: &Shared, line: String) {
    if let Ok(mut g) = p.lock() {
        g.log.push(line);
    }
}

fn frac(p: &Shared, f: f32) {
    if let Ok(mut g) = p.lock() {
        g.frac = f;
    }
}

/// 7-Zip's command-line program, from PATH.
pub fn seven_zip() -> Result<String, String> {
    // Windows: the 7-Zip shipped in engine/ (7z.exe + 7z.dll), then an installed one
    #[cfg(windows)]
    {
        let mut c = vec![];
        if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
            c.push(d.join("engine").join("7z.exe"));
        }
        for v in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Ok(pf) = std::env::var(v) {
                c.push(std::path::PathBuf::from(pf).join("7-Zip").join("7z.exe"));
            }
        }
        if let Some(f) = c.into_iter().find(|f| f.is_file()) {
            return Ok(f.display().to_string());
        }
    }
    for c in ["7zz", "7z", "7za"] {
        if Command::new(c).arg("i").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok() {
            return Ok(c.to_string());
        }
    }
    Err(if cfg!(windows) { "7-Zip not found (engine\\7z.exe). Install 7-Zip from 7-zip.org.".into() }
        else { "7-Zip not found (7zz/7z). Start the launcher through launcher.sh.".into() })
}

/// Runs 7-Zip, mapping its percentage output onto [lo, hi] of the overall bar.
pub(crate) fn run_7z(p: &Shared, args: &[&std::ffi::OsStr], lo: f32, hi: f32) -> Result<(), String> {
    let exe = seven_zip()?;
    let mut child = Command::new(&exe)
        .args(args)
        .args(["-y", "-bsp1", "-bso0", "-bse1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {exe}: {e}"))?;
    let out = child.stdout.take().ok_or("no output from 7-Zip")?;
    // -bsp1 redraws its progress with backspaces/CR: split on those as well
    let mut r = BufReader::new(out);
    let mut buf = Vec::new();
    let mut tail = String::new();
    loop {
        buf.clear();
        let n = r.read_until(b'\r', &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let s = String::from_utf8_lossy(&buf).replace('\u{8}', " ");
        for piece in s.split(['\r', '\n']) {
            let t = piece.trim();
            if let Some(pct) = t.split('%').next().and_then(|x| x.trim().parse::<f32>().ok()) {
                frac(p, lo + (hi - lo) * (pct / 100.0).clamp(0.0, 1.0));
            } else if t.contains("ERROR") || t.contains("Error") {
                tail = t.to_string();
            }
        }
    }
    let st = child.wait().map_err(|e| e.to_string())?;
    // A PS3 disc's UDF has headers 7-Zip calls "Headers Error" (exit 2) while it still
    // extracts every file whole (checked byte for byte against a working install). The
    // checks after the extraction (EBOOT.BIN, PARAM.SFO, the executable's SHA-1) judge it.
    let headers_only = st.code() == Some(2) && tail.contains("Headers Error");
    if headers_only {
        note(p, "7-Zip warning: \"Headers Error\" (normal for PS3 discs).".into());
    } else if !st.success() {
        return Err(format!("7-Zip failed ({st}). {tail}"));
    }
    frac(p, hi);
    Ok(())
}

/// Hard-links (same disk) or copies a tree; used for an already-extracted disc folder.
pub(crate) fn link_or_copy(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    for e in fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let s = e.path();
        let d = dst.join(e.file_name());
        if s.is_dir() {
            link_or_copy(&s, &d)?;
        } else if fs::hard_link(&s, &d).is_err() {
            fs::copy(&s, &d).map_err(|e| format!("{}: {e}", s.display()))?;
        }
    }
    Ok(())
}

/// The folder holding the patch's USRDIR and PARAM.SFO (the archive may wrap them in a folder).
fn find_patch_root(dir: &Path, depth: u32) -> Option<PathBuf> {
    if dir.join("USRDIR").is_dir() && dir.join("PARAM.SFO").is_file() {
        return Some(dir.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    for e in fs::read_dir(dir).ok()?.flatten() {
        if e.path().is_dir() {
            if let Some(r) = find_patch_root(&e.path(), depth - 1) {
                return Some(r);
            }
        }
    }
    None
}

fn be32(d: &[u8], o: usize) -> Result<u32, String> {
    d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated SELF".into())
}
fn be64(d: &[u8], o: usize) -> Result<u64, String> {
    d.get(o..o + 8)
        .map(|b| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
        .ok_or("truncated SELF".into())
}
fn be16(d: &[u8], o: usize) -> Result<u16, String> {
    d.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]])).ok_or("truncated SELF".into())
}

/// A fake-signed SELF (key revision 0x8000: segments plain or zlib, nothing encrypted) back
/// to its ELF -- the same rebuild as engine/tools/unfself.py.
pub fn unfself(d: &[u8]) -> Result<Vec<u8>, String> {
    if be32(d, 0)? != 0x5343_4500 {
        return Err("the translation's EBOOT is not a SELF".into());
    }
    if be16(d, 8)? != 0x8000 {
        return Err("the translation's EBOOT is encrypted (expected \"EBOOT RPCS3.BIN\")".into());
    }
    let elf_filesize = be64(d, 0x18)? as usize;
    let elf_off = be64(d, 0x30)? as usize;
    let phdr_off = be64(d, 0x38)? as usize;
    let shdr_off = be64(d, 0x40)? as usize;
    let seginfo_off = be64(d, 0x48)? as usize;
    if d.get(elf_off..elf_off + 4) != Some(b"\x7fELF".as_slice()) {
        return Err("SELF without an ELF header".into());
    }
    let e_phoff = be64(d, elf_off + 0x20)? as usize;
    let e_shoff = be64(d, elf_off + 0x28)? as usize;
    let e_phentsize = be16(d, elf_off + 0x36)? as usize;
    let e_phnum = be16(d, elf_off + 0x38)? as usize;
    let e_shentsize = be16(d, elf_off + 0x3A)? as usize;
    let e_shnum = be16(d, elf_off + 0x3C)? as usize;
    let mut out = vec![0u8; elf_filesize];
    let put = |out: &mut Vec<u8>, at: usize, src: &[u8]| -> Result<(), String> {
        out.get_mut(at..at + src.len()).ok_or("ELF out of bounds")?.copy_from_slice(src);
        Ok(())
    };
    put(&mut out, 0, d.get(elf_off..elf_off + 64).ok_or("truncated SELF")?)?;
    let phsz = e_phnum * e_phentsize;
    put(&mut out, e_phoff, d.get(phdr_off..phdr_off + phsz).ok_or("truncated SELF")?)?;
    for i in 0..e_phnum {
        let o = seginfo_off + i * 0x20;
        let f_off = be64(d, o)? as usize;
        let f_size = be64(d, o + 8)? as usize;
        let comp = be32(d, o + 16)?;
        let po = phdr_off + i * 0x38;
        let p_offset = be64(d, po + 8)? as usize;
        let p_filesz = be64(d, po + 32)? as usize;
        if f_off == 0 || f_size == 0 {
            continue;
        }
        let blob = d.get(f_off..f_off + f_size).ok_or("segment outside the SELF")?;
        let seg = if comp == 2 {
            let mut z = flate2::read::ZlibDecoder::new(blob);
            let mut v = Vec::with_capacity(p_filesz);
            z.read_to_end(&mut v).map_err(|e| format!("segment {i} failed to decompress: {e}"))?;
            v
        } else {
            blob.to_vec()
        };
        if seg.len() != p_filesz {
            return Err(format!("segment {i}: {} bytes, expected {p_filesz}", seg.len()));
        }
        put(&mut out, p_offset, &seg)?;
    }
    // non-alloc sections (symbols etc.) are stored verbatim at a constant delta
    let delta = shdr_off as i64 - e_shoff as i64;
    for i in 0..e_shnum {
        let o = shdr_off + i * e_shentsize;
        let typ = be32(d, o + 4)?;
        let flags = be64(d, o + 8)?;
        let off = be64(d, o + 24)? as i64;
        let size = be64(d, o + 32)? as usize;
        if typ == 8 || size == 0 || flags & 2 != 0 {
            continue;
        }
        let src = off + delta;
        if src < 0 || src as usize + size > d.len() || off as usize + size > out.len() {
            continue;
        }
        let (src, off) = (src as usize, off as usize);
        out[off..off + size].copy_from_slice(&d[src..src + size]);
    }
    if e_shnum > 0 {
        let shsz = e_shnum * e_shentsize;
        if let Some(s) = d.get(shdr_off..shdr_off + shsz) {
            if e_shoff + shsz <= out.len() {
                out[e_shoff..e_shoff + shsz].copy_from_slice(s);
            }
        }
    }
    Ok(out)
}

pub(crate) fn sha1_file(p: &Path) -> Option<String> {
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

fn sha1_hex(d: &[u8]) -> String {
    let mut h = sha1_smol::Sha1::new();
    h.update(d);
    h.digest().to_string()
}

/// The whole install, on its own thread; `p` carries the progress back to the window.
/// `patch` None: the Japanese original, no translation.
pub fn run(root: PathBuf, disc: PathBuf, patch: Option<PathBuf>, p: Shared) {
    let r = install(&root, &disc, patch.as_deref(), &p);
    if let Ok(mut g) = p.lock() {
        g.done = true;
        match r {
            Ok(()) => {
                g.step = "Installation complete and verified.".into();
                g.frac = 1.0;
                let st = g.step.clone();
                g.log.push(st);
            }
            Err(e) => {
                g.log.push(format!("ERROR: {e}"));
                g.error = Some(e);
            }
        }
    }
}

/// The translation (an archive, or its extracted folder) -> `tr_new` (USRDIR/<files>, PARAM.SFO,
/// USRDIR/EBOOT.elf checked); `tmp` is where an archive is unpacked (removed after).
pub(crate) fn make_translation(patch: &Path, tmp: &Path, tr_new: &Path, p: &Shared) -> Result<usize, String> {
    for d in [tmp, tr_new] {
        if d.exists() {
            fs::remove_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
    }
    if !patch.exists() {
        return Err("the translation file does not exist".into());
    }
    let proot = if patch.is_dir() {
        find_patch_root(patch, 2).ok_or("the translation folder has no USRDIR and PARAM.SFO")?
    } else {
        set(p, "Extracting the translation…", 0.76);
        let mut out = std::ffi::OsString::from("-o");
        out.push(tmp.as_os_str());
        run_7z(p, &["x".as_ref(), patch.as_os_str(), out.as_os_str()], 0.76, 0.88)?;
        find_patch_root(tmp, 3).ok_or("the translation archive has no USRDIR and PARAM.SFO")?
    };
    set(p, "Preparing the translation…", 0.89);
    let rpcs3_eboot = proot.join("USRDIR/EBOOT RPCS3.BIN");
    if !rpcs3_eboot.is_file() {
        return Err("the translation has no \"EBOOT RPCS3.BIN\"".into());
    }
    let usr = tr_new.join("USRDIR");
    link_or_copy(&proot.join("USRDIR"), &usr)?;
    fs::copy(proot.join("PARAM.SFO"), tr_new.join("PARAM.SFO")).map_err(|e| e.to_string())?;
    let mut n = 0usize;
    count_files(&usr, &mut n);
    note(p, format!("{n} translation files"));

    // the executable the port runs in English
    set(p, "Preparing the executable…", 0.93);
    let selfd = fs::read(usr.join("EBOOT RPCS3.BIN")).map_err(|e| e.to_string())?;
    let elf = unfself(&selfd)?;
    let h = sha1_hex(&elf);
    note(p, format!("EBOOT.elf SHA-1 {h}"));
    if h != EN_EBOOT_SHA1 {
        return Err(format!(
            "the resulting executable is not the expected one ({}…). A different translation version?",
            &h[..12]
        ));
    }
    fs::write(usr.join("EBOOT.elf"), &elf).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(usr.join("EBOOT RPCS3.BIN"));
    if tmp.exists() {
        let _ = fs::remove_dir_all(tmp);
    }
    Ok(n)
}

fn count_files(d: &Path, n: &mut usize) {
    for e in fs::read_dir(d).into_iter().flatten().flatten() {
        if e.path().is_dir() {
            count_files(&e.path(), n);
        } else {
            *n += 1;
        }
    }
}

/// `old` aside as `<name>.old-<secs>` (a symlink is removed): never deleted.
fn move_aside(old: &Path, p: &Shared) -> Result<(), String> {
    if let Ok(meta) = fs::symlink_metadata(old) {
        if meta.file_type().is_symlink() {
            fs::remove_file(old).map_err(|e| e.to_string())?;
        } else {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let name = old.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let to = old.with_file_name(format!("{name}.old-{secs}"));
            fs::rename(old, &to).map_err(|e| e.to_string())?;
            note(p, format!("The previous {name} was kept at {}", to.display()));
        }
    }
    Ok(())
}

fn install(root: &Path, disc: &Path, patch: Option<&Path>, p: &Shared) -> Result<(), String> {
    let stage = root.join("game.installing");
    let patch_tmp = root.join("patch.installing");
    let tr_stage = root.join("translation.installing");
    for d in [&stage, &patch_tmp, &tr_stage] {
        if d.exists() {
            fs::remove_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
    }

    set(p, "Checking the files and free space…", 0.01);
    let disc_is_dir = disc.is_dir();
    if !disc_is_dir && !disc.is_file() {
        return Err("the game ISO (or folder) does not exist".into());
    }
    if disc_is_dir && !disc.join("PS3_GAME").is_dir() {
        return Err("the game folder must contain PS3_GAME".into());
    }
    if let Some(patch) = patch {
        if !patch.exists() {
            return Err("the translation file does not exist".into());
        }
    }
    if !disc_is_dir {
        let free = fs4::available_space(root).map_err(|e| e.to_string())?;
        note(p, format!("Free space: {:.1} GB", free as f64 / 1e9));
        if free < NEED_BYTES {
            return Err(format!(
                "not enough space: {:.1} GB free, about 23 GB needed",
                free as f64 / 1e9
            ));
        }
    }

    // 1. the disc
    if disc_is_dir {
        set(p, "Copying the game from the folder…", 0.05);
        link_or_copy(&disc.join("PS3_GAME"), &stage.join("PS3_GAME"))?;
        if disc.join("PS3_DISC.SFB").is_file() {
            let _ = fs::copy(disc.join("PS3_DISC.SFB"), stage.join("PS3_DISC.SFB"));
        }
        frac(p, 0.75);
    } else {
        set(p, "Extracting the ISO (this may take a few minutes)…", 0.03);
        let mut out = std::ffi::OsString::from("-o");
        out.push(stage.as_os_str());
        run_7z(
            p,
            &["x".as_ref(), disc.as_os_str(), out.as_os_str(), "PS3_GAME".as_ref(), "PS3_DISC.SFB".as_ref()],
            0.03,
            0.75,
        )?;
    }
    let usrdir = stage.join("PS3_GAME/USRDIR");
    if !usrdir.join("EBOOT.BIN").is_file() {
        return Err("PS3_GAME/USRDIR/EBOOT.BIN is not in the ISO: is this really Ryu ga Gotoku Kenzan!?".into());
    }
    let sfo = fs::read(stage.join("PS3_GAME/PARAM.SFO")).unwrap_or_default();
    if !sfo.windows(9).any(|w| w == b"BLJM60064") {
        return Err("the disc's PARAM.SFO is not from BLJM60064 (Ryu ga Gotoku Kenzan!)".into());
    }
    if usrdir.join("EBOOT RPCS3.BIN").exists() {
        return Err("this copy already has the translation in it: install from the original, untouched ISO".into());
    }
    note(p, "Disc: BLJM60064".into());

    // 2. the translation, beside the game
    if let Some(patch) = patch {
        make_translation(patch, &patch_tmp, &tr_stage, p)?;
    } else {
        note(p, "No translation: the Japanese original.".into());
    }

    // 3. into place
    set(p, "Finishing…", 0.97);
    move_aside(&root.join("game"), p)?;
    fs::rename(&stage, root.join("game")).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(root.join("game_location.txt"));   // a game used from elsewhere until now
    if patch.is_some() {
        move_aside(&root.join("translation"), p)?;
        fs::rename(&tr_stage, root.join("translation")).map_err(|e| e.to_string())?;
    }
    let _ = crate::lang::set(root, if patch.is_some() { crate::lang::Lang::En } else { crate::lang::Lang::Jp });
    if patch_tmp.exists() {
        let _ = fs::remove_dir_all(&patch_tmp);
    }
    Ok(())
}
