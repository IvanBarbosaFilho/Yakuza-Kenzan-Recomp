//! The launcher's settings: kept in `kenzan.ini` (one `key=value` per line) next to `run.sh`,
//! and handed to the game as the environment variables `run.sh` and the engine read.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One choice of a list setting: what is stored, what the player reads.
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

pub const RESOLUTIONS: &[Choice] = &[
    Choice { value: "720p", label: "720p (PS3 original)" },
    Choice { value: "1080p", label: "1080p" },
    Choice { value: "1440p", label: "1440p" },
    Choice { value: "4k", label: "4K" },
];
pub const DISPLAY_MODES: &[Choice] = &[
    Choice { value: "window", label: "Windowed" },
    Choice { value: "fullscreen", label: "Fullscreen" },
];
pub const PRESENT_MODES: &[Choice] = &[
    Choice { value: "mailbox", label: "No tearing, low latency (default)" },
    Choice { value: "fifo", label: "Classic VSync" },
    Choice { value: "immediate", label: "Immediate (may tear)" },
];
pub const AA_MODES: &[Choice] = &[
    Choice { value: "smaa", label: "SMAA" },
    Choice { value: "fxaa", label: "FXAA" },
    Choice { value: "off", label: "Off" },
];
pub const MSAA_MODES: &[Choice] = &[
    Choice { value: "off", label: "Off" },
    Choice { value: "2", label: "2x" },
    Choice { value: "4", label: "4x" },
];
pub const SHADOW_MODES: &[Choice] = &[
    Choice { value: "0", label: "Original" },
    Choice { value: "1", label: "High (2x map)" },
];
pub const FPS_MODES: &[Choice] = &[
    Choice { value: "30", label: "30 FPS (original)" },
    Choice { value: "60", label: "60 FPS (recommended)" },
    Choice { value: "120", label: "120 FPS" },
];
pub const RUMBLE_MODES: &[Choice] = &[
    Choice { value: "0", label: "Off" },
    Choice { value: "1", label: "On (experimental)" },
];
pub const INPUT_MODES: &[Choice] = &[
    Choice { value: "both", label: "Controller, keyboard and mouse" },
    Choice { value: "controller", label: "Controller only" },
    Choice { value: "keyboard", label: "Keyboard and mouse only" },
];
pub const ANISO_MODES: &[Choice] = &[
    Choice { value: "1", label: "Off" },
    Choice { value: "4", label: "4x" },
    Choice { value: "8", label: "8x" },
    Choice { value: "16", label: "16x" },
];

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub resolution: String,
    pub display_mode: String,
    pub present: String,
    pub aa: String,
    pub msaa: String,
    pub shadows: String,
    pub sharpen: f32,
    pub aniso: String,
    pub fps: String,
    pub rumble: String,
    pub input_mode: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            resolution: "1080p".into(),
            display_mode: "window".into(),
            present: "mailbox".into(),
            aa: "smaa".into(),
            msaa: "off".into(),
            shadows: "1".into(),
            sharpen: 0.3,
            aniso: "16".into(),
            fps: "60".into(),
            rumble: "0".into(),
            input_mode: "both".into(),
        }
    }
}

/// A named group of settings for the Performance page.
pub struct Preset {
    pub name: &'static str,
    pub about: &'static str,
    pub apply: fn(&mut Settings),
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Low",
        about: "For modest PCs: 720p, original shadows, 30 FPS.",
        apply: |s| {
            s.resolution = "720p".into();
            s.shadows = "0".into();
            s.msaa = "off".into();
            s.aa = "fxaa".into();
            s.fps = "30".into();
        },
    },
    Preset {
        name: "Standard",
        about: "The tested balance: 1080p, high shadows, 60 FPS.",
        apply: |s| {
            s.resolution = "1080p".into();
            s.shadows = "1".into();
            s.msaa = "off".into();
            s.aa = "smaa".into();
            s.fps = "60".into();
        },
    },
    Preset {
        name: "High",
        about: "For powerful PCs: 1440p, high shadows, 2x MSAA, 60 FPS.",
        apply: |s| {
            s.resolution = "1440p".into();
            s.shadows = "1".into();
            s.msaa = "2".into();
            s.aa = "smaa".into();
            s.fps = "60".into();
        },
    },
];

