//! 视觉主题：调色板 + egui 全局样式 + 背景图。
//!
//! 深浅两套配色都由 `derive()` 从一个「底色」推出来（面、凹陷、描边都是底色的明暗偏移），
//! 所以用户自定义背景色时整套界面会跟着一起走，而不是只换窗口底、卡片还留在原来的深色。
//!
//! 当前生效的模式 / 底色 / 面板透明度存在几个原子量里，界面代码用 `theme::p()` 取调色板。

use egui::{Color32, Context, FontId, Rounding, Stroke, TextStyle};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::OnceLock;

pub struct Palette {
    /// 窗口最底层背景。
    pub bg: Color32,
    /// 抬高一层：卡片 / 瓦片 / 顶栏底栏。
    pub surface: Color32,
    /// 控件填充色（按钮底、滑块轨道）。必须和 `surface` 有可见反差：
    /// 深色主题里比底色亮，浅色主题里比底色暗（卡片是白的，再往白推就看不见了）。
    pub control: Color32,
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
    /// 这套配色是不是浅色。
    pub light: bool,
}

/// 统一的圆角。
pub const R_CARD: f32 = 10.0;
pub const R_CTRL: f32 = 6.0;

/// 深色默认底色。
pub const DEFAULT_DARK_BG: [u8; 3] = [0x11, 0x13, 0x19];
/// 浅色默认底色。
pub const DEFAULT_LIGHT_BG: [u8; 3] = [0xEF, 0xF1, 0xF5];

