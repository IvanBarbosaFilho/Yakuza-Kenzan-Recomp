//! The launcher's look: the Shippori Mincho faces (SIL OFL 1.1, subset to the characters used;
//! assets/OFL-ShipporiMincho.txt) and the hero panel, all drawn in code -- no art from the game.

use eframe::egui::{
    self, epaint::PathShape, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily,
    FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0x14, 0x10, 0x0F);
pub const PANEL: Color32 = Color32::from_rgb(0x1B, 0x15, 0x13);
pub const CARD: Color32 = Color32::from_rgb(0x22, 0x1A, 0x17);
pub const FIELD: Color32 = Color32::from_rgb(0x2C, 0x22, 0x1E);
pub const LINE: Color32 = Color32::from_rgb(0x3B, 0x2D, 0x27);
pub const CRIMSON: Color32 = Color32::from_rgb(0xA3, 0x1D, 0x1D);
pub const CRIMSON_HI: Color32 = Color32::from_rgb(0xD0, 0x33, 0x2C);
pub const GOLD: Color32 = Color32::from_rgb(0xD4, 0xAF, 0x55);
pub const GOLD_DIM: Color32 = Color32::from_rgb(0x8A, 0x70, 0x38);
pub const TEXT: Color32 = Color32::from_rgb(0xEC, 0xE4, 0xD8);
pub const MUTED: Color32 = Color32::from_rgb(0x9E, 0x92, 0x88);
pub const BAD: Color32 = Color32::from_rgb(0xD8, 0x5A, 0x4C);

pub fn mincho() -> FontFamily {
    FontFamily::Name("mincho".into())
}
pub fn mincho_bold() -> FontFamily {
    FontFamily::Name("mincho-bold".into())
}
pub fn title(size: f32) -> FontId {
    FontId::new(size, mincho_bold())
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();
    f.font_data.insert(
        "mincho".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/ShipporiMincho-Regular.subset.ttf"))),
    );
    f.font_data.insert(
        "mincho-bold".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/ShipporiMincho-Bold.subset.ttf"))),
    );
    // egui's own faces stay behind each family for anything the subset lacks (the check marks)
    let fallback = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with = |first: &str| {
        let mut v = vec![first.to_string()];
        v.extend(fallback.iter().cloned());
        v
    };
    f.families.insert(mincho(), with("mincho"));
    f.families.insert(mincho_bold(), with("mincho-bold"));
    f.families.insert(FontFamily::Proportional, with("mincho"));
    ctx.set_fonts(f);
}

/// A tiny deterministic hash -> [0,1): the embers' fixed traits.
fn h(i: u32, k: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1) ^ k.wrapping_mul(0x85EB_CA77);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65536.0
}

fn vgradient(p: &egui::Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), top);
    mesh.colored_vertex(r.right_top(), top);
    mesh.colored_vertex(r.right_bottom(), bottom);
    mesh.colored_vertex(r.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(Shape::mesh(mesh));
}

