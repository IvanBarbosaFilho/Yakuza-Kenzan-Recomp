#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Kenzan launcher: the settings of the Ryu ga Gotoku Kenzan! PC port and its Play button.
//! Settings live in kenzan.ini next to run.sh; Play runs run.sh with them as its environment.

mod art;
mod checks;
mod config;
mod controls;
mod install;
mod layout;
mod shortcut;
mod update;
mod import;
mod lang;

use art::*;
use config::{Choice, Settings};
use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Stroke, Vec2};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Home,
    Graphics,
    Performance,
    Controls,
    Install,
    Mods,
    Log,
}

struct App {
    root: PathBuf,
    lay: layout::Layout,
    saved: Settings,
    s: Settings,
    tab: Tab,
    gpu: String,
    game: Option<Child>,
    started: Option<Instant>,
    last_exit: Option<String>,
    message: Option<(String, Instant)>,
    log_text: String,
    log_quiet: bool,
    checks_at: Option<Instant>,
    st_game: checks::Status,
    st_port: checks::Status,
    st_saves: checks::Status,
    st_lang: checks::Status,
    t0: Instant,
    disc_path: String,
    patch_path: String,
    job: Option<install::Shared>,
    inp: controls::InputFile,
    capture: Option<usize>,
    capture_mods: egui::Modifiers,
    pads: Vec<String>,
    /// the update check's answer (a newer release or none), while it runs
    upd_rx: Option<std::sync::mpsc::Receiver<Option<update::Release>>>,
    /// a newer release the player has not answered yet
    upd: Option<update::Release>,
    /// installing it: progress lines, the last one shown; "ERR: ..." / "DONE" end it
    upd_job: Option<(std::sync::mpsc::Receiver<String>, String)>,
    /// earlier installs of the game found around this port (import::find_near), when it has none
    near: Vec<PathBuf>,
    /// the Install tab's "Japanese original only (no translation)"
    jp_only: bool,
    /// a language job (convert / add the translation) and the language to switch to after it
    lang_job: Option<install::Shared>,
    lang_after: Option<lang::Lang>,
    /// the Home page asks for the ISO (to separate the translation) or the translation archive
    lang_ask: Option<lang::Lang>,
}

/// The project folder: the first ancestor of the launcher (or of the working directory) with run.sh.
fn app_bundle() -> bool {
    std::env::var_os("KZ_APPDIR").is_some()
}

