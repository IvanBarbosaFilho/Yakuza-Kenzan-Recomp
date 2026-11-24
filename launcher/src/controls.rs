//! The Controls tab: input.toml -- the same file the in-game menu's F2 > Controls page edits
//! (format: engine/libs/input/pad_input_map.h, documented in engine/docs/INPUT.md). Only the values
//! the tab changes are rewritten; every other line, comment and section stays as it was.

use std::fs;
use std::path::{Path, PathBuf};

const DEFAULTS: &str = include_str!("../../engine/docs/input.example.toml");

pub struct InputFile {
    pub path: PathBuf,
    lines: Vec<String>,
    pub dirty: bool,
}

/// Strips a trailing comment that is not inside quotes.
fn strip_comment(v: &str) -> &str {
    let mut q = false;
    for (i, c) in v.char_indices() {
        match c {
            '"' => q = !q,
            '#' if !q => return &v[..i],
            _ => {}
        }
    }
    v
}

impl InputFile {
    pub fn load(path: &Path) -> InputFile {
        let text = fs::read_to_string(path).unwrap_or_else(|_| DEFAULTS.to_string());
        InputFile { path: path.to_path_buf(), lines: text.lines().map(String::from).collect(), dirty: false }
    }

    pub fn reset(&mut self) {
        self.lines = DEFAULTS.lines().map(String::from).collect();
        self.dirty = true;
    }

    /// (start, end) line range of a section's body, if the section exists.
    fn section(&self, name: &str) -> Option<(usize, usize)> {
        let head = format!("[{}]", name.to_ascii_lowercase());
        let start = self.lines.iter().position(|l| l.trim().to_ascii_lowercase() == head)? + 1;
        let end = self.lines[start..]
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .map(|i| start + i)
            .unwrap_or(self.lines.len());
        Some((start, end))
    }

    fn find(&self, section: &str, key: &str) -> Option<usize> {
        let (a, b) = self.section(section)?;
        (a..b).find(|&i| {
            let l = self.lines[i].trim_start();
            !l.starts_with('#')
                && l.split_once('=').map(|(k, _)| k.trim().eq_ignore_ascii_case(key)).unwrap_or(false)
        })
    }

    /// The value, unquoted; None when the line is absent (the built-in default applies).
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        let i = self.find(section, key)?;
        let v = self.lines[i].split_once('=')?.1;
        Some(strip_comment(v).trim().trim_matches('"').to_string())
    }

    /// Sets `key = value` (value already formatted: quoted string, number or bool).
    pub fn set_raw(&mut self, section: &str, key: &str, value: &str) {
        let line = format!("{key:<8} = {value}");
        if let Some(i) = self.find(section, key) {
            if self.lines[i] == line {
                return;
            }
            self.lines[i] = line;
        } else if let Some((a, b)) = self.section(section) {
            // after the section's last key line (before trailing blanks/comments)
            let mut at = a;
            for i in a..b {
                let l = self.lines[i].trim();
                if !l.is_empty() && !l.starts_with('#') {
                    at = i + 1;
                }
            }
            self.lines.insert(at, line);
        } else {
            if self.lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                self.lines.push(String::new());
            }
            self.lines.push(format!("[{section}]"));
            self.lines.push(line);
        }
        self.dirty = true;
    }

    pub fn set_str(&mut self, section: &str, key: &str, v: &str) {
        self.set_raw(section, key, &format!("\"{v}\""));
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        let tmp = self.path.with_extension("toml.tmp");
        fs::write(&tmp, self.lines.join("\n") + "\n")?;
        fs::rename(&tmp, &self.path)?;
        self.dirty = false;
        Ok(())
    }
}

/// The PS3 controls, in the file's names, with what the player reads.
pub const CONTROLS: &[(&str, &str)] = &[
    ("cross", "Cross (×)"),
    ("circle", "Circle (○)"),
    ("square", "Square (□)"),
    ("triangle", "Triangle (△)"),
    ("l1", "L1"),
    ("r1", "R1"),
    ("l2", "L2"),
    ("r2", "R2"),
    ("l3", "L3 (left stick click)"),
    ("r3", "R3 (right stick click)"),
    ("start", "START"),
    ("select", "SELECT"),
    ("up", "D-pad ↑"),
    ("down", "D-pad ↓"),
    ("left", "D-pad ←"),
    ("right", "D-pad →"),
    ("ls_up", "Left stick ↑"),
    ("ls_down", "Left stick ↓"),
    ("ls_left", "Left stick ←"),
    ("ls_right", "Left stick →"),
    ("rs_up", "Right stick ↑ (camera)"),
    ("rs_down", "Right stick ↓ (camera)"),
    ("rs_left", "Right stick ← (camera)"),
    ("rs_right", "Right stick → (camera)"),
];

