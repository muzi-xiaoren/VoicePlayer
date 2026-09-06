//! egui 界面 + 状态管理，把配置、音频线程、热键、profile、i18n 串起来。
use crate::audio::{self, AudioCmd, AudioCtl};
use crate::config::{AppConfig, RepeatMode, SortMode, ThemeMode, ViewMode};
use crate::hotkeys::{self, HkAction, Hotkeys};
use crate::i18n;
use crate::platform;
use crate::profile::{self, Profile, Sound};
use crate::theme;
use std::path::PathBuf;
use std::time::{Duration, Instant};
/// 网格模式下瓦片的最小宽度，用它算一行能放几列。
const TILE_W_MIN: f32 = 240.0;
/// 卡片框自身吃掉的垂直空间 = `theme::card()` 的上下内边距（10 + 10）。
/// 描边是画在边界上的、不占布局空间，横竖都一样溢出半像素，所以不算进来 ——
/// 这样瓦片高就等于卡片实际占的高，纵横间距才严格相等。
///
/// 瓦片高度 = 内容高 + 它，**不能写死**：写死过一次（108），
/// 而实际内容比估的高，卡片比格子还高，纵向间距被吃掉、字体一变就顶到下一行。
const CARD_V: f32 = 20.0;
/// 音量数值框的固定宽度。滑块轨道 = 可用宽 - 它 - 一个间距，正好填满、右对齐。
const VALUE_W: f32 = 52.0;
/// 长按多久（秒）开始拖动。太短会和点击/拖滑块打架，太长手感发黏。
const LONG_PRESS: f64 = 0.32;
/// 长按判定期间允许的手指抖动（像素）。超过就当成是在操作控件，不进入拖动。
const LONG_PRESS_SLOP: f32 = 6.0;
/// 卡片归位动画时长（秒）。
const REFLOW_TIME: f32 = 0.13;

/// 拖动排序的实时状态。
///
/// 放在 `App` 上而不是 egui 的 DragAndDrop 里，是因为要自己算「现在会插到第几格」，
/// 才能让其他卡片**实时让位**（egui 自带的拖放只有一个高亮框，没有让位动画）。
#[derive(Clone, Copy)]
struct DragState {
    /// 被按住那一项在 `profile.sounds` 里的下标。
    from: usize,
    /// 指针相对卡片左上角的偏移。拖起来时卡片不会跳到指针底下。
    grab: egui::Vec2,
    /// 是否已经越过长按阈值、真正进入拖动。
    active: bool,
    /// 按下的时刻（`ctx.input().time`）和位置，用来判长按。
    press_at: f64,
    press_pos: egui::Pos2,
}

/// 界面分页。设置项从主界面挪走，音效列表才是主角。
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Sounds,
    Settings,
}