fn find_root() -> PathBuf {
    if let Ok(r) = std::env::var("KZ_ROOT") {
        return PathBuf::from(r);
    }
    let mut starts = vec![];
    if let Ok(exe) = std::env::current_exe() {
        starts.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for s in starts {
        let mut p: Option<&Path> = Some(s.as_path());
        while let Some(d) = p {
            if d.join("run.sh").exists() && d.join("work").exists() {
                return d.to_path_buf();
            }
            p = d.parent();
        }
    }
    std::env::current_dir().unwrap_or_default()
}

/// When the last play session ended, from the newest player log.
fn last_session(lay: &layout::Layout) -> Option<String> {
    let dir = lay.work.join("logs/player");
    let newest = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with("-err.txt"))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max()?;
    let ago = SystemTime::now().duration_since(newest).ok()?.as_secs();
    Some(match ago {
        0..=119 => "just now".into(),
        120..=7199 => format!("{} minutes ago", ago / 60),
        7200..=172_799 => format!("{} hours ago", ago / 3600),
        _ => format!("{} days ago", ago / 86400),
    })
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        setup_fonts(&cc.egui_ctx);
        style(&cc.egui_ctx);
        let gpu = cc
            .gl
            .as_ref()
            .map(|gl| {
                use eframe::glow::HasContext;
                unsafe { gl.get_parameter_string(eframe::glow::RENDERER) }
            })
            .map(|r| r.split(" (").next().unwrap_or(&r).trim().to_string())
            .unwrap_or_else(|| "Unknown GPU".into());
        let lay = layout::Layout::detect(find_root());
        let (lay_input, lay_log) = (lay.input_file(), lay.last_log());
        let root = lay.data.clone();
        let s = Settings::load(&root);
        let mut app = App {
            st_game: checks::game(&root),
            st_port: checks::port(&lay, &s.fps),
            st_saves: checks::saves(&lay),
            st_lang: checks::language(&lay),
            lay,
            root,
            saved: s.clone(),
            s,
            tab: Tab::Home,
            gpu,
            game: None,
            started: None,
            last_exit: None,
            message: None,
            log_text: String::new(),
            log_quiet: true,
            checks_at: Some(Instant::now()),
            t0: Instant::now(),
            disc_path: String::new(),
            patch_path: String::new(),
            job: None,
            inp: controls::InputFile::load(&lay_input),
            capture: None,
            capture_mods: egui::Modifiers::default(),
            pads: controls::detected(&lay_log),
            upd_rx: None,
            upd: None,
            upd_job: None,
            near: vec![],
            jp_only: false,
            lang_job: None,
            lang_after: None,
            lang_ask: None,
        };
        if let Some(m) = lang::recover(&app.root) {
            app.flash(&m);
            app.recheck();
        }
        // no game here, but an earlier version of the port has one nearby: offer it (Install tab)
        if !app.st_game.ok {
            app.near = import::find_near(&app.lay.data);
            if !app.near.is_empty() {
                app.tab = Tab::Install;
            }
        }
        // a newer release on GitHub (the bundles only: a checkout updates with git)
        if app.lay.app.is_some() {
            update::cleanup();
            let (tx, rx) = std::sync::mpsc::channel();
            let skip = update::skipped(&app.lay.data);
            let japanese = lang::jp_ready(&app.lay);   // the Japanese edition updates to itself
            std::thread::spawn(move || {
                let r = update::check(japanese).ok().flatten().filter(|r| r.tag != skip);
                let _ = tx.send(r);
            });
            app.upd_rx = Some(rx);
        }
        app.reload_log();
        if let Some(m) = shortcut::ensure(&app.lay.data) {
            app.flash(&m);
        }
        app
    }

    /// The "new version" window: Update now / Later / Skip this version, then the install's progress.
    fn update_window(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.upd_rx {
            if let Ok(r) = rx.try_recv() {
                self.upd = r;
                self.upd_rx = None;
            }
        }
        if let Some((rx, last)) = &mut self.upd_job {
            while let Ok(m) = rx.try_recv() {
                *last = m;
            }
            if last == "DONE" {
                std::process::exit(0);
            }
        }
        let Some(r) = self.upd.clone() else { return };
        if self.game.is_some() {
            return;
        }
        let mut close = false;
        egui::Window::new(RichText::new("Update available").font(title(22.0)).color(GOLD))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("{} is out. You have v{}.", if r.name.is_empty() { &r.tag } else { &r.name }, update::current()))
                    .color(TEXT).size(15.5));
                ui.label(RichText::new("Your game, saves, settings, controls and mods are kept.").color(MUTED).size(13.5));
                if !r.notes.is_empty() {
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        ui.label(RichText::new(&r.notes).color(TEXT).size(13.5));
                    });
                }
                ui.add_space(8.0);
                if let Some((_, last)) = &self.upd_job {
                    if let Some(e) = last.strip_prefix("ERR: ") {
                        ui.label(RichText::new(e).color(Color32::from_rgb(0xE0, 0x70, 0x60)));
                        ui.horizontal(|ui| {
                            if ui.button("Try again").clicked() {
                                self.upd_job = None;
                            }
                            if ui.button("Open the releases page").clicked() {
                                open_url(update::RELEASES);
                            }
                            if ui.button("Close").clicked() {
                                close = true;
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(last.as_str()).color(TEXT));
                        });
                    }
                    return;
                }
                ui.horizontal(|ui| {
                    let b = egui::Button::new(RichText::new("Update now").color(Color32::WHITE)).fill(CRIMSON).stroke(Stroke::new(1.0, GOLD));
                    if ui.add(b).clicked() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        let rel = r.clone();
                        std::thread::spawn(move || {
                            let res = update::install(&rel, &tx);
                            let _ = tx.send(match res { Ok(()) => "DONE".into(), Err(e) => format!("ERR: {e}") });
                        });
                        self.upd_job = Some((rx, "Starting the download…".into()));
                    }
                    if ui.button("Later").clicked() {
                        close = true;
                    }
                    if ui.button("Skip this version").clicked() {
                        update::skip(&self.lay.data, &r.tag);
                        close = true;
                    }
                });
            });
        if close {
            self.upd = None;
            self.upd_job = None;
        }
    }

    fn recheck(&mut self) {
        self.st_game = checks::game(&self.root);
        self.st_port = checks::port(&self.lay, &self.s.fps);
        self.st_saves = checks::saves(&self.lay);
        self.st_lang = checks::language(&self.lay);
        self.checks_at = Some(Instant::now());
    }

    /// A language job that ended: switch to the language it prepared.
    fn poll_lang(&mut self) {
        let Some(job) = self.lang_job.clone() else { return };
        let (done, ok) = job.lock().map(|g| (g.done, g.error.is_none())).unwrap_or((false, false));
        if !done || self.lang_after.is_none() {
            return;
        }
        let to = self.lang_after.take().unwrap();
        if !ok {
            self.recheck();
            return;
        }
        self.lang_ask = None;
        self.switch_lang(to);
    }

    /// Switch to `to`, asking first for what it still needs (the ISO, the translation).
    fn switch_lang(&mut self, to: lang::Lang) {
        let data = self.root.clone();
        match to {
            lang::Lang::Jp if !lang::japanese_game_ready(&data) => {
                self.lang_ask = Some(lang::Lang::Jp);   // the ISO, to take the disc's files back
                return;
            }
            lang::Lang::En if !lang::english_ready(&data) => {
                self.lang_ask = Some(lang::Lang::En);   // the translation archive
                return;
            }
            _ => {}
        }
        match lang::set(&data, to) {
            Ok(()) => self.flash(if to == lang::Lang::Jp { "The game will start in Japanese." } else { "The game will start in English." }),
            Err(e) => self.flash(&format!("Could not save the language: {e}")),
        }
        self.recheck();
    }

    fn save(&mut self) {
        match self.s.save(&self.root) {
            Ok(()) => {
                self.saved = self.s.clone();
                self.flash("Settings saved.");
            }
            Err(e) => self.flash(&format!("Could not save: {e}")),
        }
    }

    fn flash(&mut self, m: &str) {
        self.message = Some((m.to_string(), Instant::now()));
    }

    fn play(&mut self) {
        if self.s != self.saved {
            self.save();
        }
        let mut cmd = match self.lay.play_command(&self.s.fps, &self.s.env()) {
            Ok(c) => c,
            Err(e) => {
                self.flash(&format!("Could not prepare the game: {e}"));
                return;
            }
        };
        match cmd.spawn() {
            Ok(c) => {
                self.game = Some(c);
                self.started = Some(Instant::now());
                self.last_exit = None;
            }
            Err(e) => self.flash(&format!("Could not start the game: {e}")),
        }
    }

    fn poll_game(&mut self) {
        if let Some(c) = self.game.as_mut() {
            if let Ok(Some(st)) = c.try_wait() {
                let secs = self.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                self.last_exit = Some(if st.success() {
                    format!("Session ended after {}h{:02}m", secs / 3600, (secs / 60) % 60)
                } else {
                    format!("The game closed with an error ({st}). See the Log tab.")
                });
                self.game = None;
                self.lay.archive_logs();
                // the in-game menu may have changed kenzan.ini (its Frame rate row)
                let disk = Settings::load(&self.root);
                if disk != self.saved {
                    self.saved = disk.clone();
                    self.s = disk;
                }
                self.reload_log();
                self.recheck();
            }
        }
    }

    fn reload_log(&mut self) {
        let p = self.lay.last_log();
        let text = std::fs::read(&p).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let noisy = |l: &str| {
            ["[WAIT]", "[WAKE]", "[SYS]", "[sem]", "[THREAD", "[fs]", "[mfc", "[kzflips]"].iter().any(|p| l.starts_with(p))
        };
        let lines: Vec<&str> = text.lines().filter(|l| !self.log_quiet || !noisy(l)).collect();
        let from = lines.len().saturating_sub(600);
        self.log_text = if lines.is_empty() {
            "No log yet: it shows up here after the first session.".into()
        } else {
            lines[from..].join("\n")
        };
    }
}

fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = FIELD;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = CRIMSON;
    v.selection.stroke = Stroke::new(1.0, GOLD);
    v.hyperlink_color = GOLD;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(4);
    }
    v.widgets.inactive.weak_bg_fill = FIELD;
    v.widgets.inactive.bg_fill = FIELD;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x3A, 0x2C, 0x27);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, GOLD_DIM);
    v.widgets.active.weak_bg_fill = CRIMSON;
    v.widgets.open.weak_bg_fill = Color32::from_rgb(0x3A, 0x2C, 0x27);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.slider_trailing_fill = true;
    ctx.set_visuals(v);
    let mut st = (*ctx.style()).clone();
    st.spacing.item_spacing = Vec2::new(10.0, 8.0);
    st.spacing.button_padding = Vec2::new(14.0, 7.0);
    st.spacing.slider_width = 200.0;
    st.text_styles.insert(egui::TextStyle::Body, FontId::new(16.0, mincho()));
    st.text_styles.insert(egui::TextStyle::Button, FontId::new(16.0, mincho()));
    st.text_styles.insert(egui::TextStyle::Small, FontId::new(13.0, mincho()));
    ctx.set_style(st);
}

