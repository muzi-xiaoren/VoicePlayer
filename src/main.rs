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
    init_logging();

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

/// 日志写到数据目录下的 `voiceplayer.log`。
///
/// release 构建下 `windows_subsystem = "windows"` 没有控制台，
/// 打到 stderr 的日志等于扔掉了 —— 而「热键突然不响」这类问题恰恰只能靠日志定位，
/// 所以固定落到文件，用户直接把它发过来就行。级别可以用 RUST_LOG 覆盖。
fn init_logging() {
    let mut b = env_logger::Builder::new();
    b.parse_filters(&std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()));
    let path = config::data_root().join("voiceplayer.log");
    let _ = std::fs::create_dir_all(config::data_root());
    // 每次启动重开一份，别让它无限长。
    match std::fs::File::create(&path) {
        Ok(f) => {
            b.target(env_logger::Target::Pipe(Box::new(f)));
        }
        Err(e) => eprintln!("无法写日志文件 {}: {e}", path.display()),
    }
    let _ = b.try_init();
    log::info!("VoicePlayer {} 启动", env!("CARGO_PKG_VERSION"));
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
