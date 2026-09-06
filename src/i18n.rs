//! 多语言支持（i18n）。
//!
//! 目前支持简体中文（Zh）和英文（En），默认跟随系统语言。
//! `Texts` 结构体集中存放界面所有可见文案，`Lang` 提供「检测系统语言 /
//! 从配置字符串互转 / 展示名」三个工具方法。

/// 支持的语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// 从配置里的字符串解析语言（持久化用）。None 表示配置里没有或无法识别。
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "zh" | "zh-cn" | "zh_cn" | "chinese" | "中文" => Some(Lang::Zh),
            "en" | "en-us" | "en_us" | "english" => Some(Lang::En),
            _ => None,
        }
    }

    /// 序列化成配置文件里存的短标签。
    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
        }
    }

    /// 语言选择下拉框里显示的名称。
    pub fn display(self) -> &'static str {
        match self {
            Lang::Zh => "简体中文",
            Lang::En => "English",
        }
    }

    /// 所有支持的语言（下拉框遍历用）。
    pub fn all() -> [Lang; 2] {
        [Lang::Zh, Lang::En]
    }
}

/// 检测系统当前语言，返回匹配的 Lang。检测失败时默认中文。
///
/// 注意：这里**不能**用 `Command::new("reg")` 去查注册表 —— 每 spawn 一个控制台子进程，
/// Windows 就会闪一个 cmd 黑窗（即使主程序编译成了 windows_subsystem = "windows"）。
/// 直接调 kernel32 的 GetUserDefaultLocaleName，无进程、无窗口，还更快。
#[cfg(windows)]
pub fn detect_system_lang() -> Lang {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    const LOCALE_NAME_MAX_LENGTH: usize = 85;
    let mut buf = [0u16; LOCALE_NAME_MAX_LENGTH];
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n > 0 {
        // 返回的长度含结尾的 NUL，去掉。
        let name = String::from_utf16_lossy(&buf[..(n as usize).saturating_sub(1)]);
        if name.to_lowercase().starts_with("zh") {
            return Lang::Zh;
        }
        if !name.is_empty() {
            return Lang::En;
        }
    }
    Lang::Zh
}

#[cfg(not(windows))]
pub fn detect_system_lang() -> Lang {
    // 非 Windows 平台用 LANG 环境变量粗略判断。
    if let Ok(lang) = std::env::var("LANG") {
        if lang.to_lowercase().starts_with("zh") {
            return Lang::Zh;
        }
    }
    Lang::En
}

/// 获取最终使用的语言：配置里有就用配置的，否则跟随系统。
pub fn resolve_lang(config_lang: &Option<String>) -> Lang {
    config_lang
        .as_deref()
        .and_then(Lang::from_str)
        .unwrap_or_else(detect_system_lang)
}

/// 集中存放界面所有可见文案。按 Lang 构造一份。
pub struct Texts {

    pub title: &'static str,

    // 设置区
    pub vbcable_detected: &'static str,
    pub vbcable_not_detected: &'static str,
    pub open_vbcable_url: &'static str,
    pub reinstall_detect: &'static str,

    pub output_device: &'static str,
    pub output_device_tip: &'static str,
    pub microphone: &'static str,
    pub monitor_device: &'static str,
    pub monitor_device_tip: &'static str,
    pub system_default: &'static str,
    pub no_monitor: &'static str,

    pub mic_passthrough: &'static str,
    pub effect_volume: &'static str,
    pub repeat_behavior: &'static str,
    pub repeat_restart: &'static str,
    pub repeat_overlap: &'static str,
    pub repeat_toggle: &'static str,
    pub autostart: &'static str,
    pub retry: &'static str,
    pub audio_engine_error: &'static str,

    // 语言选择
    pub language: &'static str,

    // 锁定
    pub lock: &'static str,
    pub unlock: &'static str,
    pub lock_tooltip: &'static str,
    pub unlock_tooltip: &'static str,