/// 正在为「谁」捕获快捷键。
#[derive(Clone, Copy, PartialEq)]
enum CaptureTarget {
    Sound(usize),
    Stop,
}
/// UI 一帧里收集下来、帧末统一处理的动作。
#[derive(Default)]
struct Pending {
    rebuild_engine: bool,
    switch_profile: Option<String>,
    new_profile: Option<String>,
    pick_folder: bool,
    remove_external: Option<String>,
    play: Vec<usize>,
    capture: Option<CaptureTarget>,
    clear_hotkey: Vec<usize>,
    clear_stop: bool,
    set_volume: Vec<(usize, f32)>,
    open_folder: bool,
    reregister: bool,
    lock_toggled: bool,
    lang_changed: bool,
    /// 把第 .0 项移动到第 .1 项的位置。
    reorder: Option<(usize, usize)>,
    sort_mode: Option<SortMode>,
    view_mode: Option<ViewMode>,
    /// 这一帧有人按住了第 N 项的拖动手柄。
    grip_pressed: Option<usize>,
    pick_bg_image: bool,
    clear_bg_image: bool,
    retheme: bool,
}
pub struct App {
    config: AppConfig,
    audio: AudioCtl,
    hotkeys: Option<Hotkeys>,
    profiles: Vec<String>,
    profile: Option<Profile>,
    out_devices: Vec<String>,
    in_devices: Vec<String>,
    vbcable: bool,
    capturing: Option<CaptureTarget>,
    new_profile_name: String,
    /// 「新建配置」输入框是否展开。
    show_new_profile: bool,
    /// 音效搜索关键字。
    search: String,
    page: Page,
    /// 已加载的背景图纹理，以及它对应的文件路径（路径变了才重新解码）。
    bg_tex: Option<egui::TextureHandle>,
    bg_tex_path: Option<PathBuf>,
    /// 上次上主题时的 (是否浅色, 自定义底色, 有无背景图)。变了才重新 apply。
    theme_sig: Option<(bool, Option<[u8; 3]>, bool)>,
    /// 拖动排序状态。渲染走 `&self`，所以用 RefCell 装。
    drag: std::cell::RefCell<Option<DragState>>,
    /// 上一帧量到的卡片真实高度 [网格, 列表]。0 = 还没量过，先用公式估。
    card_h: std::cell::Cell<[f32; 2]>,
   last_scan: Instant,
   /// 进程启动时刻。用来判断「开了这么久还一个按键都没收到」= 钩子被挡了。
   started: Instant,
   last_signature: Vec<String>,
   lang: i18n::Lang,
    last_window_save: Instant,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
       install_cjk_fonts(&cc.egui_ctx);
       let mut config = AppConfig::load();
        theme::apply(
            &cc.egui_ctx,
            matches!(config.theme_mode, ThemeMode::Light),
            config.bg_color,
            config.bg_image.is_some(),
        );
        let lang = i18n::resolve_lang(&config.language);
        let texts = i18n::Texts::new(lang);
        let out_devices = audio::output_devices();
        let in_devices = audio::input_devices();
        let vbcable = audio::vbcable_installed();
        if config.output_device.is_none() {
            config.output_device = audio::guess_cable_output();
        }
        let profiles = Self::all_profiles(&config, texts.default_profile);
        let active = config
            .active_profile
            .clone()
            .filter(|n| profiles.contains(n))
            .or_else(|| profiles.first().cloned());
        config.active_profile = active.clone();
        let profile = active
            .as_deref()
            .map(|n| {
                let mut p = Profile::load(n, &Self::dir_for_config(&config, n));
                p.apply_sort(config.sort_mode);
                p
            });
        let last_signature = profile
            .as_ref()
            .map(|p| Profile::file_signature(&p.dir))
            .unwrap_or_default();
        let audio = audio::spawn(config.clone());
        // 全局热键：用低级键盘钩子，回调直接驱动音频线程。
        let hotkeys = {
            let audio2 = audio.clone();
            Hotkeys::new(move |action| match action {
                HkAction::Play { path, volume } => audio2.send(AudioCmd::Play { path, volume }),
                HkAction::StopAll => audio2.send(AudioCmd::StopAll),
            })
            .ok()
        };
        if let Some(hk) = &hotkeys {
            hk.set_locked(config.locked);
        }
        let mut app = Self {
            config,
            audio,
            hotkeys,
            profiles,
            profile,
            out_devices,
            in_devices,
            vbcable,
            capturing: None,
            new_profile_name: String::new(),
            show_new_profile: false,
            search: String::new(),
            page: Page::Sounds,
            bg_tex: None,
            bg_tex_path: None,
            theme_sig: None,
            drag: std::cell::RefCell::new(None),
            card_h: std::cell::Cell::new([0.0, 0.0]),
           last_scan: Instant::now(),
           started: Instant::now(),
           last_signature,
           lang,
            last_window_save: Instant::now(),
       };
        app.config.save();
        app.reregister_hotkeys();
        app
    }
    fn texts(&self) -> &'static i18n::Texts {
        i18n::Texts::new(self.lang)
    }
    fn all_profiles(cfg: &AppConfig, default_name: &str) -> Vec<String> {
        let mut names = profile::list_profiles(default_name);
        for n in cfg.external_profiles.keys() {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
        names.sort();
        names
    }
    fn dir_for_config(cfg: &AppConfig, name: &str) -> PathBuf {
        cfg.external_profiles
            .get(name)
            .cloned()
            .unwrap_or_else(|| crate::config::profiles_dir().join(name))
    }
    fn dir_for(&self, name: &str) -> PathBuf {
        Self::dir_for_config(&self.config, name)
    }
    fn rebuild_engine(&mut self) {
        self.audio.send(AudioCmd::Rebuild(self.config.clone()));
    }
    /// 整体重建热键绑定表。每次 profile 切换 / 快捷键变更 / 音量调整后调用。
    fn reregister_hotkeys(&mut self) {
        let Some(hk) = self.hotkeys.as_ref() else { return };
        let mut bindings: Vec<(String, HkAction)> = Vec::new();
        if let Some(p) = &self.profile {
            for s in &p.sounds {
                if let Some(combo) = &s.hotkey {
                    bindings.push((
                        combo.clone(),
                        HkAction::Play { path: s.path.clone(), volume: s.volume },
                    ));
                }
            }
        }
        if let Some(stop) = &self.config.stop_hotkey {
            bindings.push((stop.clone(), HkAction::StopAll));
        }
        hk.update_bindings(&bindings);
    }
    fn switch_profile(&mut self, name: &str) {
        self.config.active_profile = Some(name.to_string());
        let dir = self.dir_for(name);
        let mut p = Profile::load(name, &dir);
        p.apply_sort(self.config.sort_mode);
        self.last_signature = Profile::file_signature(&p.dir);
        self.profile = Some(p);
        self.config.save();
        self.reregister_hotkeys();
    }
    fn add_external_folder(&mut self, dir: PathBuf, _dialog_title: &str) {
        if let Some(name) = self
            .config
            .external_profiles
            .iter()
            .find(|(_, d)| **d == dir)
            .map(|(n, _)| n.clone())
        {
            self.switch_profile(&name);
            return;
        }
        let base = dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| dir.display().to_string());
        let mut name = base.clone();
        let mut i = 2;
        while self.profiles.contains(&name) {
            name = format!("{base} ({i})");
            i += 1;
        }
        self.config.external_profiles.insert(name.clone(), dir);
        self.config.save();
        self.profiles = Self::all_profiles(&self.config, self.texts().default_profile);
        self.switch_profile(&name);
    }
    /// 每帧对一次主题：模式 / 底色 / 背景图任一变化就重新上样式。
    /// 「跟随系统」也走这里，所以系统在运行中切深浅色能实时跟上。
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let light = match self.config.theme_mode {
            ThemeMode::Light => true,
            ThemeMode::Dark => false,
            ThemeMode::System => ctx
                .system_theme()
                .map(|t| t == egui::Theme::Light)
                .unwrap_or(false),
        };
        theme::set_window_opacity(self.config.window_opacity);
        let sig = (light, self.config.bg_color, self.config.bg_image.is_some());
        if self.theme_sig != Some(sig) {
            self.theme_sig = Some(sig);
            theme::apply(ctx, light, self.config.bg_color, self.config.bg_image.is_some());
        }
        if self.config.bg_image != self.bg_tex_path {
            self.bg_tex_path = self.config.bg_image.clone();
            self.bg_tex = self
                .config
                .bg_image
                .as_deref()
                .and_then(|p| load_bg_texture(ctx, p));
            if self.bg_tex.is_none() && self.bg_tex_path.is_some() {
                log::warn!("背景图加载失败：{:?}", self.bg_tex_path);
            }
        }
    }

    fn refresh_devices(&mut self) {
        self.out_devices = audio::output_devices();
        self.in_devices = audio::input_devices();
        self.vbcable = audio::vbcable_installed();
    }
    fn maybe_rescan(&mut self) {
        if self.last_scan.elapsed() < Duration::from_millis(1500) {
            return;
        }
        self.last_scan = Instant::now();
        let info = self.profile.as_ref().map(|p| (p.name.clone(), p.dir.clone()));
        if let Some((name, dir)) = info {
            let sig = Profile::file_signature(&dir);
            if sig != self.last_signature {
                self.last_signature = sig;
                let mut p = Profile::load(&name, &dir);
                p.apply_sort(self.config.sort_mode);
                self.profile = Some(p);
                self.reregister_hotkeys();
            }
        }
    }
   /// 轮询钩子线程的捕获结果。
    fn handle_capture(&mut self, ctx: &egui::Context) {
        if self.capturing.is_none() {
            return;
        }
        // 先试低级钩子通道（窗口无焦点时仍可工作）
        let mut result = self.hotkeys.as_ref().and_then(|hk| hk.poll_capture());
        // 回退到 egui 键盘事件（窗口有焦点时更可靠）
        if result.is_none() {
            result = self.poll_egui_capture(ctx);
        }
        let Some(capture_result) = result else { return };
        let target = self.capturing.take();
        if let Some(hk) = &self.hotkeys {
            hk.stop_capture();
        }
        if let (Some((mods, vk)), Some(target)) = (capture_result, target) {
            let combo = hotkeys::format_combo(mods, vk);
            match target {
                CaptureTarget::Sound(i) => {
                    if let Some(p) = self.profile.as_mut() {
                        if let Some(s) = p.sounds.get_mut(i) {
                            s.hotkey = Some(combo);
                        }
                        p.save_bindings();
                    }
                }
                CaptureTarget::Stop => {
                    self.config.stop_hotkey = Some(combo);
                    self.config.save();
                }
            }
            self.reregister_hotkeys();
        }
    }
    /// 当窗口有焦点时，用 egui 自己的键盘事件来捕获快捷键（比钩子通道更可靠）。
    fn poll_egui_capture(&self, ctx: &egui::Context) -> Option<Option<(u8, u32)>> {
        let mut captured = None;
        ctx.input(|input| {
            for event in &input.events {
                if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event {
                    if *key == egui::Key::Escape {
                        captured = Some(None);
                        return;
                    }
                   if let Some(vk) = egui_key_to_vk(*key) {
                        let vk = disambiguate_numpad(vk);
                        let mut mods = 0u8;
                        if modifiers.ctrl { mods |= hotkeys::MOD_CTRL; }
                        if modifiers.alt { mods |= hotkeys::MOD_ALT; }
                       if modifiers.shift { mods |= hotkeys::MOD_SHIFT; }
                       captured = Some(Some((mods, vk)));
                       return;
                   }
                }
            }
       });
       captured
   }
    /// 窗口有焦点时用 egui 键盘事件触发快捷键。
    ///
    /// 这条路**永远开着**，它是钩子的保险丝：钩子被系统悄悄摘掉时，
    /// 至少前台还能用。重复触发由 `Hotkeys` 内部按来源去重挡掉
    /// （钩子刚为同一个键触发过，这里就跳过），不需要在这里关掉整条路 ——
    /// 上一版就是关掉了它，钩子一死就变成前后台全哑。
    fn poll_egui_trigger(&self, ctx: &egui::Context) {
        if self.capturing.is_some() {
            return;
        }
        let Some(hk) = self.hotkeys.as_ref() else { return };
        ctx.input(|input| {
            for event in &input.events {
                if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event {
                    if let Some(vk) = egui_key_to_vk(*key) {
                        let vk = disambiguate_numpad(vk);
                        let mut mods = 0u8;
                        if modifiers.ctrl { mods |= hotkeys::MOD_CTRL; }
                        if modifiers.alt { mods |= hotkeys::MOD_ALT; }
                        if modifiers.shift { mods |= hotkeys::MOD_SHIFT; }
                        hk.trigger_from_vk(mods, vk);
                    }
                }
            }
        });
    }
    fn apply(&mut self, pending: Pending) {
       let texts = self.texts();
        let mut need_reregister = pending.reregister;
        let mut need_retheme = pending.retheme;
        for (i, v) in pending.set_volume {
            if let Some(p) = self.profile.as_mut() {
                if let Some(s) = p.sounds.get_mut(i) {
                    s.volume = v;
                    // 即时更新正在播放的音轨音量（无需重新触发）
                    self.audio.send(AudioCmd::SetSoundVolume {
                        path: s.path.clone(),
                        volume: v,
                    });
                }
                p.save_bindings();
            }
            need_reregister = true;
        }
        for i in pending.clear_hotkey {
            if let Some(p) = self.profile.as_mut() {
                if let Some(s) = p.sounds.get_mut(i) {
                    s.hotkey = None;
                }
                p.save_bindings();
            }
            need_reregister = true;
        }
        if pending.clear_stop {
            self.config.stop_hotkey = None;
            self.config.save();
            need_reregister = true;
        }
        for i in pending.play {
            if let Some(p) = &self.profile {
                if let Some(s) = p.sounds.get(i) {
                    self.audio.send(AudioCmd::Play {
                        path: s.path.clone(),
                        volume: s.volume,
                    });
                }
            }
        }
        if pending.open_folder {
            if let Some(p) = &self.profile {
                platform::open_folder(&p.dir);
            }
        }
        if let Some(t) = pending.capture {
            self.capturing = Some(t);
            if let Some(hk) = self.hotkeys.as_mut() {
                hk.start_capture();
            }
        }
        if pending.lock_toggled {
            if let Some(hk) = &self.hotkeys {
                hk.set_locked(self.config.locked);
            }
            self.config.save();
        }
        if pending.lang_changed {
            self.config.language = Some(self.lang.code().to_string());
            self.config.save();
        }
        if let Some(input) = pending.new_profile {
            let input = input.trim().to_string();
            if !input.is_empty() {
                let p = PathBuf::from(&input);
                if p.is_absolute() {
                    if p.is_dir() || std::fs::create_dir_all(&p).is_ok() {
                        self.add_external_folder(p, texts.select_folder_dialog_title);
                        self.new_profile_name.clear();
                        self.show_new_profile = false;
                    }
                } else if !input.contains('/') && !input.contains('\\') && profile::create_profile(&input).is_ok() {
                    self.profiles = Self::all_profiles(&self.config, texts.default_profile);
                    self.switch_profile(&input);
                    self.new_profile_name.clear();
                    self.show_new_profile = false;
                }
            }
        }
        if let Some((from, to)) = pending.reorder {
            if let Some(p) = self.profile.as_mut() {
                p.move_sound(from, to);
                p.save_bindings();
            }
            // 手动拖过一次，排序方式就变成「自定义」—— 否则下次重排会把手动顺序冲掉。
            if self.config.sort_mode != SortMode::Custom {
                self.config.sort_mode = SortMode::Custom;
                self.config.save();
            }
            need_reregister = true;
        }
        if let Some(m) = pending.sort_mode {
            self.config.sort_mode = m;
            self.config.save();
            if let Some(p) = self.profile.as_mut() {
                p.apply_sort(m);
                // 顺手把新顺序落盘：之后切到「自定义」还是这个顺序，不会跳回去。
                p.save_bindings();
            }
            need_reregister = true;
        }
        if let Some(v) = pending.view_mode {
            self.config.view_mode = v;
            self.config.save();
        }
        if pending.pick_bg_image {
            if let Some(f) = rfd::FileDialog::new()
                .set_title(texts.bg_image_dialog_title)
                .add_filter("image", &["png", "jpg", "jpeg"])
                .pick_file()
            {
                self.config.bg_image = Some(f);
                self.config.save();
                need_retheme = true;
            }
        }
        if pending.clear_bg_image {
            self.config.bg_image = None;
            self.bg_tex = None;
            self.bg_tex_path = None;
            self.config.save();
            need_retheme = true;
        }
        if pending.pick_folder {
            if let Some(dir) = rfd::FileDialog::new()
                .set_title(texts.select_folder_dialog_title)
                .pick_folder()
            {
                self.add_external_folder(dir, texts.select_folder_dialog_title);
            }
        }
        if let Some(name) = pending.remove_external {
            self.config.external_profiles.remove(&name);
            self.profiles = Self::all_profiles(&self.config, texts.default_profile);
            if self.config.active_profile.as_deref() == Some(name.as_str()) {
                if let Some(first) = self.profiles.first().cloned() {
                    self.switch_profile(&first);
                } else {
                    self.config.active_profile = None;
                    self.profile = None;
                    self.config.save();
                    need_reregister = true;
                }
            } else {
                self.config.save();
            }
        }
        if let Some(name) = pending.switch_profile {
            self.switch_profile(&name);
        }
        if pending.rebuild_engine {
            self.rebuild_engine();
        }
        if need_reregister {
            self.reregister_hotkeys();
        }
        let _ = need_retheme; // 主题变化由 update() 里的 sync_theme 统一检测
    }
    // ─────────────────────────── 顶栏 ───────────────────────────
    /// 应用名 + 页面切换 + 锁定 + 语言。
    fn ui_topbar(&mut self, ui: &mut egui::Ui, pending: &mut Pending) {
        let texts = self.texts();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(texts.title).size(16.0).strong().color(theme::p().accent));
            ui.add_space(10.0);

            // 分段控件式的页面切换
            if ui.selectable_label(self.page == Page::Sounds, texts.tab_sounds).clicked() {
                self.page = Page::Sounds;
            }
            if ui.selectable_label(self.page == Page::Settings, texts.tab_settings).clicked() {
                self.page = Page::Settings;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut lang_sel = self.lang;
                egui::ComboBox::from_id_salt("lang")
                    .width(96.0)
                    .selected_text(self.lang.display())
                    .show_ui(ui, |ui| {
                        ui.label(egui::RichText::new(texts.language).size(11.5).color(theme::p().dim));
                        for l in i18n::Lang::all() {
                            if ui.selectable_value(&mut lang_sel, l, l.display()).changed() && l != self.lang {
                                self.lang = l;
                                pending.lang_changed = true;
                            }
                        }
                    });

                // 锁定：锁上时用警告色，一眼能看出快捷键当前不响应
                let (label, tip, color) = if self.config.locked {
                    (texts.lock, texts.lock_tooltip, theme::p().warn)
                } else {
                    (texts.unlock, texts.unlock_tooltip, theme::p().dim)
                };
                // 用一个小圆点代替锁 emoji（字体里不一定有）
                let dot = if self.config.locked { theme::p().warn } else { theme::p().ok };
                let prev = self.config.locked;
                if ui
                    .add(egui::Button::new(egui::RichText::new(label).color(color)).frame(false))
                    .on_hover_text(tip)
                    .clicked()
                {
                    self.config.locked = !self.config.locked;
                }
                if self.config.locked != prev {
                    pending.lock_toggled = true;
                }
                let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, dot);
            });
        });
    }

    // ─────────────────────────── 底部状态栏 ───────────────────────────
    /// 常驻信息：链路是否通、输出设备、总音量、停止全部。
    fn ui_statusbar(&mut self, ui: &mut egui::Ui, pending: &mut Pending) {
        let texts = self.texts();
        ui.horizontal(|ui| {
            // 钩子没装上 = 全局热键彻底没有；装上了但开了半天一个键都没收到
            // = 被系统挡住了（多半是前台程序以管理员权限运行）。两种都要让用户看见，
            // 否则表现都是「按了没反应」，而且只有前台还能用，很容易误判成没坏。
            match self.hotkeys.as_ref() {
                Some(h) if !h.hook_installed() => {
                    theme::status_dot(ui, theme::p().danger, texts.hook_failed);
                    ui.separator();
                }
                Some(h)
                    if h.event_count() == 0
                        && self.started.elapsed() >= Duration::from_secs(30) =>
                {
                    theme::status_dot(ui, theme::p().warn, texts.hook_blocked);
                    ui.separator();
                }
                _ => {}
            }
            if self.vbcable {
                theme::status_dot(ui, theme::p().ok, texts.vbcable_detected);
            } else {
                theme::status_dot(ui, theme::p().warn, texts.vbcable_not_detected);
                if ui.small_button(texts.open_vbcable_url).clicked() {
                    platform::open_url(platform::VBCABLE_URL);
                }
                if ui.small_button(texts.reinstall_detect).clicked() {
                    self.refresh_devices();
                    pending.rebuild_engine = true;
                }
            }

            let n_playing = self.audio.playing_snapshot().len();
            if n_playing > 0 {
                ui.add_space(4.0);
                theme::badge(
                    ui,
                    &format!("▶ {} {}", texts.playing_now, n_playing),
                    theme::p().accent,
                    theme::p().accent_soft,
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(texts.stop_all_btn).clicked() {
                    self.audio.send(AudioCmd::StopAll);
                }
                if let Some(h) = &self.config.stop_hotkey {
                    theme::badge(ui, &hotkeys::pretty_combo(h), theme::p().dim, theme::p().sunken);
                }
                ui.add_space(6.0);
                ui.spacing_mut().slider_width = 110.0;
                let resp = ui.add(
                    egui::Slider::new(&mut self.config.effect_volume, 0.0..=1.5)
                        .show_value(true)
                        .fixed_decimals(2),
                );
                let picked = volume_preset_menu(
                    &resp,
                    &self.config.volume_presets.clone(),
                    texts.volume_preset_menu,
                    1.5,
                    texts.reset,
                );
                if let Some(v) = picked {
                    self.config.effect_volume = v;
                }
                if resp.changed() || picked.is_some() {
                    self.audio.send(AudioCmd::SetEffectVolume(self.config.effect_volume));
                    self.config.save();
                }
                ui.label(egui::RichText::new(texts.master_volume).size(12.0).color(theme::p().dim));
                // 右侧这组和左边的状态文字之间留出间距，否则会挤在一起
                ui.add_space(16.0);
            });
        });
    }

    // ─────────────────────────── 音效页 ───────────────────────────
    fn ui_sounds(
        &mut self,
        ui: &mut egui::Ui,
        profiles: &[String],
        sounds: &[Sound],
        playing: &std::collections::HashSet<PathBuf>,
        presets: &[f32],
        pending: &mut Pending,
    ) {
        let texts = self.texts();

        // ── 工具条：配置选择 + 文件夹操作 ──
        ui.horizontal(|ui| {
            let cur = self
                .config
                .active_profile
                .clone()
                .unwrap_or_else(|| texts.none_label.to_string());
            let is_external = self
                .config
                .active_profile
                .as_ref()
                .map(|n| self.config.external_profiles.contains_key(n))
                .unwrap_or(false);
            let shown = if is_external { format!("📁 {cur}") } else { cur };
            egui::ComboBox::from_id_salt("profile")
                .width(180.0)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for name in profiles {
                        let selected = self.config.active_profile.as_deref() == Some(name);
                        let label = if self.config.external_profiles.contains_key(name) {
                            format!("📁 {name}")
                        } else {
                            name.clone()
                        };
                        if ui.selectable_label(selected, label).clicked() && !selected {
                            pending.switch_profile = Some(name.clone());
                        }
                    }
                })
                .response
                .on_hover_text(texts.profile);
            if ui.button(texts.open_folder).clicked() {
                pending.open_folder = true;
            }
            if ui.button(texts.add_folder).on_hover_text(texts.select_folder_tooltip).clicked() {
                pending.pick_folder = true;
            }
            if ui
                .selectable_label(self.show_new_profile, texts.create)
                .on_hover_text(texts.new_profile_tooltip)
                .clicked()
            {
                self.show_new_profile = !self.show_new_profile;
            }
            if is_external && ui.button("✖").on_hover_text(texts.remove_tooltip).clicked() {
                pending.remove_external = self.config.active_profile.clone();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 排序方式：选到「自定义」以外的档位会立刻重排；拖过卡片会自动切回自定义。
                let mut mode = self.config.sort_mode;
                egui::ComboBox::from_id_salt("sortmode")
                    .width(112.0)
                    .selected_text(texts.sort_label(mode))
                    .show_ui(ui, |ui| {
                        for m in [
                            SortMode::NameAsc,
                            SortMode::NameDesc,
                            SortMode::TimeAsc,
                            SortMode::TimeDesc,
                            SortMode::Custom,
                        ] {
                            ui.selectable_value(&mut mode, m, texts.sort_label(m));
                        }
                    })
                    .response
                    .on_hover_text(texts.sort_tip);
                if mode != self.config.sort_mode {
                    pending.sort_mode = Some(mode);
                }
                // 网格 / 列表
                let mut view = self.config.view_mode;
                ui.selectable_value(&mut view, ViewMode::List, texts.view_list);
                ui.selectable_value(&mut view, ViewMode::Grid, texts.view_grid);
                if view != self.config.view_mode {
                    pending.view_mode = Some(view);
                }
                ui.label(
                    egui::RichText::new(format!("{} {}", sounds.len(), texts.sound_count))
                        .size(12.0)
                        .color(theme::p().dim),
                );
            });
        });

        // ── 新建配置：默认收起，展开后单独占一行 ──
        if self.show_new_profile {
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.new_profile_name)
                        .desired_width(ui.available_width() - 80.0)
                        .hint_text(texts.new_profile_placeholder),
                );
                let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(texts.confirm).clicked() || submit {
                    pending.new_profile = Some(self.new_profile_name.clone());
                }
            });
        }
        // ── 搜索：独占一行 ──
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .desired_width(ui.available_width())
                .hint_text(format!("🔍 {}", texts.search_placeholder)),
        );

        ui.add_space(4.0);

        if sounds.is_empty() {
            self.ui_empty_hint(ui, texts.empty_hint);
            return;
        }

        // 搜索过滤，同时保留原始下标（pending 里用的是原始下标）
        let needle = self.search.trim().to_lowercase();
        let visible: Vec<(usize, &Sound)> = sounds
            .iter()
            .enumerate()
            .filter(|(_, s)| needle.is_empty() || s.name.to_lowercase().contains(&needle))
            .collect();
        if visible.is_empty() {
            self.ui_empty_hint(ui, texts.no_match);
            return;
        }

        // ── 布局：自己算位置 + 动画归位 ──
        //
        // 不用 egui 的流式布局，是因为要做「拖动时其他卡片实时让位」：
        // 每一项的目标位置由它在**当前显示顺序**里的槽位算出来，
        // 实际画的位置用 animate_value_with_time 从旧位置插值过去，
        // 于是拖动经过谁，谁就滑开，松手后再滑回整齐的格子里。
        //
        // 位置一律记「相对内容区左上角」的偏移，不是屏幕坐标 ——
        // 否则一滚动整页都会跟着做归位动画。
        let list_mode = self.config.view_mode == ViewMode::List;
        let gap = ui.spacing().item_spacing.x;
        let n = visible.len();

        // 瓦片高度按实际内容算：每行固定 row_h，行间 item_spacing.y，
        // 再加卡片框自身的 CARD_V。列表一行、网格三行。
        let row_h = row_height(ui);
        let gap_y = ui.spacing().item_spacing.y;
        let card_rows = if list_mode { 1.0 } else { 3.0 };
        let content_h = row_h * card_rows + gap_y * (card_rows - 1.0);
        // 有上一帧量到的真实高度就用它，第一帧才用公式估的值兜底。
        let measured = self.card_h.get()[usize::from(list_mode)];
        let tile_h = if measured > 0.0 { measured } else { content_h + CARD_V };

        // 滚动条的宽度**只在真的会出现滚动条时**才扣。
        // 以前无条件扣掉，没滚动条时右边就白空一条，卡片右缘比上面的搜索框短一截。
        // egui 默认是悬浮滚动条（不占宽），所以不扣才是对的。
        let sc = ui.spacing().scroll;
        let bar = if sc.floating {
            sc.floating_allocated_width.max(sc.bar_width + sc.bar_inner_margin)
        } else {
            sc.bar_width + sc.bar_inner_margin + sc.bar_outer_margin
        };
        let full = ui.available_width().max(160.0);
        let avail_h = ui.available_height();
        // 布局只依赖宽度：先按满宽算一次，超高才让出滚动条的位置再算一次。
        // 宽度变窄只会让内容更高，不会反过来又不需要滚动条，所以不会来回抖。
        let layout_for = |w: f32| -> (usize, f32, f32) {
            let (cols, tile_w) = if list_mode {
                (1usize, w)
            } else {
                let c = (((w + gap) / (TILE_W_MIN + gap)).floor()).max(1.0);
                (c as usize, ((w - gap * (c - 1.0)) / c).floor().max(160.0))
            };
            let rows = n.div_ceil(cols);
            (cols, tile_w, rows as f32 * (tile_h + gap) - gap)
        };
        let avail = if layout_for(full).2 > avail_h { (full - bar).max(160.0) } else { full };
        let (cols, tile_w, content_total_h) = layout_for(avail);
        let cell = egui::vec2(tile_w + gap, tile_h + gap);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let origin = ui.cursor().left_top();
            // 先把整块地占掉，滚动条才知道内容有多高。
            ui.allocate_exact_size(egui::vec2(avail, content_total_h), egui::Sense::hover());

            let pointer = ui.ctx().pointer_interact_pos();
            let mut drag = *self.drag.borrow();
            // 被拖的那一项在当前可见列表里的槽位。搜索把它过滤掉了就当没在拖。
            let from_slot = drag
                .filter(|d| d.active)
                .and_then(|d| visible.iter().position(|(i, _)| *i == d.from));
            // 指针落在第几格 = 松手后会插到哪儿。
            let drop_slot = match (from_slot, pointer) {
                (Some(_), Some(pos)) => {
                    let rel = pos - origin;
                    let col = ((rel.x / cell.x).floor() as isize).clamp(0, cols as isize - 1) as usize;
                    let row = (rel.y / cell.y).floor().max(0.0) as usize;
                    Some((row * cols + col).min(n.saturating_sub(1)))
                }
                _ => None,
            };

            // 显示顺序：把被拖的那一项抽出来，插到目标槽位。其余项顺次让位。
            let mut order: Vec<usize> = (0..n).collect();
            if let (Some(f), Some(t)) = (from_slot, drop_slot) {
                let it = order.remove(f);
                order.insert(t.min(order.len()), it);
            }

            // 目标空位：拖动时画一个虚位，明确告诉用户会落在哪。
            if let Some(t) = drop_slot {
                let r = egui::Rect::from_min_size(
                    origin + egui::vec2((t % cols) as f32 * cell.x, (t / cols) as f32 * cell.y),
                    egui::vec2(tile_w, tile_h),
                );
                ui.painter().rect_filled(
                    r,
                    egui::Rounding::same(theme::R_CARD),
                    theme::p().accent_soft,
                );
            }

            for (slot, &vi) in order.iter().enumerate() {
                let (i, s) = visible[vi];
                let dragged = drag.map(|d| d.active && d.from == i).unwrap_or(false);
                let target = egui::vec2((slot % cols) as f32 * cell.x, (slot / cols) as f32 * cell.y);
                // 被拖的那张跟着指针走，不做归位动画（否则会「追」着手指跑）。
                let pos = if dragged {
                    // 位置照样喂进动画表，松手时才不会从旧值弹一下。
                    let p = pointer.map(|p| p - drag.map(|d| d.grab).unwrap_or_default())
                        .unwrap_or(origin + target);
                    let off = p - origin;
                    ui.ctx().animate_value_with_time(tile_anim_id(&s.path, 0), off.x, 0.0);
                    ui.ctx().animate_value_with_time(tile_anim_id(&s.path, 1), off.y, 0.0);
                    p
                } else {
                    origin
                        + egui::vec2(
                            ui.ctx().animate_value_with_time(tile_anim_id(&s.path, 0), target.x, REFLOW_TIME),
                            ui.ctx().animate_value_with_time(tile_anim_id(&s.path, 1), target.y, REFLOW_TIME),
                        )
                };
                let rect = egui::Rect::from_min_size(pos, egui::vec2(tile_w, tile_h));

                if dragged {
                    // 拖起来的那张只画外观、不放控件：手指正压在上面，
                    // 放真控件的话滑块会被顺手拖走。
                    ghost_tile(ui, rect, s, list_mode);
                    continue;
                }

                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                child.set_width(tile_w);
                child.set_max_width(tile_w);
                // 卡片底衬先 interact 再画内容：内容里的按钮/滑块画在后面、层级更高，
                // 会优先吃掉点击，所以长按只会在「按在空白处」时触发。
                let bg = child.interact(
                    rect,
                    egui::Id::new(("tile-bg", i)),
                    egui::Sense::click_and_drag(),
                );
                self.ui_sound_tile(
                    &mut child, i, s, playing.contains(&s.path), presets, tile_w, content_h,
                    list_mode, pending,
                );

                let now = child.input(|inp| inp.time);
                // 手柄：按下即拖，不用等长按。
                if pending.grip_pressed.take() == Some(i) && drag.is_none() {
                    if let Some(pos) = pointer {
                        drag = Some(DragState {
                            from: i,
                            grab: pos - rect.min,
                            active: true,
                            press_at: now,
                            press_pos: pos,
                        });
                    }
                } else if bg.is_pointer_button_down_on() && drag.is_none() {
                    if let Some(pos) = pointer {
                        drag = Some(DragState {
                            from: i,
                            grab: pos - rect.min,
                            active: false,
                            press_at: now,
                            press_pos: pos,
                        });
                    }
                }
            }

            // ── 拖动状态机 ──
            let down = ui.input(|inp| inp.pointer.any_down());
            if let Some(d) = drag.as_mut() {
                if !d.active {
                    let moved = pointer.map(|p| (p - d.press_pos).length()).unwrap_or(0.0);
                    if moved > LONG_PRESS_SLOP {
                        // 按下后马上就滑动 = 在操作控件或滚页面，不是要拖卡片。
                        // 这里只是让它**永远等不到**长按，不能直接清空 ——
                        // 清空了下一帧按键还按着，会重新计时，滑到一半停手就诈尸变成拖动。
                        d.press_at = f64::MAX;
                    } else if ui.input(|inp| inp.time) - d.press_at >= LONG_PRESS && down {
                        d.active = true;
                    }
                }
            }
            if let Some(d) = drag {
                if d.active {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    ui.ctx().request_repaint();
                }
                if !down {
                    // 松手：落到目标槽位。
                    if d.active {
                        if let (Some(f), Some(t)) = (from_slot, drop_slot) {
                            if f != t {
                                pending.reorder = Some((d.from, visible[t].0));
                            }
                        }
                    }
                    drag = None;
                }
            }
            *self.drag.borrow_mut() = drag;
        });
    }

    /// 居中的空状态提示。
    fn ui_empty_hint(&self, ui: &mut egui::Ui, text: &str) {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(text).color(theme::p().dim))
                    .wrap_mode(egui::TextWrapMode::Wrap),
            );
        });
    }

    /// 单个音效项。`compact` = 列表模式（一行放下所有东西）。
    ///
    /// 所有宽度都从外面给的 `tile_w` 精确分配、加起来正好等于卡片内宽，
    /// 绝不用 `available_width()` 反推，也绝不让某个控件「有就占位、没有就不占」——
    /// 那会让同一排卡片里的滑块长短不一、数值框对不上一条竖线。
    #[allow(clippy::too_many_arguments)]
    fn ui_sound_tile(
        &self,
        ui: &mut egui::Ui,
        i: usize,
        s: &Sound,
        is_playing: bool,
        presets: &[f32],
        tile_w: f32,
        content_h: f32,
        compact: bool,
        pending: &mut Pending,
    ) {
        let inner_w = tile_w - 24.0;
        let g = ui.spacing().item_spacing.x;
        let row_h = row_height(ui);
        let card = theme::card(is_playing).show(ui, |ui| {
            ui.set_width(inner_w);
            ui.set_max_width(inner_w);
            if compact {
                // 一行放下：手柄 12 + 播放 28 + 名字(弹性) + 快捷键 + 音量，
                // 五块宽度加四个间距 = inner_w。
                let vol_w = (inner_w * 0.28).clamp(110.0, 180.0);
                let hk_w = (inner_w * 0.22).clamp(110.0, 150.0);
                let name_w = (inner_w - 40.0 - 4.0 * g - vol_w - hk_w).max(40.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(content_h);
                    drag_handle(ui, i, pending);
                    self.tile_play_button(ui, i, pending);
                    sized_row(ui, name_w, row_h, |ui| self.tile_name(ui, s, is_playing));
                    sized_row(ui, hk_w, row_h, |ui| self.tile_hotkey(ui, i, s, pending));
                    sized_row(ui, vol_w, row_h, |ui| {
                        self.tile_volume(ui, i, s, presets, vol_w, pending)
                    });
                });
            } else {
                // 三行都用固定高度块，不让某一行按自己的内容长高 ——
                // 「按钮高 = 字高 + 内边距」和「滑块高 = interact_size」本来就不一样，
                // 放任它们各长各的，卡片实际高度就和外面算的格子对不上。
                ui.vertical(|ui| {
                    ui.set_min_height(content_h);
                    sized_row(ui, inner_w, row_h, |ui| {
                        drag_handle(ui, i, pending);
                        self.tile_play_button(ui, i, pending);
                        self.tile_name(ui, s, is_playing);
                    });
                    sized_row(ui, inner_w, row_h, |ui| self.tile_hotkey(ui, i, s, pending));
                    sized_row(ui, inner_w, row_h, |ui| {
                        self.tile_volume(ui, i, s, presets, inner_w, pending)
                    });
                });
            }
        });
        // 把卡片**真实**高度记下来给下一帧当格子高。
        // 光靠公式算不准：egui 里按钮高 = 字高 + 内边距、滑块高 = interact_size，
        // 各控件规则不同，还随字体和 DPI 变，算出来和实际差一点点，
        // 卡片就会比格子高、纵向间距被吃掉。量一次比猜十次靠谱。
        // 高度只取决于内容和样式，不受格子高影响，所以不会来回抖。
        let mut m = self.card_h.get();
        m[usize::from(compact)] = card.response.rect.height();
        self.card_h.set(m);
    }

    fn tile_play_button(&self, ui: &mut egui::Ui, i: usize, pending: &mut Pending) {
        if ui.add(egui::Button::new("▶").min_size(egui::vec2(28.0, 24.0))).clicked() {
            pending.play.push(i);
        }
    }

    fn tile_name(&self, ui: &mut egui::Ui, s: &Sound, is_playing: bool) {
        let name = egui::RichText::new(&s.name).strong().color(if is_playing {
            theme::p().accent
        } else {
            theme::p().text
        });
        ui.add(egui::Label::new(name).wrap_mode(egui::TextWrapMode::Truncate));
    }

    /// 快捷键小徽章。绑了是实心块，没绑是同尺寸的描边块 ——
    /// 以前没绑的是一行浅灰无边框文字，一排里「蓝块、灰字、灰字」轻重不一，
    /// 看着就像没排齐。
    fn tile_hotkey(&self, ui: &mut egui::Ui, i: usize, s: &Sound, pending: &mut Pending) {
        let texts = self.texts();
        let pal = theme::p();
        if self.capturing == Some(CaptureTarget::Sound(i)) {
            ui.add(
                egui::Button::new(
                    egui::RichText::new(texts.capturing_cancel).size(12.0).color(pal.text),
                )
                .fill(pal.accent),
            );
        } else if let Some(h) = &s.hotkey {
            let btn = egui::Button::new(
                egui::RichText::new(hotkeys::pretty_combo(h)).size(12.0).color(pal.text),
            )
            .fill(pal.accent_soft);
            if ui.add(btn).on_hover_text(texts.set_hotkey).clicked() {
                pending.capture = Some(CaptureTarget::Sound(i));
            }
            if ui.small_button("✖").on_hover_text(texts.clear).clicked() {
                pending.clear_hotkey.push(i);
            }
        } else {
            let btn = egui::Button::new(
                egui::RichText::new(format!("＋ {}", texts.set_hotkey_short)).size(12.0).color(pal.dim),
            )
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0, pal.border));
            if ui.add(btn).on_hover_text(texts.set_hotkey).clicked() {
                pending.capture = Some(CaptureTarget::Sound(i));
            }
        }
    }

    /// 音量：滑块 + 固定宽度的数值框，两者加一个间距正好等于 `width`。
    ///
    /// 没有「复位」按钮 —— 它以前只在音量 ≠ 1 时出现，一出现就把滑块压短 46px，
    /// 同一排的滑块长短不一、数值框错开一大截。复位挪进右键菜单（本来就有档位菜单）。
    fn tile_volume(
        &self, ui: &mut egui::Ui, i: usize, s: &Sound, presets: &[f32], width: f32,
        pending: &mut Pending,
    ) {
        let texts = self.texts();
        let g = ui.spacing().item_spacing.x;
        let mut v = s.volume;
        let mut changed: Option<f32> = None;
        ui.push_id(("vol", i), |ui| {
            ui.spacing_mut().slider_width = (width - VALUE_W - g).max(40.0);
            let sresp = ui
                .add(egui::Slider::new(&mut v, 0.0..=2.0).show_value(false))
                .on_hover_text(texts.volume_slider_tip);
            let vresp = ui.add_sized(
                egui::vec2(VALUE_W, row_height(ui)),
                egui::DragValue::new(&mut v).speed(0.01).range(0.0..=2.0).fixed_decimals(2),
            );
            if sresp.changed() || vresp.changed() {
                changed = Some(v);
            }
            // 滑块和数值框都能右键出档位菜单。
            for r in [&sresp, &vresp] {
                if let Some(pv) = volume_preset_menu(r, presets, texts.volume_preset_menu, 2.0, texts.reset) {
                    changed = Some(pv);
                }
            }
        });
        if let Some(nv) = changed {
            pending.set_volume.push((i, nv));
        }
    }

    // ─────────────────────────── 设置页 ───────────────────────────
    fn ui_settings(
        &mut self,
        ui: &mut egui::Ui,
        out_devices: &[String],
        in_devices: &[String],
        pending: &mut Pending,
    ) {
        let texts = self.texts();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let w = ui.available_width();

            // ── 外观 ──
            theme::card(false).show(ui, |ui| {
                ui.set_width(w - 26.0);
                theme::section_title(ui, texts.section_appearance);
                labeled_row(ui, texts.theme_mode, |ui| {
                    let mut m = self.config.theme_mode;
                    ui.selectable_value(&mut m, ThemeMode::System, texts.theme_system);
                    ui.selectable_value(&mut m, ThemeMode::Dark, texts.theme_dark);
                    ui.selectable_value(&mut m, ThemeMode::Light, texts.theme_light);
                    if m != self.config.theme_mode {
                        self.config.theme_mode = m;
                        // 换模式时把自定义底色清掉，否则深色的底色会被带进浅色主题里
                        self.config.bg_color = None;
                        self.config.save();
                    }
                });
                labeled_row(ui, texts.bg_color, |ui| {
                    let light = self.theme_sig.map(|t| t.0).unwrap_or(false);
                    let default_bg = if light {
                        theme::DEFAULT_LIGHT_BG
                    } else {
                        theme::DEFAULT_DARK_BG
                    };
                    let mut rgb = self.config.bg_color.unwrap_or(default_bg);
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        self.config.bg_color = Some(rgb);
                        self.config.save();
                    }
                    if self.config.bg_color.is_some() && ui.small_button(texts.reset).clicked() {
                        self.config.bg_color = None;
                        self.config.save();
                    }
                });
                labeled_row(ui, texts.bg_image, |ui| {
                    if ui.button(texts.bg_image_pick).clicked() {
                        pending.pick_bg_image = true;
                    }
                    match &self.config.bg_image {
                        Some(p) => {
                            if ui.small_button(texts.clear).clicked() {
                                pending.clear_bg_image = true;
                            }
                            let name = p
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or_default()
                                .to_string();
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(name).size(12.0).color(theme::p().dim),
                                )
                                .wrap_mode(egui::TextWrapMode::Truncate),
                            );
                        }
                        None => {
                            ui.label(
                                egui::RichText::new(texts.bg_image_none)
                                    .size(12.0)
                                    .color(theme::p().dim),
                            );
                        }
                    }
                });
                let has_img = self.config.bg_image.is_some();
                labeled_row(ui, texts.bg_opacity, |ui| {
                    // 没设图片时置灰而不是藏起来，否则用户会以为没有这个设置
                    if ui
                        .add_enabled(
                            has_img,
                            egui::Slider::new(&mut self.config.bg_opacity, 0.0..=1.0).fixed_decimals(2),
                        )
                        .changed()
                    {
                        self.config.save();
                    }
                });
                labeled_row_tip(ui, texts.window_opacity, Some(texts.window_opacity_hint), |ui| {
                    if ui
                        .add(egui::Slider::new(&mut self.config.window_opacity, 0.0..=1.0).fixed_decimals(2))
                        .changed()
                    {
                        self.config.save();
                    }
                });
                hint(ui, texts.appearance_hint, theme::p().dim);
            });

            // ── 设备 ──
            theme::card(false).show(ui, |ui| {
                ui.set_width(w - 26.0);
                theme::section_title(ui, texts.section_devices);
                labeled_row_tip(ui, texts.output_device, Some(texts.output_device_tip), |ui| {
                    device_combo(ui, "out", &mut self.config.output_device, out_devices, texts.system_default, &mut pending.rebuild_engine);
                });
                labeled_row(ui, texts.microphone, |ui| {
                    device_combo(ui, "in", &mut self.config.input_device, in_devices, texts.system_default, &mut pending.rebuild_engine);
                });
                labeled_row_tip(ui, texts.monitor_device, Some(texts.monitor_device_tip), |ui| {
                    device_combo(ui, "mon", &mut self.config.monitor_device, out_devices, texts.no_monitor, &mut pending.rebuild_engine);
                });
                if pending.rebuild_engine {
                    self.config.save();
                }
            });

            // ── 播放 ──
            theme::card(false).show(ui, |ui| {
                ui.set_width(w - 26.0);
                theme::section_title(ui, texts.section_playback);
                if ui.checkbox(&mut self.config.mic_passthrough, texts.mic_passthrough).changed() {
                    self.audio.send(AudioCmd::SetMicPassthrough(self.config.mic_passthrough));
                    self.config.save();
                }
                labeled_row(ui, texts.effect_volume, |ui| {
                    if ui
                        .add(egui::Slider::new(&mut self.config.effect_volume, 0.0..=1.5).fixed_decimals(2))
                        .changed()
                    {
                        self.audio.send(AudioCmd::SetEffectVolume(self.config.effect_volume));
                        self.config.save();
                    }
                });
                labeled_row(ui, texts.repeat_behavior, |ui| {
                    let mut changed = false;
                    changed |= ui.selectable_value(&mut self.config.repeat_mode, RepeatMode::Restart, texts.repeat_restart).clicked();
                    changed |= ui.selectable_value(&mut self.config.repeat_mode, RepeatMode::Overlap, texts.repeat_overlap).clicked();
                    changed |= ui.selectable_value(&mut self.config.repeat_mode, RepeatMode::Toggle, texts.repeat_toggle).clicked();
                    if changed {
                        self.audio.send(AudioCmd::SetRepeatMode(self.config.repeat_mode));
                        self.config.save();
                    }
                });
                labeled_row_tip(ui, texts.volume_presets, Some(texts.volume_presets_hint), |ui| {
                    let mut remove: Option<usize> = None;
                    let mut dirty = false;
                    for i in 0..self.config.volume_presets.len() {
                        ui.push_id(("preset", i), |ui| {
                            let v = &mut self.config.volume_presets[i];
                            if ui
                                .add(
                                    egui::DragValue::new(v)
                                        .speed(0.05)
                                        .range(0.0..=2.0)
                                        .fixed_decimals(2),
                                )
                                .changed()
                            {
                                dirty = true;
                            }
                            if ui.small_button("✖").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        self.config.volume_presets.remove(i);
                        dirty = true;
                    }
                    if self.config.volume_presets.len() < 8 && ui.small_button(texts.add).clicked() {
                        self.config.volume_presets.push(1.0);
                        dirty = true;
                    }
                    if dirty {
                        // 排序去重，菜单里才不会出现乱序和重复档位
                        let p = &mut self.config.volume_presets;
                        p.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                        p.dedup_by(|a, b| (*a - *b).abs() < 0.005);
                        self.config.save();
                    }
                });
                labeled_row(ui, texts.stop_all, |ui| {
                    let capturing = self.capturing == Some(CaptureTarget::Stop);
                    let label = if capturing {
                        texts.capturing_cancel.to_string()
                    } else {
                        self.config
                            .stop_hotkey
                            .as_ref()
                            .map(|h| hotkeys::pretty_combo(h))
                            .unwrap_or_else(|| texts.stop_all_not_set.to_string())
                    };
                    if ui.button(label).clicked() && !capturing {
                        pending.capture = Some(CaptureTarget::Stop);
                    }
                    if self.config.stop_hotkey.is_some() && ui.small_button("✖").clicked() {
                        pending.clear_stop = true;
                    }
                });
                // 热键诊断：「后台不触发」这种问题在别人机器上很难复现，
                // 把钩子的真实状态摆出来，用户截个图就能定位。
                if let Some(hk) = &self.hotkeys {
                    let installed = hk.hook_installed();
                    let events = hk.event_count();
                    let ok = installed && events > 0;
                    labeled_row_tip(ui, texts.hook_diag, Some(texts.hook_diag_tip), |ui| {
                        let color = if ok { theme::p().ok } else { theme::p().danger };
                        let idle = hk
                            .idle_ms()
                            .map(|ms| format!("{:.1}s", ms as f32 / 1000.0))
                            .unwrap_or_else(|| "-".into());
                        ui.colored_label(
                            color,
                            format!(
                                "hook={} events={events} idle={idle} reinstalls={}",
                                if installed { "on" } else { "off" },
                                hk.reinstalls(),
                            ),
                        );
                    });
                    if !ok {
                        hint(ui, texts.hook_dead_hint, theme::p().warn);
                    }
                    if ui.small_button(texts.open_log).on_hover_text(texts.open_log_tip).clicked() {
                        platform::open_folder(&crate::config::data_root());
                    }
                }
            });

            // ── 系统音频 ──
            theme::card(false).show(ui, |ui| {
                ui.set_width(w - 26.0);
                theme::section_title(ui, texts.loopback_title);
                if ui.checkbox(&mut self.config.loopback_enabled, texts.loopback_enable).changed() {
                    pending.rebuild_engine = true;
                    self.config.save();
                }
                if self.config.loopback_enabled {
                    labeled_row_tip(ui, texts.capture_device, Some(texts.capture_device_tip), |ui| {
                        device_combo(
                            ui,
                            "cap",
                            &mut self.config.capture_device,
                            out_devices,
                            texts.capture_default,
                            &mut pending.rebuild_engine,
                        );
                    });
                    labeled_row(ui, texts.loopback_volume, |ui| {
                        if ui
                            .add(egui::Slider::new(&mut self.config.loopback_volume, 0.0..=2.0).fixed_decimals(2))
                            .changed()
                        {
                            self.audio.send(AudioCmd::SetLoopbackVolume(self.config.loopback_volume));
                            self.config.save();
                        }
                    });
                }
                hint(ui, texts.loopback_hint, theme::p().dim);
                if self.config.loopback_enabled {
                    let cap = self.config.capture_device.as_deref();
                    if audio::loopback_monitor_conflicts(self.config.monitor_device.as_deref(), cap) {
                        hint(ui, texts.loopback_monitor_conflict, theme::p().warn);
                    }
                    if audio::same_output(cap, self.config.output_device.as_deref()) {
                        hint(ui, texts.capture_is_output, theme::p().dim);
                    }
                    if pending.rebuild_engine {
                        self.config.save();
                    }
                }
                ui.add_space(8.0);
                theme::section_title(ui, texts.app_audio_routing);
                hint(ui, texts.app_routing_hint, theme::p().dim);
                if ui.button(texts.open_app_volume).clicked() {
                    platform::open_app_volume_settings();
                }
            });

            // ── 启动 ──
            theme::card(false).show(ui, |ui| {
                ui.set_width(w - 26.0);
                theme::section_title(ui, texts.section_startup);
                if ui.checkbox(&mut self.config.autostart, texts.autostart).changed() {
                    if let Err(e) = platform::set_autostart(self.config.autostart) {
                        log::error!("设置开机自启失败：{e}");
                    }
                    self.config.save();
                }
            });

            // ── 出错时才出现的一张卡 ──
            if let Some(err) = self.audio.last_error() {
                theme::card(false).show(ui, |ui| {
                    ui.set_width(w - 26.0);
                    ui.colored_label(theme::p().danger, format!("{}{err}", texts.audio_engine_error));
                    if ui.button(texts.retry).clicked() {
                        pending.rebuild_engine = true;
                    }
                });
            }
        });
    }
}

