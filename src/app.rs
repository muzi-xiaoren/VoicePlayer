//! egui 界面 + 状态管理，把配置、音频线程、热键、profile、i18n 串起来。
use crate::audio::{self, AudioCmd, AudioCtl};
use crate::config::{AppConfig, RepeatMode};
use crate::hotkeys::{self, HkAction, Hotkeys};
use crate::i18n;
use crate::platform;
use crate::profile::{self, Profile, Sound};
use std::path::PathBuf;
use std::time::{Duration, Instant};
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
   last_scan: Instant,
   last_signature: Vec<String>,
   lang: i18n::Lang,
    last_window_save: Instant,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
       install_cjk_fonts(&cc.egui_ctx);
        apply_dark_theme(&cc.egui_ctx);
       let mut config = AppConfig::load();
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
    /// 当窗口有焦点时，用 egui 键盘事件触发已绑定的快捷键（钩子失效时的后备路径）。
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
        for (i, v) in pending.set_volume {
            if let Some(p) = self.profile.as_mut() {
                if let Some(s) = p.sounds.get_mut(i) {
                    s.volume = v;
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
                    }
                } else if !input.contains('/') && !input.contains('\\') && profile::create_profile(&input).is_ok() {
                    self.profiles = Self::all_profiles(&self.config, texts.default_profile);
                    self.switch_profile(&input);
                    self.new_profile_name.clear();
                }
            }
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
    }
    fn ui_settings(
        &mut self,
        ui: &mut egui::Ui,
        out_devices: &[String],
        in_devices: &[String],
        pending: &mut Pending,
    ) {
        let texts = self.texts();
        // —— 顶部行：标题 + 锁定 + 语言 ——
        ui.horizontal(|ui| {
            ui.heading(texts.title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 语言下拉
                let mut lang_sel = self.lang;
                egui::ComboBox::from_label(texts.language)
                    .selected_text(self.lang.display())
                    .show_ui(ui, |ui| {
                        for l in i18n::Lang::all() {
                            if ui.selectable_value(&mut lang_sel, l, l.display()).changed() && l != self.lang {
                                self.lang = l;
                                pending.lang_changed = true;
                            }
                       }
                   });
                ui.separator();
                // 锁定按钮
                let (lock_label, lock_tooltip) = if self.config.locked {
                    (texts.lock, texts.lock_tooltip)
                } else {
                    (texts.unlock, texts.unlock_tooltip)
                };
                let prev = self.config.locked;
                if ui.button(lock_label).on_hover_text(lock_tooltip).clicked() {
                    self.config.locked = !self.config.locked;
                }
                if self.config.locked != prev {
                    pending.lock_toggled = true;
                }
            });
        });
        ui.add_space(4.0);
        // VB-CABLE 状态
        if self.vbcable {
            ui.colored_label(egui::Color32::from_rgb(60, 170, 90), texts.vbcable_detected);
        } else {
            ui.colored_label(egui::Color32::from_rgb(200, 120, 40), texts.vbcable_not_detected);
            ui.horizontal(|ui| {
                if ui.button(texts.open_vbcable_url).clicked() {
                    platform::open_url(platform::VBCABLE_URL);
                }
                if ui.button(texts.reinstall_detect).clicked() {
                    self.refresh_devices();
                    pending.rebuild_engine = true;
                }
            });
        }
        ui.separator();
        device_combo(ui, texts.output_device, &mut self.config.output_device, out_devices, texts.system_default, &mut pending.rebuild_engine);
        device_combo(ui, texts.microphone, &mut self.config.input_device, in_devices, texts.system_default, &mut pending.rebuild_engine);
        device_combo(ui, texts.monitor_device, &mut self.config.monitor_device, out_devices, texts.no_monitor, &mut pending.rebuild_engine);
        if pending.rebuild_engine {
            self.config.save();
        }
        ui.separator();
        if ui.checkbox(&mut self.config.mic_passthrough, texts.mic_passthrough).changed() {
            self.audio.send(AudioCmd::SetMicPassthrough(self.config.mic_passthrough));
            self.config.save();
        }
        ui.horizontal(|ui| {
            ui.label(texts.effect_volume);
            let resp = ui.add(egui::Slider::new(&mut self.config.effect_volume, 0.0..=1.5));
            if resp.changed() {
                self.audio.send(AudioCmd::SetEffectVolume(self.config.effect_volume));
                self.config.save();
            }
        });
        ui.horizontal(|ui| {
            ui.label(texts.repeat_behavior);
            let mut changed = false;
            changed |= ui.selectable_value(&mut self.config.repeat_mode, RepeatMode::Restart, texts.repeat_restart).clicked();
            changed |= ui.selectable_value(&mut self.config.repeat_mode, RepeatMode::Overlap, texts.repeat_overlap).clicked();
            changed |= ui.selectable_value(&mut self.config.repeat_mode, RepeatMode::Toggle, texts.repeat_toggle).clicked();
            if changed {
               self.audio.send(AudioCmd::SetRepeatMode(self.config.repeat_mode));
               self.config.save();
           }
       });
        ui.separator();
        ui.label(texts.app_audio_routing);
        ui.label(texts.app_routing_hint);
       if ui.button(texts.open_app_volume).clicked() {
           platform::open_app_volume_settings();
       }
        ui.separator();
        ui.label(texts.loopback_title);
        if ui.checkbox(&mut self.config.loopback_enabled, texts.loopback_enable).changed() {
            pending.rebuild_engine = true;
            self.config.save();
        }
        if self.config.loopback_enabled {
            ui.horizontal(|ui| {
                ui.label(texts.loopback_volume);
                let resp = ui.add(egui::Slider::new(&mut self.config.loopback_volume, 0.0..=2.0));
                if resp.changed() {
                    self.audio.send(AudioCmd::SetLoopbackVolume(self.config.loopback_volume));
                    self.config.save();
                }
            });
        }
        ui.add_space(2.0);
        ui.label(
            egui::Label::new(egui::RichText::new(texts.loopback_hint).small().color(egui::Color32::from_gray(140)))
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
        ui.separator();
       if ui.checkbox(&mut self.config.autostart, texts.autostart).changed() {
            if let Err(e) = platform::set_autostart(self.config.autostart) {
                log::error!("设置开机自启失败：{e}");
            }
            self.config.save();
        }
        if let Some(err) = self.audio.last_error() {
            ui.separator();
            ui.colored_label(egui::Color32::from_rgb(210, 70, 70), format!("{}{err}", texts.audio_engine_error));
            if ui.button(texts.retry).clicked() {
                pending.rebuild_engine = true;
            }
        }
    }
    fn ui_sounds(
        &mut self,
        ui: &mut egui::Ui,
        profiles: &[String],
        sounds: &[Sound],
        pending: &mut Pending,
    ) {
        let texts = self.texts();
        ui.horizontal(|ui| {
            let cur = self.config.active_profile.clone().unwrap_or_else(|| texts.none_label.to_string());
            egui::ComboBox::from_label(texts.profile)
                .selected_text(cur)
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
                });
            if ui.button(texts.open_folder).clicked() {
                pending.open_folder = true;
            }
            let is_external = self.config.active_profile.as_ref().map(|n| self.config.external_profiles.contains_key(n)).unwrap_or(false);
            if is_external && ui.button(texts.remove).on_hover_text(texts.remove_tooltip).clicked() {
                pending.remove_external = self.config.active_profile.clone();
            }
        });
        ui.horizontal(|ui| {
            ui.label(texts.new_profile);
            ui.text_edit_singleline(&mut self.new_profile_name).on_hover_text(texts.new_profile_placeholder);
            if ui.button(texts.create).clicked() {
                pending.new_profile = Some(self.new_profile_name.clone());
            }
            if ui.button(texts.select_folder).on_hover_text(texts.select_folder_tooltip).clicked() {
                pending.pick_folder = true;
            }
        });
        ui.horizontal(|ui| {
            let label = self.config.stop_hotkey.as_ref().map(|h| hotkeys::pretty_combo(h)).unwrap_or_else(|| texts.stop_all_not_set.to_string());
            ui.label(format!("{}{label}", texts.stop_all));
            if ui.button(texts.set_hotkey).clicked() {
                pending.capture = Some(CaptureTarget::Stop);
            }
            if self.config.stop_hotkey.is_some() && ui.button(texts.clear).clicked() {
                pending.clear_stop = true;
            }
        });
        ui.separator();
        if sounds.is_empty() {
            ui.label(texts.empty_hint);
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, s) in sounds.iter().enumerate() {
               ui.horizontal(|ui| {
                    if ui.button("▶").clicked() {
                        pending.play.push(i);
                    }
                    // 给右侧控件预留固定宽度，文件名用剩余空间截断显示
                    let avail = ui.available_width();
                    let controls_w = 290.0_f32;
                    let label_w = (avail - controls_w).max(60.0).min(avail - 40.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(label_w, 18.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.add(
                                egui::Label::new(&s.name)
                                    .wrap_mode(egui::TextWrapMode::Truncate),
                            );
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                       let mut v = s.volume;
                        let resp = ui.add_sized(
                            egui::vec2(60.0, 16.0),
                            egui::Slider::new(&mut v, 0.0..=1.5)
                                .show_value(false)
                                .fixed_decimals(1),
                        );
                       if resp.changed() {
                           pending.set_volume.push((i, v));
                       }
                       let capturing_this = self.capturing == Some(CaptureTarget::Sound(i));
                        if s.hotkey.is_some() && ui.button("✖").clicked() {
                            pending.clear_hotkey.push(i);
                        }
                        let btn_label = if capturing_this {
                            texts.capturing_cancel.to_string()
                        } else {
                            s.hotkey.as_ref().map(|h| hotkeys::pretty_combo(h)).unwrap_or_else(|| texts.set_hotkey_short.to_string())
                        };
                        if ui.button(btn_label).clicked() && !capturing_this {
                            pending.capture = Some(CaptureTarget::Sound(i));
                        }
                    });
                });
            }
        });
    }
}
impl eframe::App for App {
   fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 始终以 ~50ms 刷帧：保证快捷键触发响应及时（egui 后备触发依赖刷帧）。
        ctx.request_repaint_after(Duration::from_millis(50));
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
        let mut pending = Pending::default();
        egui::TopBottomPanel::top("settings").resizable(false).show(ctx, |ui| {
            self.ui_settings(ui, &out_devices, &in_devices, &mut pending);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.ui_sounds(ui, &profiles, &sounds, &mut pending);
        });
       self.apply(pending);
        // Save window geometry every ~2 seconds so position/size persist across restarts.
        if self.last_window_save.elapsed() >= Duration::from_secs(2) {
            self.last_window_save = Instant::now();
            let size = ctx.screen_rect().size();
            let pos = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min));
            let new_w = Some(size.x);
            let new_h = Some(size.y);
            let (new_x, new_y) = match pos {
                Some(p) => (Some(p.x), Some(p.y)),
                None => (self.config.window_x, self.config.window_y),
            };
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
    label: &str,
    selected: &mut Option<String>,
    devices: &[String],
    none_text: &str,
    changed: &mut bool,
) {
    egui::ComboBox::from_label(label)
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

/// Apply a professional dark theme inspired by OBS / Voicemeeter.
fn apply_dark_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = egui::Color32::from_rgb(24, 27, 38);
    v.window_fill = egui::Color32::from_rgb(28, 32, 44);
    v.extreme_bg_color = egui::Color32::from_rgb(14, 17, 24);
    v.selection.bg_fill = egui::Color32::from_rgb(56, 112, 200);
    v.widget_noninteractive.bg_fill = egui::Color32::from_rgb(24, 27, 38);
    v.widget_noninteractive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(150, 155, 170));
    v.widget_inactive.bg_fill = egui::Color32::from_rgb(36, 41, 56);
    v.widget_inactive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 205, 220));
    v.widget_hovered.bg_fill = egui::Color32::from_rgb(48, 55, 72);
    v.widget_hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(230, 235, 245));
    v.widget_active.bg_fill = egui::Color32::from_rgb(56, 65, 88);
    v.widget_active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(240, 245, 255));
    ctx.set_visuals(v);
}