/// A setting row: name and explanation on the left, the control on the right.
fn row(ui: &mut egui::Ui, name: &str, about: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 270.0).max(200.0));
            ui.label(RichText::new(name).font(title(17.0)));
            if !about.is_empty() {
                ui.label(RichText::new(about).color(MUTED).size(14.0));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(8.0);
}

fn combo(ui: &mut egui::Ui, id: &str, list: &[Choice], value: &mut String) {
    egui::ComboBox::from_id_salt(id)
        .width(250.0)
        .selected_text(config::label_of(list, value))
        .show_ui(ui, |ui| {
            for c in list {
                ui.selectable_value(value, c.value.to_string(), c.label);
            }
        });
}

fn chip(ui: &mut egui::Ui, st: &checks::Status) {
    let (mark, col) = if st.ok { ("✔", GOLD) } else { ("✖", BAD) };
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(0x10, 0x08, 0x08, 200))
        .stroke(Stroke::new(1.0, col))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(12, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(format!("{mark} {}", st.text)).color(col).size(14.0));
        });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_game();
        self.poll_lang();
        self.update_window(ctx);
        // the embers move: ~30 redraws a second while the window is shown
        ctx.request_repaint_after(Duration::from_millis(33));
        if self.checks_at.map(|t| t.elapsed() > Duration::from_secs(5)).unwrap_or(true) && self.game.is_none() {
            self.recheck();
        }
        let t = self.t0.elapsed().as_secs_f64();
        if let Some(i) = self.capture {
            let (events, mods) = ctx.input(|inp| (inp.events.clone(), inp.modifiers));
            let mut got: Option<Option<&'static str>> = None;   // Some(None) = cancelled
            for ev in &events {
                if let egui::Event::Key { key, pressed: true, .. } = ev {
                    got = Some(if *key == egui::Key::Escape { None } else { controls::key_name(*key) });
                    break;
                }
            }
            if got.is_none() {
                let was = self.capture_mods;
                if mods.shift && !was.shift { got = Some(Some("Shift")); }
                else if mods.ctrl && !was.ctrl { got = Some(Some("Ctrl")); }
                else if mods.alt && !was.alt { got = Some(Some("Alt")); }
            }
            if let Some(r) = got {
                if let Some(name) = r {
                    self.inp.set_str("keyboard", controls::CONTROLS[i].0, name);
                }
                self.capture = None;
            }
        }

        egui::TopBottomPanel::bottom("tabs")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x0E, 0x0A, 0x09))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x5A, 0x16, 0x14)))
                    .inner_margin(egui::Margin::symmetric(22, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tb, name) in [
                        (Tab::Home, "Home"),
                        (Tab::Graphics, "Graphics"),
                        (Tab::Performance, "Performance"),
                        (Tab::Controls, "Controls"),
                        (Tab::Install, "Install"),
                        (Tab::Mods, "Mods"),
                        (Tab::Log, "Log"),
                    ] {
                        if tab(ui, name, self.tab == tb).clicked() {
                            self.tab = tb;
                            if tb == Tab::Log {
                                self.reload_log();
                            }
                            if tb == Tab::Controls && !self.inp.dirty {
                                // the in-game menu may have changed it
                                self.inp = controls::InputFile::load(&self.lay.input_file());
                                self.pads = controls::detected(&self.lay.last_log());
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("v{} · {}", update::VERSION,
                            if cfg!(windows) { "Windows" } else if app_bundle() { "Linux · AppImage" } else { "Linux · dev" }))
                            .color(MUTED).size(13.0));
                    });
                });
            });

        // the right side: the art, the title, Play and the checks
        egui::SidePanel::right("hero")
            .exact_width(460.0)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| {
                let r = ui.max_rect();
                hero_backdrop(ui, r, t);
                let inner = r.shrink2(Vec2::new(30.0, 26.0));
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} · {} · {}",
                                config::label_of(config::RESOLUTIONS, &self.s.resolution),
                                config::label_of(config::FPS_MODES, &self.s.fps),
                                config::label_of(config::DISPLAY_MODES, &self.s.display_mode)
                            ))
                            .color(TEXT)
                            .size(14.5),
                        );
                        ui.add_space(4.0);
                        chip(ui, &checks::Status { ok: true, text: self.gpu.clone() });
                        chip(ui, &self.st_saves);
                        chip(ui, &self.st_port);
                        chip(ui, &self.st_lang);
                        chip(ui, &self.st_game);
                        ui.add_space(14.0);
                        if let Some(m) = &self.last_exit {
                            ui.label(RichText::new(m).color(MUTED).size(13.5));
                        }
                        let can = self.st_game.ok && self.st_port.ok && self.st_lang.ok && self.game.is_none();
                        let label = if self.game.is_some() { "PLAYING…" } else { "PLAY" };
                        if play_button(ui, label, can).clicked() {
                            self.play();
                        }
                        ui.add_space(16.0);
                        ui.label(RichText::new("RYU GA GOTOKU KENZAN!").font(title(17.0)).color(GOLD));
                        ui.label(RichText::new("見参!").font(title(92.0)).color(TEXT));
                        ui.label(RichText::new("龍が如く").font(title(26.0)).color(GOLD));
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 36, right: 30, top: 26, bottom: 10 }))
            .show(ctx, |ui| {
                let (title_txt, sub) = match self.tab {
                    Tab::Home => ("Home", "Unofficial PC port of Ryu ga Gotoku Kenzan! (PS3). The game files are your own."),
                    Tab::Graphics => ("Graphics", "Saved to kenzan.ini. Takes effect the next time the game starts."),
                    Tab::Performance => ("Performance", "Frame rate and presets for every kind of PC."),
                    Tab::Controls => ("Controls", "Keyboard, mouse and controller. Same file as the Controls page of the in-game menu (F2)."),
                    Tab::Install => ("Install", "Builds the game from your ISO and the community English translation."),
                    Tab::Mods => ("Mods", "Files that replace the game's while you play. Your game files are never changed."),
                    Tab::Log => ("Log", "The last session. If something goes wrong, send this text."),
                };
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title_txt).font(title(38.0)).color(TEXT));
                        ui.label(RichText::new(sub).color(MUTED).size(14.5));
                    });
                    if matches!(self.tab, Tab::Graphics | Tab::Performance | Tab::Controls) {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let dirty = self.s != self.saved || (self.tab == Tab::Controls && self.inp.dirty);
                            let b = egui::Button::new(RichText::new("SAVE").font(title(17.0)).color(if dirty { Color32::WHITE } else { MUTED }))
                                .fill(if dirty { CRIMSON } else { FIELD })
                                .stroke(Stroke::new(1.0, if dirty { GOLD } else { LINE }))
                                .min_size(Vec2::new(120.0, 40.0));
                            if ui.add_enabled(dirty, b).clicked() {
                                if self.inp.dirty {
                                    if let Err(e) = self.inp.save() {
                                        self.flash(&format!("Could not save the controls: {e}"));
                                    }
                                }
                                self.save();
                            }
                        });
                    }
                });
                if let Some((m, t0)) = &self.message {
                    if t0.elapsed() < Duration::from_secs(4) {
                        ui.label(RichText::new(m).color(GOLD));
                    } else {
                        self.message = None;
                    }
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(2.0);
                    match self.tab {
                        Tab::Home => home(ui, self),
                        Tab::Graphics => graphics(ui, &mut self.s),
                        Tab::Performance => performance(ui, &mut self.s),
                        Tab::Controls => controls_tab(ui, self),
                        Tab::Install => install_tab(ui, self),
                        Tab::Mods => mods_tab(ui, self),
                        Tab::Log => log(ui, self),
                    }
                    ui.add_space(16.0);
                });
            });
    }
}

