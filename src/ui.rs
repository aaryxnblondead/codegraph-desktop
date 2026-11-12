//! Presentation layer: palette and custom-painted widgets.
//!
//! This is the one module in the library that touches egui. Everything else
//! (`graph`, `indexer`, `metrics`, `analysis`, ...) stays GUI-free so it can be
//! tested and embedded headlessly.
//!
//! Every painter-drawn control publishes an AccessKit node through
//! `Response::widget_info`; otherwise screen readers would see an unlabelled
//! clickable rectangle.

use egui::{Color32, RichText, Vec2};

use crate::graph_view::GraphView;
use crate::model::SymbolId;
use crate::warnings::Severity;

pub const TEXT: Color32 = Color32::from_rgb(231, 236, 241);
pub const TEXT_DIM: Color32 = Color32::from_rgb(154, 165, 176);
pub const MUTED: Color32 = Color32::from_rgb(139, 149, 161);
pub const ACCENT: Color32 = Color32::from_rgb(92, 141, 255);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(52, 82, 148);
pub const DANGER: Color32 = Color32::from_rgb(240, 96, 96);
pub const WARN: Color32 = Color32::from_rgb(235, 172, 66);
pub const OK: Color32 = Color32::from_rgb(94, 199, 132);
pub const EDGE: Color32 = Color32::from_rgb(58, 70, 82);
pub const BG: Color32 = Color32::from_rgb(9, 11, 14);
pub const PANEL: Color32 = Color32::from_rgb(15, 18, 22);
pub const CARD: Color32 = Color32::from_rgb(21, 25, 31);
pub const CARD_HOVER: Color32 = Color32::from_rgb(28, 34, 41);
pub const HAIRLINE: Color32 = Color32::from_rgb(36, 43, 51);

// ---- design primitives -------------------------------------------------

/// Card surface: rounded, hairline border, comfortable padding.
pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0, HAIRLINE))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 10))
}

pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    card_frame().show(ui, add).inner
}

/// Small rounded status chip. Built from a real label so screen readers see
/// the text instead of a painted rectangle.
pub fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.linear_multiply(0.16))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).monospace().size(10.0).color(color));
        });
}

/// Big number with a caption, used for the summary strip.
pub fn stat_tile(ui: &mut egui::Ui, value: &str, label: &str, color: Color32) {
    card(ui, |ui| {
        ui.set_min_width(104.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(value).size(22.0).strong().color(color));
            ui.label(RichText::new(label).size(10.5).color(TEXT_DIM));
        });
    });
}

pub fn section_title(ui: &mut egui::Ui, text: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(text).size(11.0).strong().color(TEXT_DIM));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).size(10.5).color(MUTED));
        }
    });
}

/// Files that changed recently, for the "live" highlights.
#[derive(Debug, Clone, Default)]
pub struct Touched {
    /// Project-relative paths, most recent first.
    pub paths: Vec<String>,
}

impl Touched {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// A node is lit when it is the changed file itself (file view / ego view)
    /// or when it is a module containing the changed file (module overview).
    pub fn matches(&self, node_file: &str) -> bool {
        self.paths.iter().any(|p| {
            p == node_file
                || (p.len() > node_file.len()
                    && p.starts_with(node_file)
                    && p.as_bytes().get(node_file.len()) == Some(&b'/'))
        })
    }

    pub fn contains_file(&self, file: &str) -> bool {
        self.paths.iter().any(|p| p == file)
    }
}

/// Copy `text` to the clipboard from a small button.
pub fn copy_button(ui: &mut egui::Ui, text: &str, label: &str) -> bool {
    let response = ui.add(
        egui::Button::new(RichText::new(label).size(11.0))
            .fill(CARD)
            .corner_radius(egui::CornerRadius::same(6)),
    );
    response.clone().on_hover_text("Copy to clipboard");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Copy {label}"))
    });
    if response.clicked() {
        ui.ctx().copy_text(text.to_string());
        true
    } else {
        false
    }
}