impl eframe::App for App {
    /// 窗口清除色。整窗不透明度 < 1 时，这里的 alpha 决定桌面透进来多少。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::clear_color()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 始终以 ~50ms 刷帧：保证快捷键触发响应及时（egui 后备触发依赖刷帧）。
        ctx.request_repaint_after(Duration::from_millis(50));
        self.sync_theme(ctx);
        if let Some(tex) = &self.bg_tex {
            theme::paint_background(ctx, tex, self.config.bg_opacity);
        }
        self.maybe_rescan();
        let was_capturing = self.capturing.is_some();
        self.handle_capture(ctx);
        // 捕获完成的这一帧不触发（同一个按键事件会在捕获和触发里各处理一次）
        if !was_capturing {
            self.poll_egui_trigger(ctx);
        }

        let out_devices = self.out_devices.clone();
        let in_devices = self.in_devices.clone();
        let profiles = self.profiles.clone();
        let sounds = self.profile.as_ref().map(|p| p.sounds.clone()).unwrap_or_default();
        let playing = self.audio.playing_snapshot();
        let presets = self.config.volume_presets.clone();
        let mut pending = Pending::default();

        egui::TopBottomPanel::top("topbar")
            .frame(
                egui::Frame::none()
                    .fill(theme::surface_fill())
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show(ctx, |ui| self.ui_topbar(ui, &mut pending));

        egui::TopBottomPanel::bottom("statusbar")
            .frame(
                egui::Frame::none()
                    .fill(theme::surface_fill())
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
            )
            .show(ctx, |ui| self.ui_statusbar(ui, &mut pending));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(theme::body_fill())
                    .inner_margin(egui::Margin::same(12.0)),
            )
            .show(ctx, |ui| match self.page {
                Page::Sounds => {
                    self.ui_sounds(ui, &profiles, &sounds, &playing, &presets, &mut pending)
                }
                Page::Settings => self.ui_settings(ui, &out_devices, &in_devices, &mut pending),
            });