    // 音效区
    pub profile: &'static str,
    pub open_folder: &'static str,
    pub remove_tooltip: &'static str,
    pub new_profile_placeholder: &'static str,
    pub create: &'static str,
    pub select_folder_tooltip: &'static str,

    pub stop_all: &'static str,
    pub stop_all_not_set: &'static str,
    pub set_hotkey: &'static str,
    pub clear: &'static str,

    pub empty_hint: &'static str,
    pub set_hotkey_short: &'static str,
    pub capturing_cancel: &'static str,
    pub none_label: &'static str,
    pub select_folder_dialog_title: &'static str,

    // 默认 profile 名
    pub default_profile: &'static str,

    // 应用音频路由
    pub app_audio_routing: &'static str,
    pub app_routing_hint: &'static str,
    pub open_app_volume: &'static str,

    // 系统音频捕获（WASAPI Loopback）
    pub loopback_title: &'static str,
    pub loopback_enable: &'static str,
    pub loopback_volume: &'static str,
    pub loopback_hint: &'static str,
    pub loopback_monitor_conflict: &'static str,

    // 单个音效音量

    // 新界面：导航 / 分组 / 状态栏
    pub tab_sounds: &'static str,
    pub tab_settings: &'static str,
    pub search_placeholder: &'static str,
    pub no_match: &'static str,
    pub new_profile_tooltip: &'static str,
    pub add_folder: &'static str,
    pub confirm: &'static str,
    pub reset: &'static str,

    // 外观
    pub section_appearance: &'static str,
    pub theme_mode: &'static str,
    pub theme_system: &'static str,
    pub theme_dark: &'static str,
    pub theme_light: &'static str,
    pub bg_color: &'static str,
    pub bg_image: &'static str,
    pub bg_image_pick: &'static str,
    pub bg_image_none: &'static str,
    pub bg_opacity: &'static str,
    pub bg_image_dialog_title: &'static str,
    pub appearance_hint: &'static str,
    pub window_opacity: &'static str,
    pub window_opacity_hint: &'static str,

    // 音量档位
    pub volume_presets: &'static str,
    pub volume_presets_hint: &'static str,
    pub volume_preset_menu: &'static str,
    pub volume_slider_tip: &'static str,
    pub add: &'static str,
    pub sort_tip: &'static str,
    pub sort_name_asc: &'static str,
    pub sort_name_desc: &'static str,
    pub sort_time_asc: &'static str,
    pub sort_time_desc: &'static str,
    pub sort_custom: &'static str,
    pub view_grid: &'static str,
    pub view_list: &'static str,
    pub hook_failed: &'static str,
    pub hook_blocked: &'static str,
    pub hook_diag: &'static str,
    pub hook_diag_tip: &'static str,
    pub hook_dead_hint: &'static str,
    pub open_log: &'static str,
    pub open_log_tip: &'static str,
    pub capture_device: &'static str,
    pub capture_device_tip: &'static str,
    pub capture_default: &'static str,
    pub capture_is_output: &'static str,
    pub section_devices: &'static str,
    pub section_playback: &'static str,
    pub section_startup: &'static str,
    pub master_volume: &'static str,
    pub stop_all_btn: &'static str,
    pub playing_now: &'static str,
    pub sound_count: &'static str,
}

