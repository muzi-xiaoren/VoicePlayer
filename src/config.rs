//! 全局配置与数据目录。
//!
//! 数据都放在系统的 AppData 目录下（Windows: `%APPDATA%\VoicePlayer\`）：
//! ```text
//! %APPDATA%\VoicePlayer\
//! ├─ config.json        # 本文件对应的全局配置
//! └─ profiles\          # 每个子文件夹 = 一个配置（profile）
//!    └─ 默认\
//!       ├─ xxx.mp3       # 用户丢进来的音频
//!       └─ _bindings.json
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 界面主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeMode {
    /// 跟随系统的深色 / 浅色设置。
    #[default]
    System,
    Dark,
    Light,
}

/// 重复按同一个快捷键时的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    /// 停掉正在播的，从头重播。
    #[default]
    Restart,
    /// 不停旧的，再叠一份一起播。
    Overlap,
    /// 第一次播放，再按一次停止。
    Toggle,
}

/// 数据根目录：`%APPDATA%\VoicePlayer\`（拿不到时退回当前目录）。
pub fn data_root() -> PathBuf {
    directories::ProjectDirs::from("com", "muzi", "VoicePlayer")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn profiles_dir() -> PathBuf {
    data_root().join("profiles")
}

pub fn config_file() -> PathBuf {
    data_root().join("config.json")
}

/// 全局配置。字段变动时 `#[serde(default)]` 保证老配置文件也能读。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 输出设备名（应选 VB-CABLE 的 `CABLE Input`）。None = 系统默认输出。
    pub output_device: Option<String>,
    /// 采集用的真麦克风名。None = 系统默认输入。
    pub input_device: Option<String>,
    /// 可选的监听设备（你的耳机），让自己也能听到音效。None = 不监听。
    pub monitor_device: Option<String>,
    /// 当前激活的 profile 名（= profiles 下的子文件夹名）。
    pub active_profile: Option<String>,
    /// 音效音量 0.0–1.5。
    pub effect_volume: f32,
    /// 是否把真麦转发到输出设备（关掉后只放音效、不转发说话声）。
    pub mic_passthrough: bool,
    /// 「停止所有音效」的全局快捷键，如 "Ctrl+Alt+X"。
    pub stop_hotkey: Option<String>,
    /// 重复按同一个快捷键时的行为。
    pub repeat_mode: RepeatMode,
    /// 外部音效文件夹配置：名字 -> 任意位置的文件夹（profiles 目录之外）。
    pub external_profiles: BTreeMap<String, PathBuf>,
    /// 开机自启（仅 Windows 生效）。
    pub autostart: bool,
    /// 语言设置。None = 跟随系统语言。
    pub language: Option<String>,
    /// 锁定快捷键。锁定后按快捷键不会触发任何音效。
    pub locked: bool,
    pub loopback_pid: Option<u32>,
    pub loopback_name: Option<String>,
    pub loopback_enabled: bool,
    pub loopback_volume: f32,
    /// 深色 / 浅色 / 跟随系统。
    pub theme_mode: ThemeMode,
    /// 自定义背景色（sRGB）。None = 用当前主题的默认底色。
    /// 卡片、输入框、描边的明暗都由它推出来。
    pub bg_color: Option<[u8; 3]>,
    /// 背景图片路径。None = 不用背景图。
    pub bg_image: Option<PathBuf>,
    /// 背景图片不透明度 0.0–1.0。
    pub bg_opacity: f32,
    /// 音量快速档位。右键音量滑块时弹出来供一键选择，可在设置里增删改。
    pub volume_presets: Vec<f32>,
    /// 整个窗口的不透明度 0.0–1.0。1.0 = 完全不透明；0.0 = 底完全透明，
    /// 只剩文字和控件浮在桌面上（它们始终不透明，所以窗口不会真的消失）。
    pub window_opacity: f32,
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
    pub window_w: Option<f32>,
    pub window_h: Option<f32>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            output_device: None,
            input_device: None,
            monitor_device: None,
            active_profile: None,
            effect_volume: 1.0,
            mic_passthrough: true,
            stop_hotkey: None,
            repeat_mode: RepeatMode::default(),
            external_profiles: BTreeMap::new(),
            autostart: false,
            language: None,
            locked: false,
            loopback_pid: None,
            loopback_name: None,
            loopback_enabled: false,
            loopback_volume: 1.0,
            theme_mode: ThemeMode::default(),
            bg_color: None,
            bg_image: None,
            bg_opacity: 0.35,
            volume_presets: vec![0.1, 0.5, 0.8, 1.0, 1.2, 1.5],
            window_opacity: 1.0,
            window_x: None,
            window_y: None,
            window_w: None,
            window_h: None,
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        match std::fs::read_to_string(config_file()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("config.json 解析失败，用默认配置：{e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        if let Some(dir) = config_file().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(config_file(), s) {
                    log::error!("写 config.json 失败：{e}");
                }
            }
            Err(e) => log::error!("序列化配置失败：{e}"),
        }
    }
}