        self.apply(pending);

        // 每 ~2 秒记一次窗口几何，下次启动恢复。
        // 注意：ctx.screen_rect() 的原点恒为 (0,0)，拿不到窗口在屏幕上的位置，
        // 位置必须从 viewport 信息里的 outer_rect 取（和启动时 with_position 对应的就是外框）。
        if self.last_window_save.elapsed() >= Duration::from_secs(2) {
            self.last_window_save = Instant::now();
            let (outer, inner, minimized) = ctx.input(|i| {
                let vp = i.viewport();
                (vp.outer_rect, vp.inner_rect, vp.minimized.unwrap_or(false))
            });
            // 最小化时窗口坐标没有意义，别把它存下来。
            if !minimized {
                let new_x = outer.map(|r| r.min.x).or(self.config.window_x);
                let new_y = outer.map(|r| r.min.y).or(self.config.window_y);
                let size = inner.map(|r| r.size()).unwrap_or_else(|| ctx.screen_rect().size());
                let new_w = Some(size.x);
                let new_h = Some(size.y);
                if self.config.window_x != new_x || self.config.window_y != new_y
                    || self.config.window_w != new_w || self.config.window_h != new_h
                {
                    self.config.window_x = new_x;
                    self.config.window_y = new_y;
                    self.config.window_w = new_w;
                    self.config.window_h = new_h;
                    self.config.save();
                }
            }
        }
    }
}