pub fn ini_path(root: &Path) -> PathBuf {
    root.join("kenzan.ini")
}

impl Settings {
    pub fn load(root: &Path) -> Settings {
        let mut s = Settings::default();
        let Ok(text) = fs::read_to_string(ini_path(root)) else { return s };
        let kv: BTreeMap<String, String> = text
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() || l.starts_with('#') || l.starts_with('[') {
                    return None;
                }
                let (k, v) = l.split_once('=')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        let pick = |key: &str, list: &[Choice], cur: &mut String| {
            if let Some(v) = kv.get(key) {
                if list.iter().any(|c| c.value == v) {
                    *cur = v.clone();
                }
            }
        };
        pick("resolution", RESOLUTIONS, &mut s.resolution);
        pick("display_mode", DISPLAY_MODES, &mut s.display_mode);
        pick("present", PRESENT_MODES, &mut s.present);
        pick("aa", AA_MODES, &mut s.aa);
        pick("msaa", MSAA_MODES, &mut s.msaa);
        pick("shadows", SHADOW_MODES, &mut s.shadows);
        pick("aniso", ANISO_MODES, &mut s.aniso);
        pick("fps", FPS_MODES, &mut s.fps);
        pick("rumble", RUMBLE_MODES, &mut s.rumble);
        pick("input_mode", INPUT_MODES, &mut s.input_mode);
        if let Some(v) = kv.get("sharpen").and_then(|v| v.parse::<f32>().ok()) {
            s.sharpen = v.clamp(0.0, 1.0);
        }
        s
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let text = format!(
            "# Ryu ga Gotoku Kenzan! PC port -- written by the launcher\n\
             [graphics]\n\
             resolution={}\ndisplay_mode={}\npresent={}\naa={}\nmsaa={}\nshadows={}\nsharpen={:.2}\naniso={}\n\
             [performance]\n\
             fps={}\n\
             [controls]\n\
             rumble={}\ninput_mode={}\n",
            self.resolution, self.display_mode, self.present, self.aa, self.msaa, self.shadows,
            self.sharpen, self.aniso, self.fps, self.rumble, self.input_mode
        );
        fs::write(ini_path(root), text)
    }

    /// The environment the game reads (run.sh keeps its own default for anything not set here).
    pub fn env(&self) -> Vec<(String, String)> {
        let mut e: Vec<(String, String)> = vec![
            ("KZ_FPS".into(), self.fps.clone()),
            ("PS3RECOMP_RENDER_SCALE".into(), self.resolution.clone()),
            ("PS3RECOMP_VK_PRESENT".into(), self.present.clone()),
            ("PS3RECOMP_AA".into(), self.aa.clone()),
            ("PS3RECOMP_SHADOW_RES".into(), self.shadows.clone()),
            ("PS3RECOMP_SHARPEN".into(), format!("{:.2}", self.sharpen)),
            ("PS3RECOMP_ANISO".into(), self.aniso.clone()),
        ];
        e.push(("PS3_RUMBLE".into(), self.rumble.clone()));
        if self.input_mode != "both" {
            e.push(("PS3_INPUT_MODE".into(), self.input_mode.clone()));
        }
        if self.msaa != "off" {
            e.push(("PS3RECOMP_MSAA".into(), self.msaa.clone()));
        }
        if self.display_mode == "fullscreen" {
            e.push(("PS3RECOMP_VK_FULLSCREEN".into(), "1".into()));
        }
        e
    }
}

pub fn label_of(list: &[Choice], value: &str) -> &'static str {
    list.iter().find(|c| c.value == value).map(|c| c.label).unwrap_or("?")
}