fn status_line(ui: &mut egui::Ui, st: &checks::Status) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(if st.ok { "✔" } else { "✖" }).color(if st.ok { GOLD } else { BAD }).size(16.0));
        ui.label(RichText::new(&st.text).size(16.0));
    });
}

fn home(ui: &mut egui::Ui, app: &mut App) {
    section(ui, "Ready to play");
    card(ui, |ui| {
        status_line(ui, &app.st_game);
        status_line(ui, &app.st_port);
        status_line(ui, &app.st_saves);
        if !app.st_game.ok && ui.link(RichText::new("Install the game →").size(15.0)).clicked() {
            app.tab = Tab::Install;
        }
        if let Some(when) = last_session(&app.lay) {
            ui.add_space(2.0);
            ui.label(RichText::new(format!("Last session: {when}")).color(MUTED).size(14.0));
        }
    });
    language_card(ui, app);
    section(ui, "Your settings");
    card(ui, |ui| {
        egui::Grid::new("summary").num_columns(2).spacing(Vec2::new(28.0, 6.0)).show(ui, |ui| {
            let s = &app.s;
            for (k, v) in [
                ("Resolution", config::label_of(config::RESOLUTIONS, &s.resolution)),
                ("Frame rate", config::label_of(config::FPS_MODES, &s.fps)),
                ("Shadows", config::label_of(config::SHADOW_MODES, &s.shadows)),
                ("Anti-aliasing", config::label_of(config::AA_MODES, &s.aa)),
                ("Display", config::label_of(config::DISPLAY_MODES, &s.display_mode)),
            ] {
                ui.label(RichText::new(k).color(MUTED));
                ui.label(RichText::new(v).font(title(16.0)));
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        if ui.link(RichText::new("Adjust in Graphics →").size(14.5)).clicked() {
            app.tab = Tab::Graphics;
        }
    });
}

/// Home's Language card: the current language and one button to the other.
fn language_card(ui: &mut egui::Ui, app: &mut App) {
    if !app.st_game.ok {
        return;
    }
    let running = app.lang_job.as_ref().map(|j| j.lock().map(|g| !g.done).unwrap_or(false)).unwrap_or(false)
        || app.game.is_some();
    let cur = lang::get(&app.root);
    section(ui, "Language");
    card(ui, |ui| {
        status_line(ui, &app.st_lang);
        let other = if cur == lang::Lang::Jp { lang::Lang::En } else { lang::Lang::Jp };
        // the normal edition has no Japanese files: the Japanese edition is a separate download
        let edition = other != lang::Lang::Jp || lang::jp_ready(&app.lay);
        ui.horizontal(|ui| {
            let label = if other == lang::Lang::Jp { "Switch to Japanese" } else { "Switch to English" };
            if ui.add_enabled(!running && edition, egui::Button::new(RichText::new(label).size(15.0))).clicked() {
                app.switch_lang(other);
            }
        });
        if !edition {
            ui.label(RichText::new(format!("Japanese needs the Japanese edition of the port ({}). Your game, \
                saves and settings stay as they are: download it, put it in place of this one, and switch here.",
                update::asset(true))).color(MUTED).size(13.5));
            if ui.link(RichText::new("Open the releases page →").size(14.0)).clicked() {
                open_url(update::RELEASES);
            }
        }
        ui.label(RichText::new("Your saves work in both languages.").color(MUTED).size(13.5));
        match app.lang_ask {
            Some(lang::Lang::Jp) => {
                ui.add_space(6.0);
                ui.label(RichText::new("This installation has the translation inside the game's files (an earlier version of the \
                    port installed it so). To play in Japanese, the 1040 files it replaced are read back from your ISO once \
                    (about 4.4 GB); the translation is kept beside the game, and switching is instant from then on.")
                    .color(TEXT).size(14.0));
                path_row(ui, "Game (ISO or folder)", "The same BLJM60064 disc the game was installed from.", &mut app.disc_path, &["iso", "ISO"], !running);
                let ok = !running && !app.disc_path.trim().is_empty();
                if ui.add_enabled(ok, egui::Button::new(RichText::new("Separate the translation").size(15.0))).clicked() {
                    let p: install::Shared = Default::default();
                    let (data, disc, job) = (app.root.clone(), PathBuf::from(app.disc_path.trim()), p.clone());
                    std::thread::spawn(move || lang::convert(data, disc, job));
                    app.lang_job = Some(p);
                    app.lang_after = Some(lang::Lang::Jp);
                }
            }
            Some(lang::Lang::En) => {
                ui.add_space(6.0);
                ui.label(RichText::new("The game was installed without the translation. Point to the community English \
                    translation (the same archive the Install page asks for) to add it.").color(TEXT).size(14.0));
                path_row(ui, "English translation", "The patch archive (.7z or .zip), or its folder with USRDIR and PARAM.SFO.", &mut app.patch_path, &["7z", "zip", "rar"], !running);
                let ok = !running && !app.patch_path.trim().is_empty();
                if ui.add_enabled(ok, egui::Button::new(RichText::new("Add the translation").size(15.0))).clicked() {
                    let p: install::Shared = Default::default();
                    let (data, patch, job) = (app.root.clone(), PathBuf::from(app.patch_path.trim()), p.clone());
                    std::thread::spawn(move || lang::add_translation(data, patch, job));
                    app.lang_job = Some(p);
                    app.lang_after = Some(lang::Lang::En);
                }
            }
            None => {}
        }
        if let Some(job) = app.lang_job.clone() {
            if let Ok(g) = job.lock() {
                if !g.done || g.error.is_some() {
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(g.frac).fill(if g.error.is_some() { BAD } else { CRIMSON }).desired_height(18.0)
                        .text(RichText::new(format!("{:.0}%", g.frac * 100.0)).color(Color32::WHITE)));
                    ui.label(RichText::new(&g.step).size(14.5).color(if g.error.is_some() { BAD } else { TEXT }));
                    if let Some(e) = &g.error {
                        ui.label(RichText::new(format!("Error: {e}")).color(BAD));
                    }
                    if !g.done {
                        ui.label(RichText::new("Do not close the launcher meanwhile.").color(GOLD).size(13.5));
                    }
                }
            }
        }
    });
}

fn graphics(ui: &mut egui::Ui, s: &mut Settings) {
    section(ui, "Display");
    card(ui, |ui| {
        row(ui, "Display mode", "Fullscreen fills the whole monitor.", |ui| {
            combo(ui, "display", config::DISPLAY_MODES, &mut s.display_mode)
        });
        row(ui, "Presentation", "How frames reach the monitor. The default avoids tearing without adding lag.", |ui| {
            combo(ui, "present", config::PRESENT_MODES, &mut s.present)
        });
    });
    section(ui, "Resolution");
    card(ui, |ui| {
        row(ui, "Internal resolution", "The resolution the game renders at. The PS3 used 720p; higher is sharper and heavier.", |ui| {
            combo(ui, "res", config::RESOLUTIONS, &mut s.resolution)
        });
    });
    section(ui, "Quality");
    card(ui, |ui| {
        row(ui, "Shadows", "High doubles the shadow map resolution: crisper edges.", |ui| {
            combo(ui, "shadows", config::SHADOW_MODES, &mut s.shadows)
        });
        row(ui, "Anti-aliasing", "Smooths jagged edges after rendering. SMAA is cleaner; FXAA is lighter.", |ui| {
            combo(ui, "aa", config::AA_MODES, &mut s.aa)
        });
        row(ui, "MSAA", "Smoothing on the geometry itself. Looks great, but taxes the GPU.", |ui| {
            combo(ui, "msaa", config::MSAA_MODES, &mut s.msaa)
        });
        row(ui, "Anisotropic filtering", "Floors and walls stay sharp even at an angle.", |ui| {
            combo(ui, "aniso", config::ANISO_MODES, &mut s.aniso)
        });
        row(ui, "Sharpening", "Detail enhancement (AMD CAS). 0 turns it off.", |ui| {
            ui.add(egui::Slider::new(&mut s.sharpen, 0.0..=1.0).step_by(0.05).fixed_decimals(2));
        });
    });
}

fn performance(ui: &mut egui::Ui, s: &mut Settings) {
    section(ui, "Frame rate");
    card(ui, |ui| {
        row(ui, "FPS", "60 is recommended. 120 needs a powerful PC.", |ui| {
            combo(ui, "fps", config::FPS_MODES, &mut s.fps)
        });
        if s.fps == "120" {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(0x60, 0x48, 0x10, 60))
                .stroke(Stroke::new(1.0, GOLD_DIM))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.label(RichText::new("⚠  In crowded scenes, like the Gion festival, the game may run in slow motion at 120 FPS.").color(GOLD));
                });
        }
    });
    section(ui, "Presets");
    ui.label(RichText::new("One click sets several options at once; press Save or Play to keep them.").color(MUTED).size(14.0));
    ui.add_space(4.0);
    for p in config::PRESETS {
        card(ui, |ui| {
            row(ui, p.name, p.about, |ui| {
                if ui.add(egui::Button::new(RichText::new("Apply").font(title(16.0))).min_size(Vec2::new(120.0, 36.0))).clicked() {
                    (p.apply)(s);
                }
            });
        });
        ui.add_space(6.0);
    }
}