/// 卡片左上角的拖动手柄。用 painter 画六个点，不依赖字体里有没有对应字形。
///
/// 按住手柄**立刻**进入拖动；卡片其余空白处则要长按 —— 那里叠着滑块和按钮，
/// 一按就拖会把它们的手势抢走。
fn drag_handle(ui: &mut egui::Ui, i: usize, pending: &mut Pending) {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(12.0, 24.0),
        egui::Sense::click_and_drag(),
    );
    if resp.is_pointer_button_down_on() {
        pending.grip_pressed = Some(i);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let c = if resp.hovered() { theme::p().accent } else { theme::p().dim };
    let p = ui.painter();
    for row in 0..3 {
        for col in 0..2 {
            let pos = egui::pos2(
                rect.center().x + (col as f32 - 0.5) * 5.0,
                rect.center().y + (row as f32 - 1.0) * 5.0,
            );
            p.circle_filled(pos, 1.4, c);
        }
    }
}

/// 卡片里一行的统一高度。
///
/// 取「最高的那种控件」：按钮 = 文字行高 + 上下内边距，滑块 / 数值框 = interact_size.y。
/// 直接拿 interact_size.y 当行高会偏小 —— 正文 14px 的按钮比它高，
/// 于是每张卡都悄悄比算出来的格子高一截，纵向间距被吃掉。
fn row_height(ui: &egui::Ui) -> f32 {
    let pad = ui.spacing().button_padding.y * 2.0;
    let body = ui.text_style_height(&egui::TextStyle::Body);
    let small = ui.fonts(|f| f.row_height(&egui::FontId::proportional(12.0)));
    (body + pad).max(small + pad).max(ui.spacing().interact_size.y)
}