/// One ridge of ink-wash mountains: a jagged silhouette filled down to the bottom.
fn ridge(p: &egui::Painter, r: Rect, base: f32, amp: f32, seed: u32, color: Color32) {
    let n = 48;
    let mut pts = Vec::with_capacity(n + 3);
    for i in 0..=n {
        let fx = i as f32 / n as f32;
        let x = r.left() + fx * r.width();
        let a = (fx * 6.3 + seed as f32).sin() * 0.55
            + (fx * 13.1 + seed as f32 * 1.7).sin() * 0.25
            + (h(i as u32, seed) - 0.5) * 0.35;
        let y = r.top() + r.height() * base - a * amp;
        pts.push(Pos2::new(x, y));
    }
    pts.push(r.right_bottom());
    pts.push(r.left_bottom());
    // concave outline: fan triangles from the bottom edge, one column at a time
    let mut mesh = egui::Mesh::default();
    for w in pts[..=n].windows(2) {
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(w[0], color);
        mesh.colored_vertex(w[1], color);
        mesh.colored_vertex(Pos2::new(w[1].x, r.bottom()), color);
        mesh.colored_vertex(Pos2::new(w[0].x, r.bottom()), color);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    p.add(Shape::mesh(mesh));
}

/// The right-hand panel's backdrop: dusk gradient, the sun, ink mountains, the watermark and embers.
pub fn hero_backdrop(ui: &Ui, r: Rect, t: f64) {
    let p = ui.painter_at(r);
    vgradient(&p, r, Color32::from_rgb(0x3A, 0x10, 0x0E), Color32::from_rgb(0x0C, 0x07, 0x07));
    // the watermark, behind everything: 龍が如く down the left edge, barely there
    for (i, ch) in ["龍", "が", "如", "く"].iter().enumerate() {
        p.text(
            Pos2::new(r.left() + 22.0, r.top() + 24.0 + i as f32 * 92.0),
            Align2::LEFT_TOP,
            *ch,
            title(84.0),
            Color32::from_rgba_unmultiplied(0xD4, 0xAF, 0x55, 7),
        );
    }
    // sun with a soft glow (many faint rings, no visible steps)
    let c = Pos2::new(r.left() + r.width() * 0.60, r.top() + r.height() * 0.30);
    let rad = r.width() * 0.26;
    for k in (1..=48).rev() {
        let a = (3.0 * (1.0 - k as f32 / 48.0).powf(2.0)) as u8 + 1;
        p.circle_filled(c, rad + k as f32 * 3.2, Color32::from_rgba_unmultiplied(0xE0, 0x40, 0x30, a));
    }
    p.circle_filled(c, rad, Color32::from_rgb(0xB8, 0x24, 0x1E));
    p.circle_filled(c, rad * 0.97, Color32::from_rgb(0xC4, 0x2C, 0x24));
    // mountains, far to near
    ridge(&p, r, 0.52, 46.0, 3, Color32::from_rgba_unmultiplied(0x2A, 0x0E, 0x0C, 235));
    ridge(&p, r, 0.60, 38.0, 11, Color32::from_rgba_unmultiplied(0x1C, 0x0A, 0x09, 245));
    ridge(&p, r, 0.70, 30.0, 23, Color32::from_rgb(0x12, 0x08, 0x08));
    // drifting embers
    for i in 0..70u32 {
        let speed = 14.0 + h(i, 1) * 26.0;
        let x0 = h(i, 2);
        let life = r.height() + 40.0;
        let y = r.bottom() + 20.0 - ((t as f32 * speed + h(i, 3) * life) % life);
        let x = r.left() + x0 * r.width() + (t as f32 * (0.4 + h(i, 4)) + i as f32).sin() * 12.0;
        let flick = 0.55 + 0.45 * (t as f32 * (2.0 + h(i, 5) * 3.0) + i as f32).sin();
        let size = 0.8 + h(i, 6) * 1.9;
        let a = (60.0 + 150.0 * flick * (1.0 - ((r.bottom() - y) / life).clamp(0.0, 1.0))) as u8;
        p.circle_filled(Pos2::new(x, y), size, Color32::from_rgba_unmultiplied(0xF2, 0xB8, 0x5A, a));
    }
    // shade under the title block so the text reads over the art
    vgradient(
        &p,
        Rect::from_min_max(Pos2::new(r.left(), r.top() + r.height() * 0.45), r.right_bottom()),
        Color32::TRANSPARENT,
        Color32::from_rgba_unmultiplied(0x08, 0x05, 0x05, 225),
    );
}

/// The Play button: crimson, a gold edge, a glow on hover.
pub fn play_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    let size = Vec2::new(250.0, 66.0);
    let (r, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let hot = enabled && resp.hovered();
    if hot {
        for k in 1..=6 {
            p.rect_filled(
                r.expand(k as f32 * 2.5),
                CornerRadius::same(4 + k as u8),
                Color32::from_rgba_unmultiplied(0xE0, 0x40, 0x30, 14),
            );
        }
    }
    let (top, bottom) = if !enabled {
        (Color32::from_rgb(0x4A, 0x2C, 0x29), Color32::from_rgb(0x36, 0x20, 0x1E))
    } else if hot {
        (Color32::from_rgb(0xD0, 0x34, 0x2C), Color32::from_rgb(0x8E, 0x16, 0x16))
    } else {
        (Color32::from_rgb(0xB4, 0x26, 0x22), Color32::from_rgb(0x7A, 0x12, 0x12))
    };
    vgradient(p, r, top, bottom);
    p.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.5, if enabled { GOLD } else { GOLD_DIM }), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, label, title(28.0), if enabled { Color32::WHITE } else { MUTED });
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed })
}

/// A section heading: a gold diamond, the name in mincho, a hairline to the right.
pub fn section(ui: &mut Ui, name: &str) {
    ui.add_space(16.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let p = ui.painter();
    let c = Pos2::new(r.left() + 6.0, r.center().y);
    p.add(PathShape::convex_polygon(
        vec![c + Vec2::new(0.0, -5.0), c + Vec2::new(5.0, 0.0), c + Vec2::new(0.0, 5.0), c + Vec2::new(-5.0, 0.0)],
        GOLD,
        Stroke::NONE,
    ));
    let g = p.text(Pos2::new(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, name, title(19.0), CRIMSON_HI);
    p.line_segment(
        [Pos2::new(g.right() + 12.0, r.center().y), Pos2::new(r.right(), r.center().y)],
        Stroke::new(1.0, LINE),
    );
    ui.add_space(4.0);
}

/// A card: a slightly lighter panel with a fine warm edge.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A tab of the bottom bar; the active one is gold with a crimson bar under it.
pub fn tab(ui: &mut Ui, name: &str, active: bool) -> Response {
    let font = title(17.0);
    let galley = ui.painter().layout_no_wrap(name.to_uppercase(), font.clone(), TEXT);
    let size = galley.size() + Vec2::new(24.0, 18.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let col = if active {
        GOLD
    } else if resp.hovered() {
        TEXT
    } else {
        MUTED
    };
    ui.painter().text(r.center() - Vec2::new(0.0, 2.0), Align2::CENTER_CENTER, name.to_uppercase(), font, col);
    if active {
        ui.painter().line_segment(
            [Pos2::new(r.left() + 8.0, r.bottom() - 1.5), Pos2::new(r.right() - 8.0, r.bottom() - 1.5)],
            Stroke::new(3.0, CRIMSON_HI),
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