fn path_row(ui: &mut egui::Ui, name: &str, about: &str, path: &mut String, file_ext: &[&str], enabled: bool) {
    ui.label(RichText::new(name).font(title(17.0)));
    ui.label(RichText::new(about).color(MUTED).size(14.0));
    ui.horizontal(|ui| {
        ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(path).desired_width((ui.available_width() - 200.0).max(160.0)).hint_text("path…"),
        );
        if ui.add_enabled(enabled, egui::Button::new("File…")).clicked() {
            if let Some(f) = rfd::FileDialog::new().add_filter(name, file_ext).pick_file() {
                *path = f.display().to_string();
            }
        }
        if ui.add_enabled(enabled, egui::Button::new("Folder…")).clicked() {
            if let Some(f) = rfd::FileDialog::new().pick_folder() {
                *path = f.display().to_string();
            }
        }
    });
    ui.add_space(10.0);
}

fn pick(ui: &mut egui::Ui, id: &str, list: &[(&str, &str)], cur: &str, width: f32) -> Option<String> {
    let shown = list.iter().find(|(v, _)| *v == cur).map(|(_, l)| l.to_string()).unwrap_or_else(|| cur.to_string());
    let mut out = None;
    egui::ComboBox::from_id_salt(id).width(width).selected_text(shown).show_ui(ui, |ui| {
        for (v, l) in list {
            if ui.selectable_label(*v == cur, *l).clicked() {
                out = Some(v.to_string());
            }
        }
    });
    out
}