/// 在一行里占一块**固定宽高**的地方再放内容。
/// 内容画多宽都不会影响后面控件的位置，一排卡片才能对齐成竖线。
fn sized_row(ui: &mut egui::Ui, w: f32, h: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(w, h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_max_width(w);
            add(ui);
        },
    );
}

/// 每张卡片的动画位置存两个 id（x / y），按文件路径做键 ——
/// 用下标做键的话，一换顺序动画就串到别人身上了。
fn tile_anim_id(path: &std::path::Path, axis: u8) -> egui::Id {
    egui::Id::new(("tile-pos", path, axis))
}

/// 拖起来的那张卡的「影子」。只画外观不放控件，抬高一点 + 描边，
/// 让人一眼看出它正被拿在手上。
fn ghost_tile(ui: &mut egui::Ui, rect: egui::Rect, s: &Sound, compact: bool) {
    let pal = theme::p();
    let painter = ui.painter();
    let rounding = egui::Rounding::same(theme::R_CARD);
    // 手绘阴影：往右下偏一点画一层暗色，比引擎的 shadow 更可控。
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 3.0)),
        rounding,
        egui::Color32::from_black_alpha(60),
    );
    painter.rect_filled(rect, rounding, pal.surface);
    painter.rect_stroke(rect, rounding, egui::Stroke::new(1.5, pal.accent));
    let text_pos = rect.min + egui::vec2(14.0, if compact { rect.height() * 0.5 - 8.0 } else { 12.0 });
    painter.text(
        text_pos,
        egui::Align2::LEFT_TOP,
        &s.name,
        egui::FontId::proportional(14.0),
        pal.text,
    );
    if let Some(h) = &s.hotkey {
        painter.text(
            text_pos + egui::vec2(0.0, 22.0),
            egui::Align2::LEFT_TOP,
            hotkeys::pretty_combo(h),
            egui::FontId::proportional(12.0),
            pal.dim,
        );
    }
}