// ── 当前生效的主题状态（apply 时写入，p() 读取）──
static LIGHT_MODE: AtomicBool = AtomicBool::new(false);
/// 打包成 0xRRGGBB 的当前底色。
static BG_RGB: AtomicU32 = AtomicU32::new(0x11_13_19);
/// 面板不透明度：有背景图时降下来让图透出来。
static PANEL_ALPHA: AtomicU8 = AtomicU8::new(255);
/// 整窗不透明度（用户设置，0–255）。会再乘到面板和窗口底色上。
static WINDOW_ALPHA: AtomicU8 = AtomicU8::new(255);
/// 缓存：底色 + 明暗没变就不重算调色板。
static CACHE: OnceLock<std::sync::Mutex<Option<(u32, bool, &'static Palette)>>> = OnceLock::new();

/// 把颜色朝白（`lighten`）或朝黑推 `t`（0–1）。
fn shift(c: Color32, t: f32, lighten: bool) -> Color32 {
    let target = if lighten { 255.0 } else { 0.0 };
    let f = |v: u8| (v as f32 + (target - v as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// 感知亮度（0–1），用来判断一个自定义底色该配深色还是浅色文字。
fn luminance(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

/// 从一个底色推出整套配色。
fn derive(bg: Color32) -> Palette {
    let light = luminance(bg) > 0.5;
    // 深色底：面往亮里抬；浅色底：面往白里抬（卡片比底更白）。两边都是 lighten，
    // 只有凹陷和描边的方向不同。
    let surface = shift(bg, if light { 0.55 } else { 0.055 }, true);
    // 浅色主题的卡片接近纯白，控件再往白推就和卡片糊在一起（滑块轨道会整条看不见），
    // 所以浅色下控件反过来比底色更暗。
    let control = if light {
        shift(bg, 0.13, false)
    } else {
        shift(bg, 0.11, true)
    };
    let sunken = if light { shift(bg, 0.16, false) } else { shift(bg, 0.35, false) };
    let border = shift(bg, if light { 0.22 } else { 0.14 }, !light);
    Palette {
        bg,
        surface,
        control,
        sunken,
        border,
        text: if light {
            Color32::from_rgb(0x1B, 0x1E, 0x26)
        } else {
            Color32::from_rgb(0xE4, 0xE8, 0xF0)
        },
        dim: if light {
            Color32::from_rgb(0x6B, 0x72, 0x84)
        } else {
            Color32::from_rgb(0x87, 0x8F, 0xA3)
        },
        accent: if light {
            Color32::from_rgb(0x35, 0x5C, 0xD6)
        } else {
            Color32::from_rgb(0x6C, 0x8C, 0xFF)
        },
        accent_soft: if light {
            Color32::from_rgb(0xD8, 0xE1, 0xFB)
        } else {
            Color32::from_rgb(0x2A, 0x33, 0x5C)
        },
        ok: if light { Color32::from_rgb(0x1E, 0x8E, 0x4F) } else { Color32::from_rgb(0x3F, 0xBF, 0x7F) },
        warn: if light { Color32::from_rgb(0xA9, 0x6C, 0x0E) } else { Color32::from_rgb(0xE0, 0xA3, 0x3E) },
        danger: if light { Color32::from_rgb(0xC2, 0x36, 0x36) } else { Color32::from_rgb(0xE0, 0x5C, 0x5C) },
        light,
    }
}

/// 当前调色板。界面代码统一走这个，别自己写死颜色。
pub fn p() -> &'static Palette {
    let rgb = BG_RGB.load(Ordering::Relaxed);
    let light = LIGHT_MODE.load(Ordering::Relaxed);
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut g = match cache.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    if let Some((r, l, pal)) = *g {
        if r == rgb && l == light {
            return pal;
        }
    }
    let bg = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    // 调色板要活到程序结束，泄漏一份。底色变化次数是用户手动操作级别的，不会堆积。
    let pal: &'static Palette = Box::leak(Box::new(derive(bg)));
    *g = Some((rgb, light, pal));
    pal
}

fn with_alpha(c: Color32, a: u8) -> Color32 {
    if a == 255 {
        c
    } else {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
    }
}

/// 整窗半透明时，面板填充的淡出系数（0–1）。
///
/// 关键点：清除色、中央区、顶栏底栏、卡片是**一层层叠着画**的，如果每层都带 alpha，
/// 实际不透明度是 `1-(1-a)^n` —— 三层 0.3 叠出来是 0.66，拖到最低也几乎看不出效果。
/// 所以 alpha 只让最底下的清除色承担一次，上面的面板在 1.00→0.85 之间淡出到全透明，
/// 0.85 以下就是干净的单层玻璃，描边还在，结构照样看得清。
fn panel_fade() -> f32 {
    let wo = WINDOW_ALPHA.load(Ordering::Relaxed) as f32 / 255.0;
    ((wo - 0.85) / 0.15).clamp(0.0, 1.0)
}

/// 面板 / 卡片的填充色。有背景图时半透明让图透出来；整窗半透明时直接不填。
pub fn surface_fill() -> Color32 {
    let fade = panel_fade();
    if fade <= 0.0 {
        return Color32::TRANSPARENT;
    }
    let a = (PANEL_ALPHA.load(Ordering::Relaxed) as f32 * fade) as u8;
    with_alpha(p().surface, a)
}

/// 中央区域的填充色：有背景图或整窗半透明时完全透明，让下面透上来。
pub fn body_fill() -> Color32 {
    if PANEL_ALPHA.load(Ordering::Relaxed) != 255 || panel_fade() < 1.0 {
        return Color32::TRANSPARENT;
    }
    p().bg
}

/// 交给 eframe 的窗口清除色。整窗不透明度 < 1 时这里透出来的就是桌面。
pub fn clear_color() -> [f32; 4] {
    with_alpha(p().bg, WINDOW_ALPHA.load(Ordering::Relaxed)).to_normalized_gamma_f32()
}

/// 设置整窗不透明度（0.0–1.0）。
pub fn set_window_opacity(v: f32) {
    WINDOW_ALPHA.store((v.clamp(0.0, 1.0) * 255.0).round() as u8, Ordering::Relaxed);
}

/// 应用主题。`light` = 用浅色配色，`bg` = 底色（None 用该模式的默认底色），
/// `has_bg_image` = 是否有背景图（有则把面板调成半透明）。
pub fn apply(ctx: &Context, light: bool, bg: Option<[u8; 3]>, has_bg_image: bool) {
    let rgb = bg.unwrap_or(if light { DEFAULT_LIGHT_BG } else { DEFAULT_DARK_BG });
    LIGHT_MODE.store(light, Ordering::Relaxed);
    BG_RGB.store(
        ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | rgb[2] as u32,
        Ordering::Relaxed,
    );
    PANEL_ALPHA.store(if has_bg_image { 210 } else { 255 }, Ordering::Relaxed);

    let pal = p();
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
    let mut v = if pal.light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    v.panel_fill = pal.bg;
    v.window_fill = pal.surface;
    v.window_stroke = Stroke::new(1.0, pal.border);
    v.window_rounding = Rounding::same(R_CARD);
    v.extreme_bg_color = pal.sunken; // 输入框 / 滑块槽
    v.faint_bg_color = pal.surface;
    v.override_text_color = Some(pal.text);
    v.selection.bg_fill = pal.accent.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, pal.text);
    v.hyperlink_color = pal.accent;
    v.warn_fg_color = pal.warn;
    v.error_fg_color = pal.danger;

    let on_accent = if pal.light { Color32::WHITE } else { Color32::WHITE };
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = pal.surface;
    w.noninteractive.weak_bg_fill = pal.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, pal.dim);
    w.noninteractive.rounding = Rounding::same(R_CTRL);

    w.inactive.bg_fill = pal.control;
    w.inactive.weak_bg_fill = pal.control;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, pal.text);
    w.inactive.rounding = Rounding::same(R_CTRL);

    w.hovered.bg_fill = pal.accent_soft;
    w.hovered.weak_bg_fill = pal.accent_soft;
    w.hovered.bg_stroke = Stroke::new(1.0, pal.accent.gamma_multiply(0.7));
    w.hovered.fg_stroke = Stroke::new(1.0, pal.text);
    w.hovered.rounding = Rounding::same(R_CTRL);
    w.hovered.expansion = 0.0; // 默认会「胀」一下，看着晃

    w.active.bg_fill = pal.accent;
    w.active.weak_bg_fill = pal.accent;
    w.active.bg_stroke = Stroke::new(1.0, pal.accent);
    w.active.fg_stroke = Stroke::new(1.0, on_accent);
    w.active.rounding = Rounding::same(R_CTRL);
    w.active.expansion = 0.0;

    w.open.bg_fill = pal.control;
    w.open.weak_bg_fill = pal.control;
    w.open.bg_stroke = Stroke::new(1.0, pal.border);
    w.open.fg_stroke = Stroke::new(1.0, pal.text);
    w.open.rounding = Rounding::same(R_CTRL);

    style.visuals = v;
    ctx.set_style(style);
}

/// 卡片容器：抬高一层的圆角面。`accented` 时描边用强调色（表示「正在播放」）。
pub fn card(accented: bool) -> egui::Frame {
    let pal = p();
    egui::Frame::none()
        .fill(surface_fill())
        .rounding(Rounding::same(R_CARD))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .stroke(if accented {
            Stroke::new(1.0, pal.accent)
        } else {
            Stroke::new(1.0, pal.border)
        })
}

/// 设置页里的分组小标题。
pub fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).size(12.0).color(p().dim).strong());
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
    ui.label(egui::RichText::new(text).size(12.0).color(p().dim));
}

/// 在最底层铺一张背景图，等比裁剪填满窗口（cover），按 `opacity` 淡化。
pub fn paint_background(ctx: &Context, tex: &egui::TextureHandle, opacity: f32) {
    let screen = ctx.screen_rect();
    let img = tex.size_vec2();
    if img.x <= 0.0 || img.y <= 0.0 || screen.width() <= 0.0 {
        return;
    }
    // cover：按较大的缩放比铺满，多出来的部分从两边均匀裁掉（体现在 uv 上）。
    let scale = (screen.width() / img.x).max(screen.height() / img.y);
    let used_w = (screen.width() / (img.x * scale)).min(1.0);
    let used_h = (screen.height() / (img.y * scale)).min(1.0);
    let uv = egui::Rect::from_min_max(
        egui::pos2((1.0 - used_w) * 0.5, (1.0 - used_h) * 0.5),
        egui::pos2((1.0 + used_w) * 0.5, (1.0 + used_h) * 0.5),
    );
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0) as u8;
    ctx.layer_painter(egui::LayerId::background())
        .image(tex.id(), screen, uv, Color32::from_white_alpha(alpha));
}