/// A clickable list row with a hover/selected surface, a chip, a title and a
/// dim subtitle. Returns whether it was clicked.
///
/// The row is custom-painted for speed, so it must publish its own accessible
/// node: without `widget_info` a screen reader would announce an unlabelled
/// clickable area.
pub fn list_row(
    ui: &mut egui::Ui,
    selected: bool,
    live: bool,
    badge: &str,
    badge_color: Color32,
    title: &str,
    subtitle: &str,
) -> bool {
    let width = ui.available_width();
    let height = 42.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let hovered = response.hovered();
    if selected || hovered {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(8),
            if selected {
                ACCENT.linear_multiply(0.18)
            } else {
                CARD_HOVER
            },
        );
    }
    if selected {
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(8),
            egui::Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    // Keyboard focus ring, drawn separately from selection.
    if ui.memory(|m| m.has_focus(response.id)) {
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(8),
            egui::Stroke::new(2.0, Color32::WHITE),
            egui::StrokeKind::Inside,
        );
    }

    let badge_rect = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(10.0, 12.0),
        egui::vec2(44.0, 18.0),
    );
    ui.painter().rect_filled(
        badge_rect,
        egui::CornerRadius::same(5),
        badge_color.linear_multiply(0.16),
    );
    ui.painter().text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        badge,
        egui::FontId::monospace(9.5),
        badge_color,
    );

    let text_x = badge_rect.right() + 10.0;
    ui.painter().text(
        egui::pos2(text_x, rect.top() + 20.0),
        egui::Align2::LEFT_CENTER,
        truncate(title, 46),
        egui::FontId::proportional(13.0),
        TEXT,
    );
    ui.painter().text(
        egui::pos2(text_x, rect.top() + 34.0),
        egui::Align2::LEFT_CENTER,
        truncate(subtitle, 58),
        egui::FontId::proportional(10.5),
        MUTED,
    );

    if live {
        // Recent change marker: a warm dot, also described for screen readers.
        let dot = egui::pos2(rect.right() - 14.0, rect.center().y);
        ui.painter().circle_filled(dot, 4.0, WARN);
        ui.painter()
            .circle_stroke(dot, 7.0, egui::Stroke::new(1.0, WARN.linear_multiply(0.35)));
    }

    // What a screen reader announces: the whole row as one selectable button.
    let mut label = if subtitle.is_empty() {
        format!("{badge} {title}")
    } else {
        format!("{badge} {title} — {subtitle}")
    };
    if live {
        label.push_str(", recently modified");
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, selected, label.clone())
    });

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

/// Clickable column header with an explanation tooltip.
///
/// Widths come from the grid's own content, so headers line up with the cells
/// instead of floating over them.
pub fn column_header(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let text = RichText::new(label)
        .size(10.0)
        .color(if active { ACCENT } else { MUTED });
    let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let hint = column_help(label);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            if hint.is_empty() {
                label.to_string()
            } else {
                format!("{label}, column — {hint}")
            },
        )
    });
    response.on_hover_text(hint).clicked()
}

/// Plain-language definition for each metric column.
pub fn column_help(label: &str) -> &'static str {
    match label {
        "Ca" => "Afferent coupling: how many other units depend on this one.",
        "Ce" => "Efferent coupling: how many other units this one depends on.",
        "I" => "Instability = Ce / (Ca + Ce). 0 is stable, 1 is unstable.",
        "A" => "Abstractness = abstract types / all types. No types means no value.",
        "D" => "Distance from the main sequence = |A + I - 1|. Lower is better.",
        "cohesion" => "Connected groups inside the unit, and the largest group's share.",
        "cx" => "Sum of cyclomatic complexity across the unit's functions.",
        "files" => "Files assigned to this module.",
        "module" => "Module name from your config, or derived from the folder path.",
        "commits" => "Commits touching the file in the last 180 days.",
        "added" => "Lines added in the window.",
        "deleted" => "Lines deleted in the window.",
        _ => "",
    }
}

/// A filter chip that visibly reads as on or off.
pub fn filter_toggle(ui: &mut egui::Ui, on: &mut bool, label: &str, count: usize, color: Color32) {
    let text = format!("{label} {count}");
    let active = *on;
    let button = egui::Button::new(RichText::new(text).size(11.5).color(if active {
        Color32::BLACK
    } else {
        TEXT_DIM
    }))
    .fill(if active { color } else { CARD })
    .stroke(egui::Stroke::new(
        1.0,
        if active { color } else { HAIRLINE },
    ))
    .corner_radius(egui::CornerRadius::same(8));
    if ui.add(button).clicked() {
        *on = !*on;
    }
}

