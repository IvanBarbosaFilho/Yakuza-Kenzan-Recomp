//! A quick-start shortcut, made the first time the launcher runs from a bundle (the AppImage):
//! an entry in the desktop's app list (and one on the desktop when it has that folder), pointing
//! at the AppImage file itself ($APPIMAGE), with the bundle's icon copied next to the user's icons.
//! Made once: a marker in the data folder remembers it, so a deleted shortcut stays deleted.

use std::fs;
use std::path::PathBuf;

fn home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

fn data_home() -> Option<PathBuf> {
    std::env::var("XDG_DATA_HOME").ok().map(PathBuf::from).or_else(|| home().map(|h| h.join(".local/share")))
}

/// The user's desktop folder (xdg-user-dirs), if there is one.
fn desktop_dir() -> Option<PathBuf> {
    let h = home()?;
    let cfg = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| h.join(".config"));
    if let Ok(t) = fs::read_to_string(cfg.join("user-dirs.dirs")) {
        for l in t.lines() {
            if let Some(v) = l.strip_prefix("XDG_DESKTOP_DIR=") {
                let v = v.trim_matches('"').replace("$HOME", &h.display().to_string());
                let p = PathBuf::from(v);
                if p.is_dir() && p != h {
                    return Some(p);
                }
            }
        }
    }
    let d = h.join("Desktop");
    d.is_dir().then_some(d)
}

/// Writes the shortcut once. Returns a line for the window when it made one.
pub fn ensure(data: &std::path::Path) -> Option<String> {
    let appimage = std::env::var("APPIMAGE").ok()?;   // only a mounted AppImage has a stable file to point at
    let marker = data.join(".shortcut-made");
    if marker.exists() {
        return None;
    }
    let share = data_home()?;
    let apps = share.join("applications");
    let icons = share.join("icons/hicolor/256x256/apps");
    fs::create_dir_all(&apps).ok()?;
    fs::create_dir_all(&icons).ok()?;
    if let Ok(app) = std::env::var("KZ_APPDIR") {
        let _ = fs::copy(PathBuf::from(app).join("kenzan.png"), icons.join("kenzan-port.png"));
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Ryu ga Gotoku Kenzan!\nGenericName=Kenzan! — Launcher\n\
         Comment=Launcher for the Ryu ga Gotoku Kenzan! PC port\nExec=\"{appimage}\"\nIcon=kenzan-port\n\
         Terminal=false\nCategories=Game;\nStartupWMClass=kenzan-launcher\n"
    );
    fs::write(apps.join("kenzan-port.desktop"), &entry).ok()?;
    let mut where_ = vec!["in the applications menu"];
    if let Some(desk) = desktop_dir() {
        let f = desk.join("kenzan-port.desktop");
        if fs::write(&f, &entry).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&f, fs::Permissions::from_mode(0o755));
            }
            where_.push("on the desktop");
        }
    }
    let _ = fs::write(&marker, "");
    Some(format!("Shortcut created {}.", where_.join(" and ")))
}

#[cfg(test)]
mod tests {
    #[test]
    fn makes_it_once() {
        let t = std::env::temp_dir().join(format!("kzsc{}", std::process::id()));
        let home = t.join("home");
        std::fs::create_dir_all(home.join("Desktop")).unwrap();
        let data = t.join("data");
        std::fs::create_dir_all(&data).unwrap();
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::set_var("APPIMAGE", "/opt/Kenzan-x86_64.AppImage");
        }
        let m = super::ensure(&data).expect("made");
        assert!(m.contains("desktop"), "{m}");
        let e = std::fs::read_to_string(home.join(".local/share/applications/kenzan-port.desktop")).unwrap();
        assert!(e.contains("Exec=\"/opt/Kenzan-x86_64.AppImage\""), "{e}");
        assert!(home.join("Desktop/kenzan-port.desktop").exists());
        assert!(super::ensure(&data).is_none(), "only once");
        let _ = std::fs::remove_dir_all(&t);
    }
}