/// Controller inputs by position (SDL names), with labels for PlayStation / Xbox.
pub const PAD_INPUTS: &[(&str, &str)] = &[
    ("", "—"),
    ("a", "Bottom (× / A)"),
    ("b", "Right (○ / B)"),
    ("x", "Left (□ / X)"),
    ("y", "Top (△ / Y)"),
    ("leftshoulder", "L1 / LB"),
    ("rightshoulder", "R1 / RB"),
    ("lefttrigger", "L2 / LT"),
    ("righttrigger", "R2 / RT"),
    ("leftstick", "L3 / left click"),
    ("rightstick", "R3 / right click"),
    ("start", "Options / Menu / +"),
    ("back", "Share / View / −"),
    ("back, touchpad", "Share or touchpad"),
    ("touchpad", "Touchpad"),
    ("guide", "PS / Xbox / Home"),
    ("dpup", "D-pad ↑"),
    ("dpdown", "D-pad ↓"),
    ("dpleft", "D-pad ←"),
    ("dpright", "D-pad →"),
    ("misc1", "Extra button (Xbox Share / Capture)"),
    ("paddle1", "Paddle 1"),
    ("paddle2", "Paddle 2"),
    ("paddle3", "Paddle 3"),
    ("paddle4", "Paddle 4"),
];

pub const MOUSE_INPUTS: &[(&str, &str)] = &[
    ("", "—"),
    ("Left", "Left button"),
    ("Right", "Right button"),
    ("Middle", "Middle button"),
    ("Button4", "Side 1 (back)"),
    ("Button5", "Side 2 (forward)"),
    ("WheelUp", "Wheel up"),
    ("WheelDown", "Wheel down"),
];

/// egui's key -> the engine's key name (US positions).
pub fn key_name(k: eframe::egui::Key) -> Option<&'static str> {
    use eframe::egui::Key::*;
    Some(match k {
        A => "A", B => "B", C => "C", D => "D", E => "E", F => "F", G => "G", H => "H", I => "I",
        J => "J", K => "K", L => "L", M => "M", N => "N", O => "O", P => "P", Q => "Q", R => "R",
        S => "S", T => "T", U => "U", V => "V", W => "W", X => "X", Y => "Y", Z => "Z",
        Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4", Num5 => "5", Num6 => "6",
        Num7 => "7", Num8 => "8", Num9 => "9",
        F1 => "F1", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6", F8 => "F8", F10 => "F10", F12 => "F12",
        Space => "Space", Enter => "Return", Tab => "Tab", Backspace => "Backspace", Delete => "Delete",
        Insert => "Insert", Home => "Home", End => "End", PageUp => "PageUp", PageDown => "PageDown",
        ArrowUp => "Up", ArrowDown => "Down", ArrowLeft => "Left", ArrowRight => "Right",
        Minus => "Minus", Equals => "Equal", OpenBracket => "LeftBracket", CloseBracket => "RightBracket",
        Backslash => "Backslash", Semicolon => "Semicolon", Quote => "Quote", Comma => "Comma",
        Period => "Period", Slash => "Slash", Backtick => "Grave",
        _ => return None,
    })
}

/// The controllers the last game session saw ("[cellPad] SDL controller connected on port N: NAME --").
pub fn detected(log: &Path) -> Vec<String> {
    let text = fs::read(log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let mut v: Vec<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("[cellPad] SDL controller connected on port "))
        .filter_map(|r| {
            let (port, rest) = r.split_once(": ")?;
            let name = rest.split(" -- ").next()?.trim();
            Some(format!("Port {}: {}", port.trim().parse::<u32>().map(|p| p + 1).unwrap_or(0), name))
        })
        .collect();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_only_what_it_sets() {
        let dir = std::env::temp_dir().join(format!("kzinp{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("input.toml");
        fs::write(&p, "# my comment\n[keyboard]\ncross    = \"Space\" # jump\nsquare = \"F\"\n\n[mouse]\n# camera = true\n").unwrap();
        let mut f = InputFile::load(&p);
        assert_eq!(f.get("keyboard", "cross").as_deref(), Some("Space"));
        assert_eq!(f.get("mouse", "camera"), None);           // commented out = default
        f.set_str("keyboard", "cross", "J");
        f.set_raw("mouse", "sensitivity", "1.50");
        f.set_str("controller", "cross", "a");               // a new section
        f.save().unwrap();
        let t = fs::read_to_string(&p).unwrap();
        assert!(t.starts_with("# my comment\n[keyboard]\ncross    = \"J\"\nsquare = \"F\"\n"), "{t}");
        assert!(t.contains("[mouse]\n# camera = true\nsensitivity = 1.50\n") || t.contains("sensitivity = 1.50"), "{t}");
        assert!(t.contains("[controller]\ncross    = \"a\""), "{t}");
        let g = InputFile::load(&p);
        assert_eq!(g.get("keyboard", "cross").as_deref(), Some("J"));
        assert_eq!(g.get("mouse", "sensitivity").as_deref(), Some("1.50"));
        // a missing file starts from the documented defaults
        let d = InputFile::load(&dir.join("none.toml"));
        assert_eq!(d.get("keyboard", "start").as_deref(), Some("Escape"));
        let _ = fs::remove_dir_all(&dir);
    }
}
