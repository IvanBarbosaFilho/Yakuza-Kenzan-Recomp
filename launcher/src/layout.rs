//! Where everything is, in the two ways the launcher runs.
//!
//! Development (no KZ_APPDIR): the kenzan-lle checkout; Play runs run.sh, which enters the
//! game's nix-shell. Bundle (the AppImage's AppRun sets KZ_APPDIR): the port's files are read-only
//! under $KZ_APPDIR/usr; the player's (kenzan.ini, the installed game, saves, logs, caches) live in
//! $XDG_DATA_HOME/kenzan-port; Play starts KenzanRecomp itself, through the bundle's own dynamic
//! loader and libraries (KZ_LDSO / KZ_LIBPATH), with the environment run.sh would have set.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone)]
pub struct Layout {
    /// The bundle's root (AppDir), if running from one.
    pub app: Option<PathBuf>,
    /// Writable root: kenzan.ini and game/ live here.
    pub data: PathBuf,
    /// The engine's working directory (saves under gamedata/, logs/, hdd0/ ...).
    pub work: PathBuf,
    pub exe: PathBuf,
    /// Folder with libInfamous2Game{,_60,_120}.so and EBOOT_{60,120}.elf.
    pub modules: PathBuf,
}

impl Layout {
    pub fn detect(dev_root: PathBuf) -> Layout {
        // Windows: a portable folder. The launcher's own folder holds kenzan.ini, game/ and work/;
        // engine/ next to it holds KenzanRecomp.exe, SDL2.dll, the game modules and EBOOT_{60,120}.elf.
        #[cfg(windows)]
        {
            let _ = &dev_root;
            let root = std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));
            let eng = root.join("engine");
            let _ = fs::create_dir_all(root.join("work"));
            return Layout {
                exe: eng.join("KenzanRecomp.exe"),
                modules: eng.clone(),
                work: root.join("work"),
                data: root,
                app: Some(eng),
            };
        }
        #[allow(unreachable_code)]
        if let Ok(app) = std::env::var("KZ_APPDIR") {
            let app = PathBuf::from(app);
            let data = std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("kenzan-port");
            let _ = fs::create_dir_all(data.join("work"));
            return Layout {
                exe: app.join("usr/bin/KenzanRecomp"),
                modules: app.join("usr/share/kenzan"),
                work: data.join("work"),
                data,
                app: Some(app),
            };
        }
        Layout {
            app: None,
            work: dev_root.join("work"),
            exe: dev_root.join("work/build/KenzanRecomp"),
            modules: dev_root.join("work/build"),
            data: dev_root,
        }
    }

    pub fn game(&self) -> PathBuf {
        crate::import::game_dir(&self.data)
    }

    /// The module and the executable to boot for a frame rate (30 = the shipped game).
    pub fn mode(&self, fps: &str) -> (PathBuf, PathBuf) {
        let ebootdir = if self.app.is_some() { self.modules.clone() } else { self.work.clone() };
        #[cfg(windows)]
        let (m30, m60, m120) = ("KenzanGame.dll", "KenzanGame_60.dll", "KenzanGame_120.dll");
        #[cfg(not(windows))]
        let (m30, m60, m120) = ("libInfamous2Game.so", "libInfamous2Game_60.so", "libInfamous2Game_120.so");
        match fps {
            "120" => (self.modules.join(m120), ebootdir.join("EBOOT_120.elf")),
            "60" => (self.modules.join(m60), ebootdir.join("EBOOT_60.elf")),
            _ => (self.modules.join(m30), self.game().join("PS3_GAME/USRDIR/EBOOT.elf")),
        }
    }

    /// The mods folder: one folder per mod, whose files replace the game's while playing
    /// (engine runtime/ppu/ppu_fs_loose_mods.h); the game's own files are never changed.
    pub fn mods(&self) -> PathBuf {
        self.data.join("mods")
    }

    /// The controls file (input.toml), shared with the in-game menu's Controls page.
    pub fn input_file(&self) -> PathBuf {
        self.data.join("input.toml")
    }

    pub fn saves(&self) -> PathBuf {
        self.work.join("gamedata/dev_hdd0/home/00000001/savedata")
    }

    pub fn last_log(&self) -> PathBuf {
        self.work.join("logs/last_run_err.txt")
    }

    /// The command that plays the game with `env` (the launcher's settings) applied.
    pub fn play_command(&self, fps: &str, env: &[(String, String)]) -> Result<Command, String> {
        let Some(app) = &self.app else {
            let mut cmd = Command::new("bash");
            cmd.arg(self.data.join("run.sh")).current_dir(&self.data);
            // the launcher runs inside its own nix-shell (GUI libraries only): run.sh must enter the
            // game's shell.nix itself, so it must not see this one as "already in a nix-shell"
            cmd.env_remove("IN_NIX_SHELL").env_remove("LD_LIBRARY_PATH");
            let _ = fs::create_dir_all(self.mods());
            cmd.env("PS3_LOOSE_MODS_DIR", self.mods());
            for (k, v) in env {
                cmd.env(k, v);
            }
            return Ok(cmd);
        };
        let (module, eboot) = self.mode(fps);
        let w = &self.work;
        for d in ["hdd0", "hdd1", "no_flash", "logs/player"] {
            fs::create_dir_all(w.join(d)).map_err(|e| e.to_string())?;
        }
        let _ = fs::remove_dir_all(w.join("hdd1/cache"));
        #[cfg(windows)]
        let cache = w.join("cache/spirv");
        #[cfg(not(windows))]
        let cache = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(|_| w.join("cache"))
            .join("kenzan-port/spirv");
        let _ = fs::create_dir_all(&cache);
        let out = fs::File::create(w.join("logs/last_run_out.txt")).map_err(|e| e.to_string())?;
        let err = fs::File::create(w.join("logs/last_run_err.txt")).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        let mut cmd = {
            use std::os::windows::process::CommandExt;
            let _ = app;
            let mut c = Command::new(&self.exe);
            c.arg(&eboot).creation_flags(0x0800_0000);   // CREATE_NO_WINDOW: no console behind the game
            c
        };
        // through the bundle's loader: its glibc and libraries, the host's GPU drivers
        #[cfg(not(windows))]
        let ldso = std::env::var("KZ_LDSO").unwrap_or_else(|_| app.join("usr/lib/ld-linux-x86-64.so.2").display().to_string());
        #[cfg(not(windows))]
        let libpath = std::env::var("KZ_LIBPATH").unwrap_or_else(|_| app.join("usr/lib").display().to_string());
        #[cfg(not(windows))]
        let mut cmd = Command::new(ldso);
        #[cfg(not(windows))]
        cmd.arg("--library-path").arg(libpath).arg(&self.exe).arg(&eboot);
        cmd.current_dir(w).stdout(Stdio::from(out)).stderr(Stdio::from(err));
        let p = |x: &Path| x.display().to_string();
        let fixed: Vec<(&str, String)> = vec![
            ("PS3_VFS_ROOT", p(&self.game())),
            ("PS3_GAME_MODULE", p(&module)),
            ("PS3_TITLE", "Ryu ga Gotoku Kenzan!".into()),
            ("PS3_HDD0_ROOT", p(&w.join("hdd0"))),
            ("PS3_HDD1_ROOT", p(&w.join("hdd1"))),
            ("PS3_DEV_FLASH", p(&w.join("no_flash"))),
            ("PS3_NP_STRICT_OFFLINE", "1".into()),
            ("GCM_SEMA_ACQUIRE", "1".into()),
            ("GCM_SEMA_ACQUIRE_MS", "50".into()),
            ("PS3_RUMBLE", "0".into()),
            ("GCM_NO_TITLE_RECYCLE", "1".into()),
            ("KZ_FLIPSEMA", "1".into()),
            ("KZ_VSYNC", "1".into()),
            ("PS3RECOMP_VP_PRECISE", "1".into()),
            ("PS3_TOGGLE_FILE", p(&w.join("toggles.txt"))),
            ("PS3_NATIVE_JOBS", "0".into()),
            ("PS3_NATIVE_CULL", "0".into()),
            ("PS3_NATIVE_ANIM", "0".into()),
            ("KZ_FLIPMARK", "1".into()),
            // the watch thread also expires stale reservation marks (the festival's 47 -> 57 fps)
            ("KZ_WATCHR_TRIGGER", p(&w.join("kzwatchr_now"))),
            ("PS3RECOMP_VK_WINDOW", "1".into()),
            ("PS3RECOMP_VK_GUEST_PROGRAMS", "1".into()),
            ("PS3RECOMP_SPIRV_CACHE", p(&cache)),
            ("PS3_VERBOSE", "0".into()),
            ("KZ_INI", p(&self.data.join("kenzan.ini"))),
            ("PS3_INPUT_MAP", p(&self.input_file())),
            ("KZ_FPS", fps.to_string()),
            ("PS3_LOOSE_MODS_DIR", p(&self.mods())),
            // the in-game menu's About / Update / Display pages (engine vk_overlay_menu.inc)
            ("PS3_PORT_NAME", "Ryu ga Gotoku Kenzan! PC port".into()),
            ("PS3_PORT_BY", "by IvanBarbosaFilho".into()),
            ("PS3_PORT_URL", "https://github.com/IvanBarbosaFilho/Yakuza-Kenzan-Recomp".into()),
            ("PS3_PORT_VERSION", format!("v{}", env!("CARGO_PKG_VERSION"))),
            ("PS3_UPDATE_BY", "launcher".into()),
            ("PS3_ASPECT_UNSUPPORTED", "1".into()),
            // shaders translated before release (engine vk_sc_seed_import): made by
            // tools/make_shader_seed.py, shipped beside the game modules
            ("PS3RECOMP_VK_SHADER_SEED", p(&self.modules.join("shader_seed"))),
        ];
        let _ = fs::create_dir_all(self.mods());
        for (k, v) in fixed {
            cmd.env(k, v);
        }
        if fps == "120" {
            cmd.env("KZ_VBLANK_HZ", "120");
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        Ok(cmd)
    }

    /// After a bundle session: keep a copy of its logs, as run.sh does.
    pub fn archive_logs(&self) {
        if self.app.is_none() {
            return;
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dir = self.work.join("logs/player");
        let _ = fs::copy(self.work.join("logs/last_run_err.txt"), dir.join(format!("{secs}-err.txt")));
        let _ = fs::copy(self.work.join("logs/last_run_out.txt"), dir.join(format!("{secs}-out.txt")));
    }
}