fn controls_tab(ui: &mut egui::Ui, app: &mut App) {
    section(ui, "Input");
    card(ui, |ui| {
        row(ui, "Devices", "What the game listens to. Controller, keyboard and mouse together is the default.", |ui| {
            combo(ui, "inmode", config::INPUT_MODES, &mut app.s.input_mode)
        });
        row(ui, "Rumble", "Controller vibration. Experimental: let us know if it causes problems.", |ui| {
            combo(ui, "rumble", config::RUMBLE_MODES, &mut app.s.rumble)
        });
        ui.label(RichText::new(if app.pads.is_empty() { "No controller seen in the last session.".to_string() }
                              else { format!("Controllers seen in the last session: {}", app.pads.join(" · ")) })
            .color(MUTED).size(14.0));
    });

    section(ui, "Buttons");
    ui.label(RichText::new("Keyboard: click Capture and press the key (Esc cancels); separate multiple keys with commas. \
                            Key names follow US keyboard positions.").color(MUTED).size(14.0));
    ui.add_space(4.0);
    card(ui, |ui| {
        egui::Grid::new("binds").num_columns(4).spacing(Vec2::new(14.0, 8.0)).striped(false).show(ui, |ui| {
            for h in ["PS3 button", "Keyboard", "Mouse", "Controller"] {
                ui.label(RichText::new(h).font(title(15.0)).color(GOLD));
            }
            ui.end_row();
            for (i, (key, label)) in controls::CONTROLS.iter().enumerate() {
                ui.label(RichText::new(*label).size(15.0));
                ui.horizontal(|ui| {
                    let mut kb = app.inp.get("keyboard", key).unwrap_or_default();
                    if ui.add(egui::TextEdit::singleline(&mut kb).desired_width(110.0)).changed() {
                        app.inp.set_str("keyboard", key, &kb);
                    }
                    let capturing = app.capture == Some(i);
                    let b = egui::Button::new(RichText::new(if capturing { "Press…" } else { "Capture" }).size(13.5))
                        .fill(if capturing { CRIMSON } else { FIELD });
                    if ui.add(b).clicked() {
                        app.capture = if capturing { None } else { Some(i) };
                        app.capture_mods = ui.ctx().input(|inp| inp.modifiers);
                    }
                });
                let ms = app.inp.get("mouse_buttons", key).unwrap_or_default();
                if let Some(v) = pick(ui, &format!("m{i}"), controls::MOUSE_INPUTS, &ms, 150.0) {
                    app.inp.set_str("mouse_buttons", key, &v);
                }
                let pad = app.inp.get("controller", key).unwrap_or_default();
                if let Some(v) = pick(ui, &format!("p{i}"), controls::PAD_INPUTS, &pad, 170.0) {
                    app.inp.set_str("controller", key, &v);
                }
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        if ui.button("Restore default bindings").clicked() {
            app.inp.reset();
        }
    });

    section(ui, "Mouse");
    card(ui, |ui| {
        let mut cam = app.inp.get("mouse", "camera").map(|v| v != "false").unwrap_or(true);
        row(ui, "Mouse camera", "The mouse moves the camera (the right stick). Click the game window to capture the mouse.", |ui| {
            if ui.checkbox(&mut cam, "").changed() { app.inp.set_raw("mouse", "camera", if cam { "true" } else { "false" }); }
        });
        let mut sens = app.inp.get("mouse", "sensitivity").and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
        row(ui, "Sensitivity", "Camera speed with the mouse.", |ui| {
            if ui.add(egui::Slider::new(&mut sens, 0.1..=4.0).step_by(0.05).fixed_decimals(2)).changed() {
                app.inp.set_raw("mouse", "sensitivity", &format!("{sens:.2}"));
            }
        });
        for (k, name) in [("invert_y", "Invert vertical axis"), ("invert_x", "Invert horizontal axis")] {
            let mut on = app.inp.get("mouse", k).map(|v| v == "true").unwrap_or(false);
            row(ui, name, "", |ui| {
                if ui.checkbox(&mut on, "").changed() { app.inp.set_raw("mouse", k, if on { "true" } else { "false" }); }
            });
        }
    });

    section(ui, "Controller");
    card(ui, |ui| {
        let mut dz = app.inp.get("controller_settings", "deadzone").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.08);
        row(ui, "Stick dead zone", "How far a stick must move from center to register. Raise it if your character drifts.", |ui| {
            if ui.add(egui::Slider::new(&mut dz, 0.0..=0.4).step_by(0.01).fixed_decimals(2)).changed() {
                app.inp.set_raw("controller_settings", "deadzone", &format!("{dz:.2}"));
            }
        });
        let mut tt = app.inp.get("controller_settings", "trigger_threshold").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.12);
        row(ui, "Trigger threshold", "How far L2/R2 must be pressed to count as a button press.", |ui| {
            if ui.add(egui::Slider::new(&mut tt, 0.02..=0.9).step_by(0.01).fixed_decimals(2)).changed() {
                app.inp.set_raw("controller_settings", "trigger_threshold", &format!("{tt:.2}"));
            }
        });
        let mut inv = app.inp.get("controller_settings", "invert_right_y").map(|v| v == "true").unwrap_or(false);
        row(ui, "Invert camera (vertical)", "Pushing the right stick up looks down.", |ui| {
            if ui.checkbox(&mut inv, "").changed() { app.inp.set_raw("controller_settings", "invert_right_y", if inv { "true" } else { "false" }); }
        });
        let mut south = app.inp.get("controller_settings", "left_stick").map(|v| v == "right").unwrap_or(false);
        row(ui, "Swap sticks (left-handed)", "The right stick moves and the left stick controls the camera.", |ui| {
            if ui.checkbox(&mut south, "").changed() {
                app.inp.set_str("controller_settings", "left_stick", if south { "right" } else { "left" });
                app.inp.set_str("controller_settings", "right_stick", if south { "left" } else { "right" });
            }
        });
        let mut nin = app.inp.get("controller_settings", "nintendo_layout").map(|v| v == "label").unwrap_or(false);
        row(ui, "Nintendo controllers by label", "On: the A button confirms (as on a Japanese PS3, A = ×). Off: by position.", |ui| {
            if ui.checkbox(&mut nin, "").changed() { app.inp.set_str("controller_settings", "nintendo_layout", if nin { "label" } else { "position" }); }
        });
    });
}

fn install_tab(ui: &mut egui::Ui, app: &mut App) {
    let running = app.job.as_ref().map(|j| j.lock().map(|g| !g.done).unwrap_or(false)).unwrap_or(false);
    section(ui, "Status");
    card(ui, |ui| {
        status_line(ui, &app.st_game);
        if app.st_game.ok {
            ui.label(RichText::new("The game is already installed. Installing again keeps the current copy as game.old-….").color(MUTED).size(14.0));
        }
    });
    // already installed by an earlier version of the port: use it instead of installing again
    if !app.st_game.ok {
        section(ui, "Already installed?");
        let mut picked: Option<PathBuf> = None;
        card(ui, |ui| {
            if app.near.is_empty() {
                ui.label(RichText::new("If an earlier version of the port installed the game, point to its folder: \
                    the game (and its saves, settings and mods) are used as they are, nothing is installed again.")
                    .color(MUTED).size(14.0));
            } else {
                for p in app.near.clone() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Found: {}", p.display())).color(TEXT).size(14.5));
                        if ui.add_enabled(!running, egui::Button::new("Use this one")).clicked() {
                            picked = Some(p.clone());
                        }
                    });
                }
                ui.label(RichText::new("Its game, saves, settings and mods are brought here; nothing is installed again.")
                    .color(MUTED).size(13.5));
            }
            ui.add_space(4.0);
            if ui.add_enabled(!running, egui::Button::new("Use an existing installation…")).clicked() {
                if let Some(f) = rfd::FileDialog::new().pick_folder() {
                    picked = Some(f);
                }
            }
        });
        if let Some(p) = picked {
            match import::import(&p, &app.lay.data) {
                Ok(m) => {
                    app.flash(&m);
                    app.near.clear();
                    app.checks_at = None;
                    app.recheck();
                }
                Err(e) => app.flash(&e),
            }
        }
    }
    section(ui, "Files");
    card(ui, |ui| {
        path_row(ui, "Game (ISO or folder)", "Your copy of the BLJM60064 disc: the .iso file, or a folder with PS3_GAME.", &mut app.disc_path, &["iso", "ISO"], !running);
        path_row(ui, "English translation", "The community patch archive (.7z or .zip), or the extracted folder with USRDIR and PARAM.SFO.", &mut app.patch_path, &["7z", "zip", "rar"], !running && !app.jp_only);
        // the normal edition has no Japanese files: a Japanese-only install could not be played
        if lang::jp_ready(&app.lay) {
            ui.checkbox(&mut app.jp_only, RichText::new("Japanese original only (no translation; it can be added later from Home)").size(14.5));
        } else {
            app.jp_only = false;
        }
        ui.label(RichText::new("The port includes neither the game nor the translation. Translation: majDima, Arcodeaus, jurxd, PauseForTheCat, TheEdgyNerd and contributors.").color(MUTED).size(13.0));
    });
    ui.add_space(12.0);
    let ready = !running && !app.disc_path.trim().is_empty() && (app.jp_only || !app.patch_path.trim().is_empty());
    ui.horizontal(|ui| {
        let b = egui::Button::new(RichText::new("INSTALL").font(title(18.0)).color(if ready { Color32::WHITE } else { MUTED }))
            .fill(if ready { CRIMSON } else { FIELD })
            .stroke(Stroke::new(1.0, if ready { GOLD } else { LINE }))
            .min_size(Vec2::new(160.0, 42.0));
        if ui.add_enabled(ready, b).clicked() {
            let shared: install::Shared = Default::default();
            let (root, disc) = (app.root.clone(), PathBuf::from(app.disc_path.trim()));
            let patch = (!app.jp_only).then(|| PathBuf::from(app.patch_path.trim()));
            let job = shared.clone();
            std::thread::spawn(move || install::run(root, disc, patch, job));
            app.job = Some(shared);
        }
        if running {
            ui.label(RichText::new("Do not close the launcher while installing.").color(GOLD));
        }
    });
    if let Some(job) = app.job.clone() {
        if let Ok(g) = job.lock() {
            ui.add_space(10.0);
            ui.add(egui::ProgressBar::new(g.frac).fill(if g.error.is_some() { BAD } else { CRIMSON }).desired_height(22.0)
                .text(RichText::new(format!("{:.0}%", g.frac * 100.0)).color(Color32::WHITE)));
            ui.label(RichText::new(&g.step).font(title(16.0)).color(if g.error.is_some() { BAD } else { TEXT }));
            if let Some(e) = &g.error {
                ui.label(RichText::new(format!("Error: {e}")).color(BAD));
            }
            egui::Frame::new().fill(Color32::from_rgb(0x0E, 0x0A, 0x09)).stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for l in g.log.iter().rev().take(12).rev() {
                        ui.label(RichText::new(l).monospace().size(12.5).color(MUTED));
                    }
                });
            if g.done {
                app.checks_at = None;   // re-check the game on the next frame
            }
        }
    }
}

