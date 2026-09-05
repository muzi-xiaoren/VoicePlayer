//! 视觉主题：配色 token + egui 全局样式。
//!
//! 设计参考了 Voicemod / Soundpad / OBS 这类音频工具的做法：
//! 近黑的蓝灰底 + 抬高一层的卡片面 + 单一强调色，语义色（成功 / 警告 / 危险）只用于状态。
//! 所有颜色集中在 `Palette` 里，别在界面代码里散写 `Color32::from_rgb`。

use egui::{Color32, Context, FontId, Rounding, Stroke, TextStyle};

pub struct Palette {
    /// 窗口最底层背景。
    pub bg: Color32,
    /// 抬高一层：卡片 / 瓦片。
    pub surface: Color32,
    /// 卡片的悬停态。
    pub surface_hover: Color32,
    /// 压低一层：输入框、滑块槽这类「凹进去」的元素。
    pub sunken: Color32,
    /// 分隔线 / 描边。
    pub border: Color32,
    /// 正文。
    pub text: Color32,
    /// 次要说明文字。
    pub dim: Color32,
    /// 强调色（选中、快捷键徽章、正在播放）。
    pub accent: Color32,
    /// 强调色的低饱和填充版，用作徽章底色。
    pub accent_soft: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub danger: Color32,
}

pub const P: Palette = Palette {
    bg: Color32::from_rgb(0x11, 0x13, 0x19),
    surface: Color32::from_rgb(0x1A, 0x1D, 0x27),
    surface_hover: Color32::from_rgb(0x23, 0x27, 0x34),
    sunken: Color32::from_rgb(0x0C, 0x0E, 0x13),
    border: Color32::from_rgb(0x2A, 0x2F, 0x3D),
    text: Color32::from_rgb(0xE4, 0xE8, 0xF0),
    dim: Color32::from_rgb(0x87, 0x8F, 0xA3),
    accent: Color32::from_rgb(0x6C, 0x8C, 0xFF),
    accent_soft: Color32::from_rgb(0x2A, 0x33, 0x5C),
    ok: Color32::from_rgb(0x3F, 0xBF, 0x7F),
    warn: Color32::from_rgb(0xE0, 0xA3, 0x3E),
    danger: Color32::from_rgb(0xE0, 0x5C, 0x5C),
};

/// 统一的圆角。
pub const R_CARD: f32 = 10.0;
pub const R_CTRL: f32 = 6.0;

pub fn apply(ctx: &Context) {
    let mut style = (*ctx.style()).clone();

    // ── 排版 ──
    // egui 默认字号偏小，整体抬一档，并拉开标题和正文的层级。
    style.text_styles.insert(TextStyle::Heading, FontId::proportional(19.0));
    style.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));

    // ── 间距 ──
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 24.0;
    style.spacing.slider_width = 150.0;
    style.spacing.combo_width = 200.0;
    style.spacing.menu_margin = egui::Margin::same(6.0);

    // ── 配色 ──
    let mut v = egui::Visuals::dark();
    v.panel_fill = P.bg;
    v.window_fill = P.surface;
    v.window_stroke = Stroke::new(1.0, P.border);
    v.window_rounding = Rounding::same(R_CARD);
    v.extreme_bg_color = P.sunken; // 输入框 / 滑块槽
    v.faint_bg_color = P.surface;
    v.selection.bg_fill = P.accent.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, P.text);
    v.hyperlink_color = P.accent;
    v.warn_fg_color = P.warn;
    v.error_fg_color = P.danger;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = P.surface;
    w.noninteractive.weak_bg_fill = P.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, P.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, P.dim);
    w.noninteractive.rounding = Rounding::same(R_CTRL);

    w.inactive.bg_fill = P.surface_hover;
    w.inactive.weak_bg_fill = P.surface_hover;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, P.text);
    w.inactive.rounding = Rounding::same(R_CTRL);

    w.hovered.bg_fill = P.accent_soft;
    w.hovered.weak_bg_fill = P.accent_soft;
    w.hovered.bg_stroke = Stroke::new(1.0, P.accent.gamma_multiply(0.7));
    w.hovered.fg_stroke = Stroke::new(1.0, P.text);
    w.hovered.rounding = Rounding::same(R_CTRL);
    w.hovered.expansion = 0.0; // 默认会「胀」一下，看着晃

    w.active.bg_fill = P.accent;
    w.active.weak_bg_fill = P.accent;
    w.active.bg_stroke = Stroke::new(1.0, P.accent);
    w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.active.rounding = Rounding::same(R_CTRL);
    w.active.expansion = 0.0;

    w.open.bg_fill = P.surface_hover;
    w.open.weak_bg_fill = P.surface_hover;
    w.open.bg_stroke = Stroke::new(1.0, P.border);
    w.open.fg_stroke = Stroke::new(1.0, P.text);
    w.open.rounding = Rounding::same(R_CTRL);

    style.visuals = v;
    ctx.set_style(style);
}

/// 卡片容器：抬高一层的圆角面。`accented` 时描边用强调色（表示「正在播放」）。
pub fn card(accented: bool) -> egui::Frame {
    egui::Frame::none()
        .fill(P.surface)
        .rounding(Rounding::same(R_CARD))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .stroke(if accented {
            Stroke::new(1.0, P.accent)
        } else {
            Stroke::new(1.0, P.border)
        })
}

/// 设置页里的分组小标题。
pub fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(text)
            .size(12.0)
            .color(P.dim)
            .strong(),
    );
    ui.add_space(2.0);
}

/// 一个小徽章（快捷键、状态）。
pub fn badge(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::none()
        .fill(bg)
        .rounding(Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(11.5).color(fg));
        });
}

/// 状态圆点 + 文案。
pub fn status_dot(ui: &mut egui::Ui, color: Color32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    ui.label(egui::RichText::new(text).size(12.0).color(P.dim));
}
