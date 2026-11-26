//! A newer release on GitHub: found when the launcher starts, installed when the player says so.
//!
//! The check asks the GitHub API for the latest release of the port's repository (curl: in every
//! Windows 10/11 and nearly every Linux desktop; no curl = no check, the launcher opens as usual).
//! A release is newer when its tag (V1.0.1, v1.2) is above this launcher's version. Installing
//! downloads that release's file for this system, checks it against the SHA-256 GitHub lists for
//! it, and puts it in place:
//!   - Linux (the AppImage): the new AppImage replaces the running one ($APPIMAGE), then starts.
//!   - Windows (the portable folder): the zip is unpacked beside the launcher (7-Zip, as Install
//!     uses), the running "Kenzan Launcher.exe" is renamed to .old (Windows lets a running program
//!     be renamed, not overwritten), and the new files are copied over the port's own: the
//!     launcher and engine/. The game, saves, settings, controls and mods are not touched.
//! Nothing runs in a development checkout (no bundle): it updates with git.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const API: &str = "https://api.github.com/repos/IvanBarbosaFilho/Yakuza-Kenzan-Recomp/releases/latest";
pub const RELEASES: &str = "https://github.com/IvanBarbosaFilho/Yakuza-Kenzan-Recomp/releases";

/// The release file of this system and edition: the Japanese edition (it has the Japanese
/// original's files, lang.rs) updates to the Japanese edition, the normal one to the normal one.
pub fn asset(japanese: bool) -> &'static str {
    match (cfg!(windows), japanese) {
        (true, false) => "Kenzan-Windows.zip",
        (true, true) => "Kenzan-Windows-Japanese.zip",
        (false, false) => "Kenzan-x86_64.AppImage",
        (false, true) => "Kenzan-Japanese-x86_64.AppImage",
    }
}

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub notes: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// "V1.0.1" / "v1.2" / "1.0" -> [1, 0, 1]
fn parse_version(s: &str) -> Vec<u64> {
    s.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map_while(|p| p.parse::<u64>().ok()).collect()
}