pub fn draw_graph(
    ui: &mut egui::Ui,
    view: &GraphView,
    pan: &mut Vec2,
    zoom: &mut f32,
    touched: &Touched,
) -> Option<SymbolId> {
    let size = ui.available_size();
    let (response, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());
    let rect = response.rect;
    painter.rect_filled(rect, 0.0, BG);

    // Pan.
    if response.dragged() {
        *pan += response.drag_delta();
    }

    // Zoom around the pointer.
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll != 0.0 {
        if let Some(pointer) = response.hover_pos() {
            let before = (pointer - rect.center() - *pan) / *zoom;
            *zoom = (*zoom * (1.0 + scroll * 0.0015)).clamp(0.08, 8.0);
            *pan = pointer - rect.center() - before * *zoom;
        }
    }

    // Subtle dot grid so the canvas reads as a surface, not a void.
    let step = 32.0;
    let offset = *pan - (rect.center().to_vec2() * 0.0);
    let grid = Color32::from_rgb(26, 31, 37);
    let mut gx = (rect.left() + offset.x.rem_euclid(step)) - step;
    while gx < rect.right() {
        let mut gy = (rect.top() + offset.y.rem_euclid(step)) - step;
        while gy < rect.bottom() {
            painter.circle_filled(egui::pos2(gx, gy), 1.0, grid);
            gy += step;
        }
        gx += step;
    }

    let center = rect.center();
    let z = *zoom;
    let pan_v = *pan;
    let to_screen = |x: f32, y: f32| center + egui::vec2(x * z, y * z) + pan_v;

    let n = view.node_count();
    let label_all = n <= 60;

    // Edges as arrows (only when not overwhelmingly dense).
    if view.edge_count() <= 1500 {
        for &(a, b) in view.edges() {
            if a >= n || b >= n {
                continue;
            }
            let (ax, ay) = view.positions()[a];
            let (bx, by) = view.positions()[b];
            let s1 = to_screen(ax, ay);
            let s2 = to_screen(bx, by);
            if !s1.x.is_finite() || !s1.y.is_finite() || !s2.x.is_finite() || !s2.y.is_finite() {
                continue;
            }
            draw_arrow(
                &painter,
                s1,
                s2,
                view.radius(a) * z,
                view.radius(b) * z,
                z,
                EDGE,
            );
        }
    }

    // Nodes.
    let mut occupied: Vec<egui::Rect> = Vec::new();
    let mut centers: Vec<(egui::Pos2, f32, SymbolId, String, String)> = Vec::with_capacity(n);
    for (i, node) in view.nodes().iter().enumerate() {
        let (x, y) = view.positions()[i];
        let pos = to_screen(x, y);
        if !pos.x.is_finite() || !pos.y.is_finite() {
            continue;
        }
        let r = view.radius(i) * z;
        let selected = view.selected() == Some(&node.id);
        let accent = kind_color(&node.kind);
        let (fill, border, width) = if selected {
            (accent.linear_multiply(0.9), Color32::WHITE, 2.0)
        } else {
            (
                accent.linear_multiply(0.22),
                accent.linear_multiply(0.8),
                1.2,
            )
        };
        painter.circle_filled(pos, r, fill);
        painter.circle_stroke(pos, r, egui::Stroke::new(width * z.max(0.6), border));

        if z > 0.3 && (label_all || view.radius(i) >= 12.0) {
            let text = truncate(&node.name, 30);
            let font = egui::FontId::proportional((12.0 * z).clamp(9.0, 15.0));
            let galley = painter.layout_no_wrap(text, font, TEXT);
            let pad = egui::vec2(6.0, 3.0);

            // Prefer the right side, flip left near the right edge, and skip
            // labels that would collide with one already drawn.
            let mut label_pos = pos + egui::vec2(r + 5.0, 0.0);
            let right_edge = label_pos.x + galley.size().x + pad.x * 2.0;
            if right_edge > rect.right() - 6.0 {
                label_pos = pos - egui::vec2(r + 5.0 + galley.size().x + pad.x * 2.0, 0.0);
            }
            let bg = egui::Rect::from_min_size(label_pos - pad, galley.size() + pad * 2.0);
            let collides = occupied.iter().any(|o| o.expand(3.0).intersects(bg));
            if !collides && bg.left() > rect.left() && bg.right() < rect.right() {
                painter.rect_filled(bg, egui::CornerRadius::same(4), BG.linear_multiply(0.82));
                painter.galley(label_pos, galley, TEXT);
                occupied.push(bg);
            }
        }
        centers.push((
            pos,
            r.max(6.0),
            node.id.clone(),
            node.name.clone(),
            node.file.clone(),
        ));
    }

    // Hover tooltip: name, kind, file and degree for the node under the pointer.
    let hovered = ui.input(|i| i.pointer.hover_pos()).and_then(|p| {
        centers
            .iter()
            .filter(|(c, r, ..)| (*c - p).length() <= *r + 6.0)
            .min_by(|a, b| {
                (a.0 - p)
                    .length()
                    .partial_cmp(&(b.0 - p).length())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, _, id, ..)| id.clone())
    });
    if let Some(id) = hovered {
        if let Some(node) = view.nodes().iter().find(|n| n.id == id) {
            let out = view
                .edges()
                .iter()
                .filter(|(a, _)| view.nodes().get(*a).map(|n| n.id == id).unwrap_or(false))
                .count();
            let inc = view
                .edges()
                .iter()
                .filter(|(_, b)| view.nodes().get(*b).map(|n| n.id == id).unwrap_or(false))
                .count();
            let file = node.file.clone();
            let name = node.name.clone();
            let kind = node.kind.short().to_string();
            response.clone().on_hover_ui(move |ui| {
                ui.label(RichText::new(&name).strong());
                ui.label(
                    RichText::new(format!("{kind} · {file}"))
                        .size(11.0)
                        .color(MUTED),
                );
                ui.label(
                    RichText::new(format!("{inc} in · {out} out"))
                        .size(11.0)
                        .color(MUTED),
                );
            });
        }
    }

    // Floating zoom controls (bottom-left) + caption (top-left).
    let button = egui::vec2(28.0, 22.0);
    let mut bx = rect.left() + 12.0;
    let by = rect.bottom() - 34.0;
    let minus = egui::Rect::from_min_size(egui::pos2(bx, by), button);
    bx += button.x + 6.0;
    let zoom_label = egui::Rect::from_min_size(egui::pos2(bx, by), egui::vec2(48.0, button.y));
    bx += 54.0;
    let plus = egui::Rect::from_min_size(egui::pos2(bx, by), button);
    bx += button.x + 6.0;
    let fit = egui::Rect::from_min_size(egui::pos2(bx, by), egui::vec2(34.0, button.y));

    for (r, label) in [(minus, "-"), (plus, "+"), (fit, "fit")] {
        painter.rect_filled(r, egui::CornerRadius::same(6), CARD);
        painter.rect_stroke(
            r,
            egui::CornerRadius::same(6),
            egui::Stroke::new(1.0, HAIRLINE),
            egui::StrokeKind::Inside,
        );
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(13.0),
            TEXT_DIM,
        );
    }
    painter.text(
        zoom_label.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.0}%", *zoom * 100.0),
        egui::FontId::monospace(10.5),
        MUTED,
    );

    // Caption + hint.
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Label,
            true,
            format!(
                "Dependency graph, {} nodes and {} edges. Each node is a separate button.",
                n,
                view.edge_count()
            ),
        )
    });
    painter.text(
        rect.left_top() + egui::vec2(14.0, 12.0),
        egui::Align2::LEFT_TOP,
        format!("{} nodes · {} edges", n, view.edge_count()),
        egui::FontId::proportional(11.0),
        MUTED,
    );
    painter.text(
        rect.right_top() + egui::vec2(-14.0, 12.0),
        egui::Align2::RIGHT_TOP,
        "drag to pan · scroll to zoom · click a node to drill in",
        egui::FontId::proportional(10.5),
        MUTED,
    );

    // Each node is a real interactive widget (not just painted geometry), so
    // the accessibility tree exposes it as a focusable, labelled button.
    let mut clicked: Option<SymbolId> = None;
    for (pos, r, id, name, file) in &centers {
        let hit = egui::Rect::from_center_size(*pos, egui::Vec2::splat((*r * 2.0).max(24.0)));
        let node_response = ui.interact(
            hit,
            ui.scope_id().with(("graph-node", id.0.as_str())),
            egui::Sense::click(),
        );
        let label = if touched.matches(file) {
            format!("{name} — {file}, recently modified")
        } else {
            format!("{name} — {file}")
        };
        node_response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::Button,
                true,
                view.selected() == Some(id),
                label.clone(),
            )
        });
        if node_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if node_response.clicked() {
            clicked = Some(id.clone());
        }
    }
    if clicked.is_some() {
        return clicked;
    }

    if response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            if minus.contains(p) {
                *zoom = (*zoom * 0.85).clamp(0.08, 8.0);
                return None;
            }
            if plus.contains(p) {
                *zoom = (*zoom * 1.18).clamp(0.08, 8.0);
                return None;
            }
            if fit.contains(p) {
                *zoom = 1.0;
                *pan = Vec2::ZERO;
                return None;
            }
        }
    }
    None
}