/// One mod folder in the mods folder: on unless its name starts with '_'.
struct ModEntry {
    dir: PathBuf,
    name: String,
    on: bool,
    files: usize,
    note: Option<String>,
}

fn list_mods(root: &Path) -> Vec<ModEntry> {
    fn count(d: &Path, note: &mut Option<String>) -> usize {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    n += count(&p, note);
                } else {
                    let low = p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
                    if low.ends_with(".txt") || low.ends_with(".md") {
                        if note.is_none() {
                            *note = std::fs::read_to_string(&p).ok().map(|t| t.trim().chars().take(400).collect());
                        }
                    } else {
                        n += 1;
                    }
                }
            }
        }
        n
    }
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path();
            let fname = e.file_name().to_string_lossy().to_string();
            if !p.is_dir() || fname.starts_with('.') {
                continue;
            }
            let mut note = None;
            let files = count(&p, &mut note);
            v.push(ModEntry { on: !fname.starts_with('_'), name: fname.trim_start_matches('_').to_string(), dir: p, files, note });
        }
    }
    v.sort_by_key(|m| m.name.to_lowercase());
    v
}

/// A mod zip unpacked into its own folder of the mods folder, named after the zip.
fn add_mod_zip(root: &Path, zip: &Path) -> Result<String, String> {
    let stem = zip.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mod".into());
    let name: String = stem.chars().map(|c| if c.is_alphanumeric() || " _-.()".contains(c) { c } else { '_' }).collect();
    let name = name.trim_start_matches(['_', '.']).trim().to_string();
    let dest = root.join(&name);
    if dest.exists() || root.join(format!("_{name}")).exists() {
        return Err(format!("\"{name}\" is already in the mods folder."));
    }
    let tmp = root.join(format!(".adding-{name}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let sz = install::seven_zip()?;
    let mut c = Command::new(sz);
    c.arg("x").arg("-y").arg(format!("-o{}", tmp.display())).arg(zip);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let st = c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map_err(|e| e.to_string())?;
    if !st.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("The zip could not be unpacked.".into());
    }
    // a zip that holds one folder with everything in it: use that folder
    let inner: Vec<_> = std::fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().collect();
    let src = if inner.len() == 1 && inner[0].path().is_dir() { inner[0].path() } else { tmp.clone() };
    std::fs::rename(&src, &dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(name)
}

fn mods_tab(ui: &mut egui::Ui, app: &mut App) {
    let root = app.lay.mods();
    let _ = std::fs::create_dir_all(&root);
    ui.label(
        RichText::new(
            "Each mod is a folder here. Its files take the place of the game's files with the same name \
             (or the same path under USRDIR, e.g. bin/shader/sh_iori_ps3.par) every time the game starts. \
             Nothing is copied over the game, so turning a mod off is enough to undo it.",
        )
        .color(MUTED)
        .size(14.0),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Add mod (.zip)…").clicked() {
            if let Some(z) = rfd::FileDialog::new().add_filter("Mod archive", &["zip", "7z", "rar"]).pick_file() {
                match add_mod_zip(&root, &z) {
                    Ok(n) => app.flash(&format!("Added \"{n}\". It is used the next time the game starts.")),
                    Err(e) => app.flash(&e),
                }
            }
        }
        if ui.button("Open mods folder").clicked() {
            let _ = Command::new(if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(&root).spawn();
        }
    });
    ui.add_space(10.0);
    let mods = list_mods(&root);
    if mods.is_empty() {
        ui.label(RichText::new("No mods yet.").color(MUTED).size(14.5));
        return;
    }
    for m in mods {
        egui::Frame::new()
            .fill(FIELD)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let mut on = m.on;
                    if ui.checkbox(&mut on, RichText::new(&m.name).color(TEXT).size(15.5)).changed() {
                        let parent = m.dir.parent().unwrap_or(&root).to_path_buf();
                        let to = parent.join(if on { m.name.clone() } else { format!("_{}", m.name) });
                        if let Err(e) = std::fs::rename(&m.dir, &to) {
                            app.flash(&format!("Could not switch it: {e}"));
                        }
                    }
                    ui.label(RichText::new(format!("{} file(s)", m.files)).color(MUTED).size(13.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Open").clicked() {
                            let _ = Command::new(if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(&m.dir).spawn();
                        }
                    });
                });
                if let Some(n) = &m.note {
                    ui.label(RichText::new(n).color(MUTED).size(13.0));
                }
            });
        ui.add_space(6.0);
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new("Which files a mod changed is written at the top of the Log tab after a session ([mods] lines).")
            .color(MUTED)
            .size(13.0),
    );
}