pub fn newer(tag: &str, current: &str) -> bool {
    let (a, b) = (parse_version(tag), parse_version(current));
    if a.is_empty() {
        return false;
    }
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

fn curl() -> Command {
    let mut c = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.arg("-sSfL").arg("-A").arg(format!("kenzan-launcher/{VERSION}"));
    c
}

/// This launcher's version; KZ_UPDATE_AS=<version> pretends to be another (to test an update).
pub fn current() -> String {
    std::env::var("KZ_UPDATE_AS").unwrap_or_else(|_| VERSION.to_string())
}

/// The latest release, when it is newer than this launcher and has a file for this system.
pub fn check(japanese: bool) -> Result<Option<Release>, String> {
    let name = asset(japanese);
    // KZ_UPDATE_RELEASE_JSON=<file> (tests): the API's answer read from a file, not asked for
    let body = if let Ok(f) = std::env::var("KZ_UPDATE_RELEASE_JSON") {
        std::fs::read(&f).map_err(|e| format!("{f}: {e}"))?
    } else {
        let out = curl()
            .args(["--max-time", "15", "-H", "Accept: application/vnd.github+json", API])
            .stderr(Stdio::null())
            .output()
            .map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() {
            return Err("GitHub did not answer".into());
        }
        out.stdout
    };
    let j: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let tag = j["tag_name"].as_str().unwrap_or("").to_string();
    if j["draft"].as_bool() == Some(true) || j["prerelease"].as_bool() == Some(true) || !newer(&tag, &current()) {
        return Ok(None);
    }
    let Some(asset) = j["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(name))) else {
        return Ok(None);
    };
    Ok(Some(Release {
        name: j["name"].as_str().unwrap_or(&tag).to_string(),
        notes: j["body"].as_str().unwrap_or("").trim().to_string(),
        url: asset["browser_download_url"].as_str().unwrap_or("").to_string(),
        sha256: asset["digest"].as_str().unwrap_or("").trim_start_matches("sha256:").to_lowercase(),
        size: asset["size"].as_u64().unwrap_or(0),
        tag,
    }))
}

fn sha256_file(p: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Download `r` into `dest`, reporting "Downloading… N%" through `tx`.
fn download(r: &Release, dest: &Path, tx: &Sender<String>) -> Result<(), String> {
    let _ = std::fs::remove_file(dest);
    // KZ_UPDATE_LOCAL_DIR=<dir> (tests): the release file taken from that folder (by the name at the
    // end of its URL) instead of downloaded; the SHA-256 check below still runs
    if let Ok(dir) = std::env::var("KZ_UPDATE_LOCAL_DIR") {
        let file = r.url.rsplit('/').next().unwrap_or("");
        let src = Path::new(&dir).join(file);
        let _ = tx.send(format!("Copying {} (local test)…", src.display()));
        std::fs::copy(&src, dest).map_err(|e| format!("{}: {e}", src.display()))?;
    } else {
    let mut child = curl()
        .args(["--retry", "2", "-o"])
        .arg(dest)
        .arg(&r.url)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            if !st.success() {
                return Err("The download failed. Check your connection and try again.".into());
            }
            break;
        }
        let got = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        let msg = if r.size > 0 {
            format!("Downloading {}… {}%", r.tag, (got * 100 / r.size).min(100))
        } else {
            format!("Downloading {}… {} MB", r.tag, got >> 20)
        };
        let _ = tx.send(msg);
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    }
    if !r.sha256.is_empty() {
        let _ = tx.send("Checking the download…".into());
        let h = sha256_file(dest)?;
        if h != r.sha256 {
            let _ = std::fs::remove_file(dest);
            return Err("The download is damaged (its SHA-256 does not match GitHub's). Try again.".into());
        }
    }
    Ok(())
}

/// Remove what an earlier update left behind (the old launcher on Windows, the old AppImage).
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
    if let Ok(a) = std::env::var("APPIMAGE") {
        let _ = std::fs::remove_file(format!("{a}.old"));
        let _ = std::fs::remove_file(format!("{a}.download"));
    }
}

/// Download and put the release in place. On success the new launcher has been started and the
/// caller exits.
pub fn install(r: &Release, tx: &Sender<String>) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let app = std::env::var("APPIMAGE").map_err(|_| "Not running from the AppImage: download it from GitHub.".to_string())?;
        let app = PathBuf::from(app);
        let tmp = PathBuf::from(format!("{}.download", app.display()));
        download(r, &tmp, tx)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        let old = PathBuf::from(format!("{}.old", app.display()));
        std::fs::rename(&app, &old).map_err(|e| format!("Cannot replace {}: {e}", app.display()))?;
        if let Err(e) = std::fs::rename(&tmp, &app) {
            let _ = std::fs::rename(&old, &app);
            return Err(format!("Cannot replace {}: {e}", app.display()));
        }
        let _ = tx.send("Starting the new version…".into());
        if std::env::var_os("KZ_UPDATE_NO_START").is_none() {
            Command::new(&app).spawn().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let root = exe.parent().ok_or("no folder")?.to_path_buf();
        let zip = root.join("work").join("update.zip");
        let _ = std::fs::create_dir_all(root.join("work"));
        download(r, &zip, tx)?;
        let _ = tx.send("Unpacking…".into());
        let stage = root.join("work").join("update");
        let _ = std::fs::remove_dir_all(&stage);
        let sz = crate::install::seven_zip()?;
        let mut c = Command::new(sz);
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        let st = c.arg("x").arg("-y").arg(format!("-o{}", stage.display())).arg(&zip)
            .stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| e.to_string())?;
        if !st.success() {
            return Err("The update could not be unpacked.".into());
        }
        // the zip holds one folder (Kenzan-Windows) with the launcher and engine/ in it
        let inner = std::fs::read_dir(&stage).map_err(|e| e.to_string())?.flatten()
            .find(|e| e.path().join("engine").is_dir()).map(|e| e.path()).unwrap_or(stage.clone());
        let new_exe = std::fs::read_dir(&inner).map_err(|e| e.to_string())?.flatten().map(|e| e.path())
            .find(|p| p.extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false))
            .ok_or("The update has no launcher in it.")?;
        let _ = tx.send("Installing…".into());
        let old = exe.with_extension("exe.old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old).map_err(|e| format!("Cannot replace the launcher: {e}"))?;
        let dst_exe = root.join(new_exe.file_name().unwrap());
        if let Err(e) = std::fs::copy(&new_exe, &dst_exe) {
            let _ = std::fs::rename(&old, &exe);
            return Err(format!("Cannot copy the new launcher: {e}"));
        }
        copy_tree(&inner.join("engine"), &root.join("engine"))?;
        for f in ["README.md"] {
            let _ = std::fs::copy(inner.join(f), root.join(f));
        }
        if inner.join("licenses").is_dir() {
            let _ = copy_tree(&inner.join("licenses"), &root.join("licenses"));
        }
        let _ = std::fs::remove_dir_all(&stage);
        let _ = std::fs::remove_file(&zip);
        let _ = tx.send("Starting the new version…".into());
        if std::env::var_os("KZ_UPDATE_NO_START").is_none() {
            Command::new(&dst_exe).current_dir(&root).spawn().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(windows)]
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let p = e.path();
        let d = dst.join(e.file_name());
        if p.is_dir() {
            copy_tree(&p, &d)?;
        } else {
            std::fs::copy(&p, &d).map_err(|e| format!("Cannot write {}: {e}", d.display()))?;
        }
    }
    Ok(())
}