pub fn draw_arrow(
    painter: &egui::Painter,
    s1: egui::Pos2,
    s2: egui::Pos2,
    r1: f32,
    r2: f32,
    zoom: f32,
    color: Color32,
) {
    let d = s2 - s1;
    let dist = d.length();
    if dist < 1.0 || !dist.is_finite() {
        return;
    }
    let u = d / dist;
    let start = s1 + u * r1;
    let end = s2 - u * r2;
    if (end - start).dot(u) <= 0.0 {
        return;
    }
    painter.line_segment([start, end], egui::Stroke::new(1.0 * zoom, color));

    let head = (10.0 * zoom).clamp(5.0, 18.0);
    let (sin, cos) = 0.42f32.sin_cos();
    let left = end - egui::vec2(u.x * cos - u.y * sin, u.y * cos + u.x * sin) * head;
    let right = end - egui::vec2(u.x * cos + u.y * sin, u.y * cos - u.x * sin) * head;
    painter.add(egui::Shape::convex_polygon(
        vec![end, left, right],
        color,
        egui::Stroke::new(0.0, Color32::TRANSPARENT),
    ));
}

/// Group digits for readability: 20172 -> 20,172.
pub fn fmt_num(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Shorten with an ellipsis.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Colour for a warning severity (never the only signal: chips carry a word).
pub fn severity_color(severity: Severity) -> Color32 {
    match severity {
        Severity::Error => DANGER,
        Severity::Warn => WARN,
        Severity::Info => MUTED,
    }
}

/// Colour for a symbol kind, used for badges and graph nodes.
pub fn kind_color(kind: &crate::model::SymbolKind) -> Color32 {
    use crate::model::SymbolKind as K;
    match kind {
        K::Function | K::Method => Color32::from_rgb(122, 196, 255),
        K::Struct | K::Class => Color32::from_rgb(186, 156, 255),
        K::Trait | K::Interface => Color32::from_rgb(120, 224, 200),
        K::Enum => Color32::from_rgb(240, 190, 120),
        K::Module | K::Namespace => Color32::from_rgb(150, 165, 180),
        K::Constant | K::Variable => Color32::from_rgb(220, 220, 140),
        K::TypeAlias => Color32::from_rgb(170, 210, 240),
        K::Other(_) => MUTED,
    }
}

/// Pill-style tab. Returns true when it was clicked.
pub fn tab_button(ui: &mut egui::Ui, selected: bool, label: &str, count: Option<usize>) -> bool {
    let text = match count {
        Some(n) if n > 0 => format!("{label}  {n}"),
        _ => label.to_string(),
    };
    let button = egui::Button::new(RichText::new(text).size(12.5).color(if selected {
        Color32::BLACK
    } else {
        TEXT_DIM
    }))
    .fill(if selected {
        ACCENT
    } else {
        Color32::TRANSPARENT
    })
    .corner_radius(egui::CornerRadius::same(8));
    ui.add(button).clicked()
}
