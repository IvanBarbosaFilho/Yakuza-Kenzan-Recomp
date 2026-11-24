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
    t0: Instant,
    disc_path: String,
    patch_path: String,
    job: Option<install::Shared>,
    inp: controls::InputFile,
    capture: Option<usize>,
    capture_mods: egui::Modifiers,
    pads: Vec<String>,
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
        };
        app.reload_log();
        if let Some(m) = shortcut::ensure(&app.lay.data) {
            app.flash(&m);
        }
        app
    }

    fn recheck(&mut self) {
        self.st_game = checks::game(&self.root);
        self.st_port = checks::port(&self.lay, &self.s.fps);
        self.st_saves = checks::saves(&self.lay);
        self.checks_at = Some(Instant::now());
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
                        ui.label(RichText::new(if app_bundle() { "v0.1 · Linux · AppImage" } else { "v0.1 · Linux · dev" }).color(MUTED).size(13.0));
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
                        chip(ui, &self.st_game);
                        ui.add_space(14.0);
                        if let Some(m) = &self.last_exit {
                            ui.label(RichText::new(m).color(MUTED).size(13.5));
                        }
                        let can = self.st_game.ok && self.st_port.ok && self.game.is_none();
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
    section(ui, "Files");
    card(ui, |ui| {
        path_row(ui, "Game (ISO or folder)", "Your copy of the BLJM60064 disc: the .iso file, or a folder with PS3_GAME.", &mut app.disc_path, &["iso", "ISO"], !running);
        path_row(ui, "English translation", "The community patch archive (.7z or .zip), or the extracted folder with USRDIR and PARAM.SFO.", &mut app.patch_path, &["7z", "zip", "rar"], !running);
        ui.label(RichText::new("The port includes neither the game nor the translation. Translation: majDima, Arcodeaus, jurxd, PauseForTheCat, TheEdgyNerd and contributors.").color(MUTED).size(13.0));
    });
    ui.add_space(12.0);
    let ready = !running && !app.disc_path.trim().is_empty() && !app.patch_path.trim().is_empty();
    ui.horizontal(|ui| {
        let b = egui::Button::new(RichText::new("INSTALL").font(title(18.0)).color(if ready { Color32::WHITE } else { MUTED }))
            .fill(if ready { CRIMSON } else { FIELD })
            .stroke(Stroke::new(1.0, if ready { GOLD } else { LINE }))
            .min_size(Vec2::new(160.0, 42.0));
        if ui.add_enabled(ready, b).clicked() {
            let shared: install::Shared = Default::default();
            let (root, disc, patch) = (app.root.clone(), PathBuf::from(app.disc_path.trim()), PathBuf::from(app.patch_path.trim()));
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
    if a.len() == 4 && a[1] == "--install" {
        let p: install::Shared = Default::default();
        let root = layout::Layout::detect(find_root()).data;
        let job = p.clone();
        let t = std::thread::spawn(move || install::run(root, PathBuf::from(&a[2]), PathBuf::from(&a[3]), job));
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