/// The player's "Skip this version": the tag, in <data>/update_skip.txt.
pub fn skipped(data: &Path) -> String {
    std::fs::read_to_string(data.join("update_skip.txt")).unwrap_or_default().trim().to_string()
}
pub fn skip(data: &Path, tag: &str) {
    let _ = std::fs::write(data.join("update_skip.txt"), tag);
}

#[allow(dead_code)]
pub fn data_dir_hint(p: &Path) -> PathBuf {
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        assert!(newer("V1.0.1", "1.0.0"));
        assert!(newer("v1.1", "1.0.9"));
        assert!(!newer("V1.0.0", "1.0.0"));
        assert!(!newer("V1.0.0", "1.0.1"));
        assert!(!newer("beta", "1.0.0"));
        assert!(newer("V2", "1.9.9"));
    }

    /// KZ_UPDATE_E2E=1 (network, ~400 MB): pretend to be 0.9, find the latest release, install it
    /// over a stand-in AppImage, check the result against GitHub's SHA-256.
    #[cfg(not(windows))]
    #[test]
    fn e2e_appimage() {
        if std::env::var_os("KZ_UPDATE_E2E").is_none() { return; }
        std::env::set_var("KZ_UPDATE_AS", "0.9");
        std::env::set_var("KZ_UPDATE_NO_START", "1");
        let t = std::env::temp_dir().join(format!("kz-upd-{}", std::process::id()));
        std::fs::create_dir_all(&t).unwrap();
        let app = t.join("Kenzan-x86_64.AppImage");
        std::fs::write(&app, b"old").unwrap();
        std::env::set_var("APPIMAGE", &app);
        let r = check(false).unwrap().expect("a newer release");
        println!("found {} {} bytes sha {}", r.tag, r.size, r.sha256);
        let (tx, rx) = std::sync::mpsc::channel();
        install(&r, &tx).unwrap();
        let last: Vec<String> = rx.try_iter().collect();
        println!("last: {:?}", last.last());
        assert_eq!(sha256_file(&app).unwrap(), r.sha256);
        assert!(std::fs::read(format!("{}.old", app.display())).unwrap() == b"old");
        cleanup();
        assert!(!std::path::Path::new(&format!("{}.old", app.display())).exists());
        std::fs::remove_dir_all(&t).unwrap();
    }
}
