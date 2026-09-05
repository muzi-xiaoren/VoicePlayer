//! egui 界面 + 状态管理，把配置、音频线程、热键、profile、i18n 串起来。
use crate::audio::{self, AudioCmd, AudioCtl};
use crate::config::{AppConfig, RepeatMode, ThemeMode};
use crate::hotkeys::{self, HkAction, Hotkeys};
use crate::i18n;
use crate::platform;
use crate::profile::{self, Profile, Sound};
use crate::theme;
use std::path::PathBuf;
use std::time::{Duration, Instant};
/// 音效瓦片的固定高度。三行内容 + 卡片内边距，所有卡片一样高，排版才齐。
const TILE_H: f32 = 108.0;

/// 拖动排序时带的载荷：被拖走的那一项的下标。
#[derive(Clone, Copy)]
struct DragSound(usize);

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
    sort_by_name: bool,
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
   last_scan: Instant,
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
            .map(|n| Profile::load(n, &Self::dir_for_config(&config, n)));
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
           last_scan: Instant::now(),
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
        let p = Profile::load(name, &dir);
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
                self.profile = Some(Profile::load(&name, &dir));
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
    /// 窗口有焦点时用 egui 键盘事件触发快捷键。**仅在低级钩子没装上时才走这条路** ——
    /// 钩子正常时它是全局生效的，再叠一条只会制造重复触发。
    fn poll_egui_trigger(&self, ctx: &egui::Context) {
        if self.capturing.is_some() {
            return;
        }
        let Some(hk) = self.hotkeys.as_ref() else { return };
        if hk.hook_installed() {
            return;
        }
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
            need_reregister = true;
        }
        if pending.sort_by_name {
            if let Some(p) = self.profile.as_mut() {
                p.sort_by_name();
                p.save_bindings();
            }
            need_reregister = true;
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
            let hook_ok = self.hotkeys.as_ref().map(|h| h.hook_installed()).unwrap_or(false);
            if !hook_ok {
                theme::status_dot(ui, theme::p().danger, texts.hook_failed);
                ui.separator();
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
                if ui
                    .small_button(texts.sort_by_name)
                    .on_hover_text(texts.sort_by_name_tip)
                    .clicked()
                {
                    pending.sort_by_name = true;
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

        // ── 瓦片网格 ──
        // 这里不能用 horizontal_wrapped（子 ui 高度未知，光标会逐个下漂成阶梯），
        // 也不能用 ui.columns —— columns_dyn 会按「最宽的一列」推进父 ui 的光标
        // （源码里那句 "Make sure we fit everything next frame"），只要某列内容顶到列宽，
        // 滚动区的内容宽度就会一帧帧往外长，表现就是右边被裁掉、每排还不一样宽。
        //
        // 改成：宽度只算一次（滚动条的位置先扣掉），每张卡片按固定尺寸 allocate，
        // 内部再 set_max_width 封死，任何内容都撑不出格子。
        let spacing = ui.spacing().item_spacing.x;
        let sc = &ui.spacing().scroll;
        let bar = sc.bar_width + sc.bar_inner_margin + sc.bar_outer_margin;
        let avail = (ui.available_width() - bar).max(160.0);
        const MIN_TILE: f32 = 240.0;
        let cols = (((avail + spacing) / (MIN_TILE + spacing)).floor()).max(1.0);
        let tile_w = ((avail - spacing * (cols - 1.0)) / cols).floor().max(160.0);
        let cols = cols as usize;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(avail);
            for chunk in visible.chunks(cols) {
                ui.horizontal_top(|ui| {
                    for (i, s) in chunk {
                        let is_playing = playing.contains(&s.path);
                        ui.allocate_ui_with_layout(
                            egui::vec2(tile_w, TILE_H),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_max_width(tile_w);
                                self.ui_sound_tile(ui, *i, s, is_playing, presets, tile_w, pending);
                            },
                        );
                    }
                });
            }
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

    /// 单个音效瓦片：拖动手柄 + 播放按钮 + 名字 + 快捷键徽章 + 音量。
    ///
    /// 尺寸完全由外面给的 `tile_w` / `TILE_H` 决定，内部所有控件的宽度都从它算出来，
    /// 绝不用 `available_width()` 反推 —— 否则内容一旦顶到边就会把格子撑大。
    fn ui_sound_tile(
        &self,
        ui: &mut egui::Ui,
        i: usize,
        s: &Sound,
        is_playing: bool,
        presets: &[f32],
        tile_w: f32,
        pending: &mut Pending,
    ) {
        let texts = self.texts();
        // 卡片内容的可用宽度 = 瓦片宽 - 左右内边距。
        let inner_w = tile_w - 24.0;
        let dragging_other = egui::DragAndDrop::has_payload_of_type::<DragSound>(ui.ctx());

        let frame_resp = theme::card(is_playing).show(ui, |ui| {
            ui.set_width(inner_w);
            ui.set_max_width(inner_w);
            ui.vertical(|ui| {
                ui.set_min_height(TILE_H - 24.0);

                // 第一行：拖动手柄 + 播放 + 名字（正在播时名字用强调色）
                ui.horizontal(|ui| {
                    drag_handle(ui, i);
                    if ui
                        .add(egui::Button::new("▶").min_size(egui::vec2(28.0, 24.0)))
                        .clicked()
                    {
                        pending.play.push(i);
                    }
                    let name = egui::RichText::new(&s.name).strong().color(if is_playing {
                        theme::p().accent
                    } else {
                        theme::p().text
                    });
                    ui.add(egui::Label::new(name).wrap_mode(egui::TextWrapMode::Truncate));
                });

                // 第二行：快捷键
                ui.horizontal(|ui| {
                    let capturing_this = self.capturing == Some(CaptureTarget::Sound(i));
                    if capturing_this {
                        ui.add(
                            egui::Button::new(
                                egui::RichText::new(texts.capturing_cancel)
                                    .size(12.0)
                                    .color(theme::p().text),
                            )
                            .fill(theme::p().accent),
                        );
                    } else if let Some(h) = &s.hotkey {
                        let btn = egui::Button::new(
                            egui::RichText::new(hotkeys::pretty_combo(h))
                                .size(12.0)
                                .color(theme::p().text),
                        )
                        .fill(theme::p().accent_soft);
                        if ui.add(btn).on_hover_text(texts.set_hotkey).clicked() {
                            pending.capture = Some(CaptureTarget::Sound(i));
                        }
                        if ui.small_button("✖").on_hover_text(texts.clear).clicked() {
                            pending.clear_hotkey.push(i);
                        }
                    } else {
                        let btn = egui::Button::new(
                            egui::RichText::new(format!("＋ {}", texts.set_hotkey_short))
                                .size(12.0)
                                .color(theme::p().dim),
                        )
                        .frame(false);
                        if ui.add(btn).clicked() {
                            pending.capture = Some(CaptureTarget::Sound(i));
                        }
                    }
                });

                // 第三行：单独音量（可拖、可直接输入数值、可右键选档位）
                ui.horizontal(|ui| {
                    let mut v = s.volume;
                    let mut changed: Option<f32> = None;
                    let show_reset = (v - 1.0).abs() > f32::EPSILON;
                    // 宽度从 tile_w 推，不看 available_width，撑不大格子
                    let reset_w = if show_reset { 46.0 } else { 0.0 };
                    ui.spacing_mut().slider_width = (inner_w - 62.0 - reset_w).clamp(40.0, inner_w);
                    ui.push_id(("vol", i), |ui| {
                        let resp = ui.add(
                            egui::Slider::new(&mut v, 0.0..=2.0)
                                .show_value(true)
                                .fixed_decimals(2),
                        );
                        if resp.changed() {
                            changed = Some(v);
                        }
                        // 右键这条滑块 -> 档位快选
                        if let Some(pv) =
                            volume_preset_menu(&resp, presets, texts.volume_preset_menu, 2.0)
                        {
                            changed = Some(pv);
                        }
                        if show_reset
                            && ui
                                .small_button(texts.reset)
                                .on_hover_text(texts.reset_volume_tooltip)
                                .clicked()
                        {
                            changed = Some(1.0);
                        }
                    });
                    if let Some(nv) = changed {
                        pending.set_volume.push((i, nv));
                    }
                });
            });
        });

        // 整张卡片当放置目标：拖着别的卡片经过时高亮，松手就换位置。
        if dragging_other {
            let rect = frame_resp.response.rect;
            let resp = ui.interact(rect, egui::Id::new(("drop", i)), egui::Sense::hover());
            if resp.dnd_hover_payload::<DragSound>().is_some() {
                ui.painter().rect_stroke(
                    rect,
                    egui::Rounding::same(theme::R_CARD),
                    egui::Stroke::new(2.0, theme::p().accent),
                );
            }
            if let Some(from) = resp.dnd_release_payload::<DragSound>() {
                if from.0 != i {
                    pending.reorder = Some((from.0, i));
                }
            }
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
                if self.config.loopback_enabled
                    && self.config.monitor_device.is_some()
                    && audio::loopback_monitor_conflicts(self.config.monitor_device.as_deref())
                {
                    hint(ui, texts.loopback_monitor_conflict, theme::p().warn);
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
/// 只有手柄是拖动源，卡片其余部分照常点击 —— 否则滑块会和拖动抢手势。
fn drag_handle(ui: &mut egui::Ui, i: usize) {
    let id = egui::Id::new(("drag", i));
    ui.dnd_drag_source(id, DragSound(i), |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 24.0), egui::Sense::hover());
        let c = theme::p().dim;
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
    });
}

/// 给一个音量滑块挂右键菜单，从预设档位里一键选。返回用户选中的值。
fn volume_preset_menu(
    resp: &egui::Response,
    presets: &[f32],
    title: &str,
    max: f32,
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