impl Texts {
    /// 排序档位的显示名。
    pub fn sort_label(&self, mode: crate::config::SortMode) -> &'static str {
        use crate::config::SortMode as S;
        match mode {
            S::NameAsc => self.sort_name_asc,
            S::NameDesc => self.sort_name_desc,
            S::TimeAsc => self.sort_time_asc,
            S::TimeDesc => self.sort_time_desc,
            S::Custom => self.sort_custom,
        }
    }
    pub fn new(lang: Lang) -> &'static Self {
        match lang {
            Lang::Zh => Self::zh(),
            Lang::En => Self::en(),
        }
    }

    fn zh() -> &'static Texts {
        static T: std::sync::OnceLock<Texts> = std::sync::OnceLock::new();
        T.get_or_init(|| Texts {
            title: "VoicePlayer",
            vbcable_detected: "✔ 已检测到 VB-CABLE",
            vbcable_not_detected: "⚠ 未检测到 VB-CABLE（游戏里听不到音效）",
            open_vbcable_url: "打开 VB-CABLE 下载页",
            reinstall_detect: "我装好了，重新检测",
            output_device: "输出设备",
            output_device_tip: "选 CABLE Input（VB-Audio Virtual Cable），游戏才听得到",
            microphone: "麦克风",
            monitor_device: "监听设备",
            monitor_device_tip: "可选。设成你的耳机，自己也能听到音效",
            system_default: "系统默认",
            no_monitor: "不监听",
            mic_passthrough: "转发麦克风（边说话边放音效）",
            effect_volume: "音效音量",
            repeat_behavior: "重复按同一键：",
            repeat_restart: "从头重播",
            repeat_overlap: "叠加再播",
            repeat_toggle: "一按播 / 再按停",
            autostart: "开机自启动",
            retry: "重试",
            audio_engine_error: "音频引擎错误：",
           language: "语言",
            lock: "已锁定",
            unlock: "已解锁",
            lock_tooltip: "点击解锁快捷键",
            unlock_tooltip: "点击锁定快捷键",
            profile: "配置",
            open_folder: "打开文件夹",
            remove_tooltip: "从列表移除这个外部文件夹（不删除文件）",
            new_profile_placeholder: "输入名字新建；也可以直接粘贴一个文件夹的完整路径",
            create: "新建",
            select_folder_tooltip: "把电脑上任意一个装音效的文件夹加成配置",
            stop_all: "停止所有音效：",
            stop_all_not_set: "未设置",
            set_hotkey: "设快捷键",
            clear: "清除",
            empty_hint: "这个配置还没有音频。点「打开文件夹」把 mp3 / wav 拖进去，会自动出现在这里。",
            set_hotkey_short: "设快捷键",
            capturing_cancel: "按下快捷键…（Esc 取消）",
            none_label: "（无）",
            select_folder_dialog_title: "选择音效文件夹",
            default_profile: "默认",
            app_audio_routing: "应用音频路由",
            app_routing_hint: "把特定应用（网易云、浏览器等）的音频送进虚拟麦克风：在设置页面把它的输出设备改为 CABLE Input",
            open_app_volume: "打开应用音量设置",
            loopback_title: "系统音频捕获",
            loopback_enable: "捕获系统音频到麦克风",
            loopback_volume: "捕获音量",
            loopback_hint: "将正在播放的音频（音乐、视频等）混入虚拟麦克风（需安装 VB-CABLE）",
            loopback_monitor_conflict: "监听设备就是系统默认播放设备，系统声音你本来就直接听得到；再混一份进监听会形成回授啸叫，所以这里不重复送。想单独监听请把监听设备换成另一个设备。",
            tab_sounds: "音效",
            tab_settings: "设置",
            search_placeholder: "搜索音效…",
            no_match: "没有匹配的音效。",
            new_profile_tooltip: "新建一个配置文件夹",
            add_folder: "添加文件夹",
            confirm: "确定",
            reset: "复位",
            section_appearance: "外观",
            theme_mode: "主题",
            theme_system: "跟随系统",
            theme_dark: "深色",
            theme_light: "浅色",
            bg_color: "背景色",
            bg_image: "背景图片",
            bg_image_pick: "选择图片…",
            bg_image_none: "未设置",
            bg_opacity: "图片不透明度",
            bg_image_dialog_title: "选择背景图片",
            appearance_hint: "背景色会同时决定卡片、输入框、描边的明暗；设了背景图后面板会自动变半透明。",
            window_opacity: "窗口不透明度",
            window_opacity_hint: "调低后整个窗口半透明，能看见后面的桌面 / 游戏。文字和控件始终不透明，不影响阅读；标题栏由系统绘制，不受影响。",
            volume_presets: "音量档位",
            volume_presets_hint: "右键任意音量滑块可以从这些档位里一键选择。拖动数字可改，✖ 删除，＋ 新增。",
            volume_preset_menu: "快速设置音量",
            volume_slider_tip: "右键选档位或复位",
            add: "＋ 新增",
            sort_tip: "音效列表的排序方式。手动拖动过卡片后会自动切换到「自定义」。",
            sort_name_asc: "名称 A→Z",
            sort_name_desc: "名称 Z→A",
            sort_time_asc: "时间 旧→新",
            sort_time_desc: "时间 新→旧",
            sort_custom: "自定义",
            view_grid: "网格",
            view_list: "列表",
            hook_failed: "⚠ 全局热键未生效（只有窗口在前台时才能触发）",
            hook_blocked: "⚠ 全局热键被系统挡住了，试试以管理员身份运行",
            hook_diag: "热键诊断",
            hook_diag_tip: "钩子装上了但「收到按键」一直是 0，说明系统把钩子挡掉了：最常见的原因是前台程序（游戏 / 反作弊）以管理员权限运行，而本程序不是 —— 右键本程序选「以管理员身份运行」即可。「自愈重装」是钩子被系统摘掉后自动重装的次数。",
            hook_dead_hint: "以管理员身份运行可解决多数「后台不触发」的情况",
            open_log: "打开日志目录",
            open_log_tip: "日志文件 voiceplayer.log 就在这个目录里，出问题时把它发给我。",
            capture_device: "捕获设备",
            capture_device_tip: "抓哪个播放设备上的声音。默认只抓「系统默认播放设备」——\n如果你在 Windows 音量合成器里把某个程序（比如网易云）单独指到了 CABLE Input，\n它的声音就不在默认设备上了，这里要显式选 CABLE Input 才捕得到。",
            capture_default: "系统默认播放设备",
            capture_is_output: "捕获设备和输出设备是同一个，声音已经在这条链路上了：不再重复送进麦克风，只送监听。",
            section_devices: "设备",
            section_playback: "播放",
            section_startup: "启动",
            master_volume: "总音量",
            stop_all_btn: "⏹ 停止全部",
            playing_now: "正在播放",
            sound_count: "个音效",
        })
    }

    fn en() -> &'static Texts {
        static T: std::sync::OnceLock<Texts> = std::sync::OnceLock::new();
        T.get_or_init(|| Texts {
            title: "VoicePlayer",
            vbcable_detected: "✔ VB-CABLE detected",
            vbcable_not_detected: "⚠ VB-CABLE not detected (audio won't reach the game)",
            open_vbcable_url: "Open VB-CABLE download page",
            reinstall_detect: "I installed it, re-detect",
            output_device: "Output device",
            output_device_tip: "Pick CABLE Input (VB-Audio Virtual Cable) so the game can hear it",
            microphone: "Microphone",
            monitor_device: "Monitor device",
            monitor_device_tip: "Optional. Set it to your headphones to hear the effects yourself",
            system_default: "System default",
            no_monitor: "No monitor",
            mic_passthrough: "Passthrough microphone (talk while playing effects)",
            effect_volume: "Effect volume",
            repeat_behavior: "Repeat same key:",
            repeat_restart: "Restart",
            repeat_overlap: "Overlap",
            repeat_toggle: "Toggle (play / stop)",
            autostart: "Launch on startup",
            retry: "Retry",
            audio_engine_error: "Audio engine error: ",
           language: "Language",
            lock: "Locked",
            unlock: "Unlocked",
            lock_tooltip: "Click to unlock hotkeys",
            unlock_tooltip: "Click to lock hotkeys",
            profile: "Profile",
            open_folder: "Open folder",
            remove_tooltip: "Remove this external folder from the list (files are not deleted)",
            new_profile_placeholder: "Enter a name, or paste a full folder path",
            create: "Create",
            select_folder_tooltip: "Add any sound folder on your computer as a profile",
            stop_all: "Stop all sounds: ",
            stop_all_not_set: "Not set",
            set_hotkey: "Set hotkey",
            clear: "Clear",
            empty_hint: "No audio in this profile. Click \"Open folder\" and drop mp3 / wav files in.",
            set_hotkey_short: "Set hotkey",
            capturing_cancel: "Press a hotkey… (Esc to cancel)",
            none_label: "(none)",
            select_folder_dialog_title: "Select sound folder",
            default_profile: "Default",
            app_audio_routing: "App audio routing",
            app_routing_hint: "To send a specific app's audio (browser, music player, etc.) into the virtual mic, set its output device to CABLE Input in the settings page",
            open_app_volume: "Open app volume settings",
            loopback_title: "System Audio Capture",
            loopback_enable: "Capture system audio to microphone",
            loopback_volume: "Capture Volume",
            loopback_hint: "Mix currently playing audio (music, video, etc.) into the virtual microphone (requires VB-CABLE)",
            loopback_monitor_conflict: "The monitor device is your system's default playback device, so you already hear system audio directly. Mixing it back in would cause a feedback howl, so it is skipped. Pick a different monitor device to monitor it separately.",
            tab_sounds: "Sounds",
            tab_settings: "Settings",
            search_placeholder: "Search sounds…",
            no_match: "No sounds match your search.",
            new_profile_tooltip: "Create a new profile folder",
            add_folder: "Add folder",
            confirm: "OK",
            reset: "Reset",
            section_appearance: "Appearance",
            theme_mode: "Theme",
            theme_system: "Follow system",
            theme_dark: "Dark",
            theme_light: "Light",
            bg_color: "Background color",
            bg_image: "Background image",
            bg_image_pick: "Pick image…",
            bg_image_none: "Not set",
            bg_opacity: "Image opacity",
            bg_image_dialog_title: "Select background image",
            appearance_hint: "The background color also drives card, input and border shades. With a background image, panels turn semi-transparent automatically.",
            window_opacity: "Window opacity",
            window_opacity_hint: "Lower it to see the desktop / game behind the window. Text and controls stay fully opaque; the title bar is drawn by the OS and is unaffected.",
            volume_presets: "Volume presets",
            volume_presets_hint: "Right-click any volume slider to pick one of these. Drag a number to edit, ✖ removes, ＋ adds.",
            volume_preset_menu: "Set volume",
            volume_slider_tip: "Right-click for presets or reset",
            add: "＋ Add",
            sort_tip: "How the sound list is ordered. Dragging a card switches this to Custom automatically.",
            sort_name_asc: "Name A→Z",
            sort_name_desc: "Name Z→A",
            sort_time_asc: "Oldest first",
            sort_time_desc: "Newest first",
            sort_custom: "Custom",
            view_grid: "Grid",
            view_list: "List",
            hook_failed: "⚠ Global hotkeys are not active (they only work while this window is focused)",
            hook_blocked: "⚠ Windows is blocking the global hotkey hook — try running as administrator",
            hook_diag: "Hotkey diagnostics",
            hook_diag_tip: "If the hook is installed but the key-event count stays at 0, Windows is blocking it. The usual cause is that the foreground app (a game or its anti-cheat) runs elevated while this app does not — right-click the app and pick \"Run as administrator\". \"Reinstalls\" counts how often the hook was silently removed and put back automatically.",
            hook_dead_hint: "Running as administrator fixes most \"nothing happens in the background\" cases",
            open_log: "Open log folder",
            open_log_tip: "voiceplayer.log lives in this folder — send it along when reporting a problem.",
            capture_device: "Capture device",
            capture_device_tip: "Which playback device to capture. By default only the system default device is captured —\nif you routed an app (e.g. a music player) to CABLE Input in the Windows volume mixer,\nits audio is no longer on the default device and you must pick that device here.",
            capture_default: "System default playback device",
            capture_is_output: "The capture device is the same as the output device, so the audio is already on this path: it is only sent to the monitor, not re-injected into the microphone.",
            section_devices: "Devices",
            section_playback: "Playback",
            section_startup: "Startup",
            master_volume: "Master",
            stop_all_btn: "⏹ Stop all",
            playing_now: "Playing",
            sound_count: "sounds",
        })
    }
}