fn log(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        if ui.button("Reload").clicked() {
            app.reload_log();
        }
        if ui.checkbox(&mut app.log_quiet, "Hide routine lines").changed() {
            app.reload_log();
        }
        if ui.button("Open logs folder").clicked() {
            let _ = Command::new(if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(app.lay.work.join("logs")).spawn();
        }
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(app.log_text.clone());
            app.flash("Log copied.");
        }
    });
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(Color32::from_rgb(0x0E, 0x0A, 0x09))
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.log_text.as_str())
                    .font(FontId::monospace(12.5))
                    .desired_width(f32::INFINITY)
                    .desired_rows(30),
            );
        });
}


fn main() -> eframe::Result {
    // headless helpers: --unfself <in> <out>, --install <disc> <patch> (tests and scripts)
    let a: Vec<String> = std::env::args().collect();
    if a.len() == 4 && a[1] == "--unfself" {
        let d = std::fs::read(&a[2]).expect("read");
        match install::unfself(&d) {
            Ok(e) => { std::fs::write(&a[3], &e).expect("write"); println!("ok {} bytes", e.len()); }
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        }
        return Ok(());
    }
    if a.len() == 2 && a[1] == "--play" {
        // the Play button's own path, without the window (tests): kenzan.ini's settings, the
        // same command; KZ_TIMEOUT in the environment bounds the run
        let lay = layout::Layout::detect(find_root());
        let s = Settings::load(&lay.data);
        let mut cmd = lay.play_command(&s.fps, &s.env()).unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(2) });
        let st = cmd.status().expect("spawn");
        println!("game exited: {st}");
        std::process::exit(if st.success() { 0 } else { 1 });
    }
    if a.len() == 2 && a[1] == "--self-update" {
        // the Update button's own path, without the window (tests: under Wine for the Windows
        // half): KZ_UPDATE_AS=<older version> to be offered the current release, KZ_UPDATE_NO_START=1
        // not to start the new launcher afterwards. Also written to work/logs/self-update.txt (the
        // Windows launcher has no console).
        let lay = layout::Layout::detect(find_root());
        let logf = lay.work.join("logs").join("self-update.txt");
        let _ = std::fs::create_dir_all(logf.parent().unwrap());
        let _ = std::fs::write(&logf, "");
        let log = |m: &str| {
            println!("{m}");
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&logf) { let _ = writeln!(f, "{m}"); }
        };
        let japanese = lang::jp_ready(&lay);
        log(&format!("launcher v{} (acting as v{}), {} edition, asset {}", update::VERSION, update::current(),
                     if japanese { "Japanese" } else { "normal" }, update::asset(japanese)));
        let r = match update::check(japanese) {
            Ok(Some(r)) => r,
            Ok(None) => { log("no newer release"); return Ok(()); }
            Err(e) => { log(&format!("check failed: {e}")); std::process::exit(1); }
        };
        log(&format!("found {} ({} bytes, sha256 {})", r.tag, r.size, if r.sha256.is_empty() { "none" } else { &r.sha256 }));
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let t = std::thread::spawn(move || update::install(&r, &tx));
        let mut last = String::new();
        for m in rx { if m != last { log(&format!("  {m}")); last = m; } }
        match t.join() {
            Ok(Ok(())) => log("update installed"),
            Ok(Err(e)) => { log(&format!("update failed: {e}")); std::process::exit(1); }
            Err(_) => { log("update thread panicked"); std::process::exit(1); }
        }
        return Ok(());
    }
    if a.len() == 4 && a[1] == "--install" {
        let p: install::Shared = Default::default();
        let root = layout::Layout::detect(find_root()).data;
        let job = p.clone();
        let patch = (a[3] != "-").then(|| PathBuf::from(&a[3]));   // "-": the Japanese original only
        let t = std::thread::spawn(move || install::run(root, PathBuf::from(&a[2]), patch, job));
        let mut last = String::new();
        while !t.is_finished() {
            std::thread::sleep(Duration::from_millis(500));
            if let Ok(g) = p.lock() { let l = format!("{:3.0}% {}", g.frac * 100.0, g.step); if l != last { println!("{l}"); last = l; } }
        }
        let g = p.lock().expect("lock");
        for l in &g.log { println!("  {l}"); }
        if g.error.is_some() { std::process::exit(1); }
        return Ok(());
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kenzan! — Launcher")
            .with_app_id("kenzan-launcher")
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size([1100.0, 660.0]),
        ..Default::default()
    };
    eframe::run_native("kenzan-launcher", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

fn open_url(url: &str) {
    #[cfg(windows)]
    let _ = Command::new("explorer").arg(url).spawn();
    #[cfg(not(windows))]
    let _ = Command::new("xdg-open").arg(url).spawn();
}