/// 给一个音量滑块挂右键菜单，从预设档位里一键选。返回用户选中的值。
fn volume_preset_menu(
    resp: &egui::Response,
    presets: &[f32],
    title: &str,
    max: f32,
    reset_label: &str,
) -> Option<f32> {
    let mut picked = None;
    resp.context_menu(|ui| {
        ui.label(egui::RichText::new(title).size(11.5).color(theme::p().dim));
        for v in presets.iter().copied().filter(|v| *v <= max + f32::EPSILON) {
            if ui.button(format!("{v:.2}")).clicked() {
                picked = Some(v);
                ui.close_menu();
            }
        }
        ui.separator();
        // 复位从卡片里挪到这儿：卡片上「有时有、有时没有」的按钮会把排版顶歪。
        if ui.button(format!("{reset_label} 1.00")).clicked() {
            picked = Some(1.0);
            ui.close_menu();
        }
    });
    picked
}

/// 设置页里的一行：左侧固定宽度的标签 + 右侧控件，多行之间左边缘对齐。
fn labeled_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    labeled_row_tip(ui, label, None, add);
}

/// 带说明的版本：标签保持短，长解释挂在悬停提示里（写进标签会被截断）。
fn labeled_row_tip(
    ui: &mut egui::Ui,
    label: &str,
    tip: Option<&str>,
    add: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        let text = if tip.is_some() {
            format!("{label} ⓘ")
        } else {
            label.to_string()
        };
        let resp = ui.add_sized(
            egui::vec2(120.0, 20.0),
            egui::Label::new(egui::RichText::new(text).color(theme::p().dim))
                .wrap_mode(egui::TextWrapMode::Truncate),
        );
        if let Some(t) = tip {
            resp.on_hover_text(t);
        }
        add(ui);
    });
}

