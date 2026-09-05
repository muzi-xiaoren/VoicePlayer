// release 构建下隐藏控制台黑窗（仅 Windows）。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod hotkeys;
mod i18n;
mod platform;
mod profile;
mod theme;
mod wasapi_loopback;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let icon = load_icon();
    let cfg = config::AppConfig::load();

    let mut viewport = egui::ViewportBuilder::default()
        .with_min_inner_size([380.0, 460.0])
        .with_title("VoicePlayer")
        // 透明必须在建窗时声明；之后运行时改的只是清除色的 alpha。
        .with_transparent(true)
        .with_icon(std::sync::Arc::new(icon));
    match (cfg.window_w, cfg.window_h) {
        (Some(w), Some(h)) => viewport = viewport.with_inner_size([w, h]),
        _ => viewport = viewport.with_inner_size([440.0, 640.0]),
    }
    if let (Some(x), Some(y)) = (cfg.window_x, cfg.window_y) {
        viewport = viewport.with_position([x, y]);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "VoicePlayer",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)) as Box<dyn eframe::App>)),
    )
}

/// 从编译期嵌入的 RGBA 原始像素加载窗口图标。
fn load_icon() -> egui::IconData {
    const ICON_RGBA: &[u8] = include_bytes!("../assets/icon_256.rgba");
    egui::IconData {
        rgba: ICON_RGBA.to_vec(),
        width: 256,
        height: 256,
    }
}