/// 解码一张背景图并上传成纹理。太大的图先缩到 2560 宽，避免白占显存。
fn load_bg_texture(ctx: &egui::Context, path: &std::path::Path) -> Option<egui::TextureHandle> {
    let img = match image::open(path) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("解码背景图失败 {}：{e}", path.display());
            return None;
        }
    };
    const MAX_W: u32 = 2560;
    let img = if img.width() > MAX_W {
        let h = (img.height() as f32 * MAX_W as f32 / img.width() as f32).round() as u32;
        img.resize(MAX_W, h.max(1), image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let ci = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture("bg", ci, egui::TextureOptions::LINEAR))
}

/// 一段会自动换行的小字说明。
fn hint(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.add(
        egui::Label::new(egui::RichText::new(text).size(11.5).color(color))
            .wrap_mode(egui::TextWrapMode::Wrap),
    );
}

/// 把 egui 的 Key 枚举映射成 Windows 虚拟键码（VK code），无法映射的返回 None。
fn egui_key_to_vk(key: egui::Key) -> Option<u32> {
    use egui::Key;
    Some(match key {
        Key::A => 0x41, Key::B => 0x42, Key::C => 0x43, Key::D => 0x44,
        Key::E => 0x45, Key::F => 0x46, Key::G => 0x47, Key::H => 0x48,
        Key::I => 0x49, Key::J => 0x4A, Key::K => 0x4B, Key::L => 0x4C,
        Key::M => 0x4D, Key::N => 0x4E, Key::O => 0x4F, Key::P => 0x50,
        Key::Q => 0x51, Key::R => 0x52, Key::S => 0x53, Key::T => 0x54,
        Key::U => 0x55, Key::V => 0x56, Key::W => 0x57, Key::X => 0x58,
        Key::Y => 0x59, Key::Z => 0x5A,
        Key::Num0 => 0x30, Key::Num1 => 0x31, Key::Num2 => 0x32,
        Key::Num3 => 0x33, Key::Num4 => 0x34, Key::Num5 => 0x35,
        Key::Num6 => 0x36, Key::Num7 => 0x37, Key::Num8 => 0x38,
        Key::Num9 => 0x39,
        Key::F1 => 0x70, Key::F2 => 0x71, Key::F3 => 0x72, Key::F4 => 0x73,
        Key::F5 => 0x74, Key::F6 => 0x75, Key::F7 => 0x76, Key::F8 => 0x77,
        Key::F9 => 0x78, Key::F10 => 0x79, Key::F11 => 0x7A, Key::F12 => 0x7B,
        Key::Space => 0x20,
        Key::Tab => 0x09,
        Key::Enter => 0x0D,
        Key::Backspace => 0x08,
        Key::Insert => 0x2D, Key::Delete => 0x2E,
        Key::Home => 0x24, Key::End => 0x23,
        Key::PageUp => 0x21, Key::PageDown => 0x22,
        Key::ArrowUp => 0x26, Key::ArrowDown => 0x28,
        Key::ArrowLeft => 0x25, Key::ArrowRight => 0x27,
        _ => return None,
    })
}
/// 当 egui 报告的是数字键（0x30-0x39）时，检查对应的小键盘键是否按下。
/// 如果小键盘键按下，返回小键盘 VK code（0x60-0x69），否则返回原始值。
/// 这样主键盘的「1」和小键盘的「1」可以分别绑定不同的快捷键。
#[cfg(windows)]
fn disambiguate_numpad(vk: u32) -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    if (0x30..=0x39).contains(&vk) {
        let numpad_vk = vk - 0x30 + 0x60; // Digit0=0x30 -> Numpad0=0x60
        unsafe {
            if GetAsyncKeyState(numpad_vk as i32) < 0 {
                return numpad_vk;
            }
        }
    }
    vk
}

#[cfg(not(windows))]
fn disambiguate_numpad(vk: u32) -> u32 {
    vk
}
fn device_combo(
    ui: &mut egui::Ui,
    id: &str,
    selected: &mut Option<String>,
    devices: &[String],
    none_text: &str,
    changed: &mut bool,
) {
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().min(300.0))
        .selected_text(selected.clone().unwrap_or_else(|| none_text.to_string()))
        .show_ui(ui, |ui| {
            if ui.selectable_label(selected.is_none(), none_text).clicked() && selected.is_some() {
                *selected = None;
                *changed = true;
            }
            for d in devices {
                let is_sel = selected.as_deref() == Some(d);
                if ui.selectable_label(is_sel, d).clicked() && !is_sel {
                    *selected = Some(d.clone());
                    *changed = true;
                }
            }
        });
}

fn install_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("cjk".to_owned(), egui::FontData::from_owned(bytes));
            fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "cjk".to_owned());
            fonts.families.entry(egui::FontFamily::Monospace).or_default().push("cjk".to_owned());
            ctx.set_fonts(fonts);
            return;
        }
    }
   log::warn!("没找到中文字体，中文可能显示为方块");
}
