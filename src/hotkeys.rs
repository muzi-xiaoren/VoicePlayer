//! 全局热键：使用 Windows 低级键盘钩子（WH_KEYBOARD_LL）。
//!
//! 与 RegisterHotKey 不同，低级钩子有以下优势：
//! - 不吞按键事件（始终调用 CallNextHookEx），绑定的键仍能正常打字。
//! - 在全屏独占游戏中也能工作。
//! - 用原始虚拟键码（VK code），精确区分主键盘数字键和小键盘数字键。
//!
//! 钩子回调运行在独立的 Windows 消息循环线程里，不依赖 egui 刷帧。
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
/// 修饰键位掩码。
pub const MOD_CTRL: u8 = 1;
pub const MOD_ALT: u8 = 2;
pub const MOD_SHIFT: u8 = 4;
/// 热键触发后要做的事。
#[derive(Debug, Clone)]
pub enum HkAction {
    Play { path: PathBuf, volume: f32 },
    StopAll,
}
// ─── VK code 常量 ───
const VK_ESCAPE: u32 = 0x1B;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
/// 把（修饰键掩码 + VK code）打包成一个 u32 作为 HashMap 的键。
fn combo_key(mods: u8, vk: u32) -> u32 {
    ((mods as u32) << 16) | (vk & 0xFFFF)
}
/// VK code → 展示用 token 字符串。None 表示不支持绑定。
fn vk_to_token(vk: u32) -> Option<String> {
    Some(match vk {
        0x41..=0x5A => format!("Key{}", (vk as u8) as char),
        0x30..=0x39 => format!("Digit{}", vk - 0x30),
        0x60..=0x69 => format!("Numpad{}", vk - 0x60),
        0x70..=0x7B => format!("F{}", vk - 0x6F),
        0x20 => "Space".into(),
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x26 => "Up".into(),
        0x28 => "Down".into(),
        0x25 => "Left".into(),
        0x27 => "Right".into(),
        0xBA => "OemSemicolon".into(),
        0xBB => "OemPlus".into(),
        0xBC => "OemComma".into(),
        0xBD => "OemMinus".into(),
        0xBE => "OemPeriod".into(),
        0xC0 => "OemTilde".into(),
        0xDB => "OemLeftBracket".into(),
        0xDC => "OemBackslash".into(),
        0xDD => "OemRightBracket".into(),
        0xDE => "OemQuotes".into(),
        _ => return None,
    })
}
/// token 字符串 → VK code。
fn token_to_vk(token: &str) -> Option<u32> {
    let t = token.to_lowercase();
    if t.len() == 4 && t.starts_with("key") {
        if let Some(c) = t[3..].chars().next() {
            if c.is_ascii_alphabetic() {
                return Some(c.to_ascii_uppercase() as u32);
            }
        }
    }
    Some(match t.as_str() {
        "digit0" | "0" => 0x30, "digit1" | "1" => 0x31,
        "digit2" | "2" => 0x32, "digit3" | "3" => 0x33,
        "digit4" | "4" => 0x34, "digit5" | "5" => 0x35,
        "digit6" | "6" => 0x36, "digit7" | "7" => 0x37,
        "digit8" | "8" => 0x38, "digit9" | "9" => 0x39,
        "numpad0" => 0x60, "numpad1" => 0x61,
        "numpad2" => 0x62, "numpad3" => 0x63,
        "numpad4" => 0x64, "numpad5" => 0x65,
        "numpad6" => 0x66, "numpad7" => 0x67,
        "numpad8" => 0x68, "numpad9" => 0x69,
        "f1" => 0x70, "f2" => 0x71, "f3" => 0x72,
        "f4" => 0x73, "f5" => 0x74, "f6" => 0x75,
        "f7" => 0x76, "f8" => 0x77, "f9" => 0x78,
        "f10" => 0x79, "f11" => 0x7A, "f12" => 0x7B,
        "space" => 0x20, "backspace" => 0x08,
        "tab" => 0x09, "enter" => 0x0D,
        "insert" => 0x2D, "delete" => 0x2E,
        "home" => 0x24, "end" => 0x23,
        "pageup" => 0x21, "pagedown" => 0x22,
        "up" => 0x26, "down" => 0x28,
        "left" => 0x25, "right" => 0x27,
        "oemsemicolon" => 0xBA, "oemplus" => 0xBB,
        "oemcomma" => 0xBC, "oemminus" => 0xBD,
        "oemperiod" => 0xBE, "oemtilde" => 0xC0,
        "oemleftbracket" => 0xDB, "oembackslash" => 0xDC,
        "oemrightbracket" => 0xDD, "oemquotes" => 0xDE,
        _ => return None,
    })
}
/// 把 (mods, vk) 格式化成用户可读字符串，如 "Ctrl+Alt+KeyW"。
pub fn format_combo(mods: u8, vk: u32) -> String {
    let token = vk_to_token(vk).unwrap_or_else(|| format!("VK{vk:02X}"));
    let mut parts = Vec::new();
    if mods & MOD_CTRL != 0 { parts.push("Ctrl"); }
    if mods & MOD_ALT != 0 { parts.push("Alt"); }
    if mods & MOD_SHIFT != 0 { parts.push("Shift"); }
    parts.push(&token);
    parts.join("+")
}
/// 解析存储的 combo 字符串 → (mods, vk)。None = 解析失败。
pub fn parse_combo(combo: &str) -> Option<(u8, u32)> {
    let mut mods = 0u8;
    let mut vk: Option<u32> = None;
    for part in combo.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CTRL,
            "alt" | "option" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "super" | "meta" | "cmd" => {}
            tok => vk = token_to_vk(tok),
        }
    }
    vk.map(|v| (mods, v))
}
/// 把存储的 combo 字符串美化显示。
pub fn pretty_combo(combo: &str) -> String {
    parse_combo(combo)
        .map(|(m, v)| format_combo(m, v))
        .unwrap_or_else(|| combo.to_string())
}
// ─── 共享状态 ───
struct HookShared {
    actions: Mutex<HashMap<u32, HkAction>>,
    locked: AtomicBool,
    capturing: AtomicBool,
    capture_tx: Mutex<Option<Sender<Option<(u8, u32)>>>>,
    on_action: Arc<dyn Fn(HkAction) + Send + Sync>,
    hook_installed: AtomicBool,
    /// Hotkeys 句柄还活着。回调只读这个原子量，不需要锁。
    active: AtomicBool,
    dedup_key: AtomicU32,
    dedup_time: AtomicU64,
}
/// 钩子回调要用到的共享状态。回调是**系统级热路径**（每一次按键都会走），
/// 所以这里用 OnceLock 而不是 Mutex：回调里不加锁，避免被 UI 线程卡住 ——
/// 低级钩子回调超时（默认 300ms）会被 Windows 直接摘掉，且不会有任何通知。
static SHARED: OnceLock<Arc<HookShared>> = OnceLock::new();
/// UI 端的热键管理句柄。
pub struct Hotkeys {
    shared: Arc<HookShared>,
    capture_rx: Option<Receiver<Option<(u8, u32)>>>,
}
impl Hotkeys {
    pub fn new<F>(on_action: F) -> anyhow::Result<Self>
    where
        F: Fn(HkAction) + Send + Sync + 'static,
    {
        let shared = Arc::new(HookShared {
            actions: Mutex::new(HashMap::new()),
            locked: AtomicBool::new(false),
            capturing: AtomicBool::new(false),
            capture_tx: Mutex::new(None),
            on_action: Arc::new(on_action),
            hook_installed: AtomicBool::new(false),
            active: AtomicBool::new(true),
            dedup_key: AtomicU32::new(0),
            dedup_time: AtomicU64::new(0),
        });
        #[cfg(windows)]
        {
            let s = shared.clone();
            std::thread::Builder::new()
                .name("hotkeys".into())
                .spawn(move || hook_thread(s))
                .map_err(|e| anyhow::anyhow!("创建热键线程失败：{e}"))?;
            // 给钩子线程一点时间启动
        }
        Ok(Self {
            shared,
            capture_rx: None,
        })
    }
    pub fn update_bindings(&self, bindings: &[(String, HkAction)]) {
        let mut map = HashMap::new();
        for (combo, action) in bindings {
            if let Some((mods, vk)) = parse_combo(combo) {
                map.insert(combo_key(mods, vk), action.clone());
            } else {
                log::warn!("无法解析快捷键「{combo}」");
            }
        }
        if let Ok(mut a) = self.shared.actions.lock() {
            *a = map;
        }
    }
    pub fn clear(&self) {
        if let Ok(mut a) = self.shared.actions.lock() {
            a.clear();
        }
    }
    pub fn set_locked(&self, locked: bool) {
        self.shared.locked.store(locked, Ordering::Relaxed);
    }
    pub fn start_capture(&mut self) {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut slot) = self.shared.capture_tx.lock() {
            *slot = Some(tx);
        }
        self.shared.capturing.store(true, Ordering::Relaxed);
        self.capture_rx = Some(rx);
    }
    pub fn poll_capture(&self) -> Option<Option<(u8, u32)>> {
        self.capture_rx.as_ref()?.try_recv().ok()
    }
    pub fn stop_capture(&self) {
        self.shared.capturing.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.shared.capture_tx.lock() {
            *slot = None;
        }
    }
   pub fn is_capturing(&self) -> bool {
       self.shared.capturing.load(Ordering::Relaxed)
   }
    /// 检查低级钩子是否成功安装。
    pub fn hook_installed(&self) -> bool {
        self.shared.hook_installed.load(Ordering::Relaxed)
    }
    /// 用 VK code 直接触发（egui 后备路径调用）。
    pub fn trigger_from_vk(&self, mods: u8, vk: u32) {
        self.shared.fire_if_bound(mods, vk);
    }
}

/// 当前时间的毫秒数（用于去重时间戳）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl HookShared {
    /// 查找绑定并触发动作。内置锁定检查 + 去重（防止钩子和 egui 同时触发同一按键）。
    fn fire_if_bound(&self, mods: u8, vk: u32) {
        if self.locked.load(Ordering::Relaxed) {
            return;
        }
        let key = combo_key(mods, vk);
        let now = now_ms();
        let prev_key = self.dedup_key.load(Ordering::Relaxed);
        let prev_time = self.dedup_time.load(Ordering::Relaxed);
        // 只用来挡「钩子和 egui 后备路径撞同一次按键」，不是用来限制连按频率的。
        // 原来是 300ms，会把 300ms 内的连按整个吃掉，
        // 「叠加再播」那种模式下手速快一点就丢音。
        if prev_key == key && now.saturating_sub(prev_time) < 40 {
            return;
        }
        self.dedup_key.store(key, Ordering::Relaxed);
        self.dedup_time.store(now, Ordering::Relaxed);
        if let Some(action) = self.actions.lock().ok().and_then(|m| m.get(&key).cloned()) {
            (self.on_action)(action);
        }
    }
}
impl Drop for Hotkeys {
    fn drop(&mut self) {
        self.shared.active.store(false, Ordering::Relaxed);
    }
}
fn is_modifier_key(vk: u32) -> bool {
    matches!(
        vk,
        VK_LSHIFT | VK_RSHIFT | VK_LCONTROL | VK_RCONTROL | VK_LMENU | VK_RMENU
    )
}
#[cfg(windows)]
mod win {
   use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
   use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
        HHOOK, MSG, WH_KEYBOARD_LL,
    };
    // hook_callback 在父模块里用到这两个，所以 pub(super)
    pub(super) use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, KBDLLHOOKSTRUCT,
    };
    pub(super) const HC_ACTION: i32 = 0;
    pub(super) const WM_KEYDOWN: usize = 0x0100;
    pub(super) const WM_KEYUP: usize = 0x0101;
    pub(super) const WM_SYSKEYDOWN: usize = 0x0104;
    pub(super) const WM_SYSKEYUP: usize = 0x0105;
    pub(super) unsafe fn install_hook() -> Option<HHOOK> {
        let hinst = GetModuleHandleW(std::ptr::null_mut());
        if hinst.is_null() {
            log::error!("GetModuleHandleW 失败");
            return None;
        }
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hinst, 0);
        if hook.is_null() {
            log::error!("SetWindowsHookExW 失败");
            return None;
        }
        Some(hook)
    }
    pub(super) unsafe fn run_message_loop() {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0); // HWND=isize，传 0 = 线程全部窗口
            if ret == 0 || ret == -1 {
                break;
            }
        }
    }
    pub(super) unsafe fn remove_hook(hook: HHOOK) {
        UnhookWindowsHookEx(hook);
    }
    pub(super) fn get_modifiers() -> u8 {
        use super::{
            VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU,
            VK_LSHIFT, VK_RSHIFT, MOD_CTRL, MOD_ALT, MOD_SHIFT,
        };
        unsafe {
            let mut mods = 0u8;
            if GetAsyncKeyState(VK_LCONTROL as i32) < 0
                || GetAsyncKeyState(VK_RCONTROL as i32) < 0
            {
                mods |= MOD_CTRL;
            }
            if GetAsyncKeyState(VK_LMENU as i32) < 0
                || GetAsyncKeyState(VK_RMENU as i32) < 0
            {
                mods |= MOD_ALT;
            }
            if GetAsyncKeyState(VK_LSHIFT as i32) < 0
                || GetAsyncKeyState(VK_RSHIFT as i32) < 0
            {
                mods |= MOD_SHIFT;
            }
            mods
        }
    }
    unsafe extern "system" fn hook_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        crate::hotkeys::hook_callback(code, wparam, lparam)
    }
}
#[cfg(windows)]
fn hook_callback(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == win::HC_ACTION {
        let kb = unsafe { &*(lparam as *const win::KBDLLHOOKSTRUCT) };
        let vk = kb.vkCode;
        let is_down = wparam == win::WM_KEYDOWN || wparam == win::WM_SYSKEYDOWN;
        let is_up = wparam == win::WM_KEYUP || wparam == win::WM_SYSKEYUP;
        if is_down || is_up {
            if let Some(shared) = SHARED.get().filter(|s| s.active.load(Ordering::Relaxed)) {
                if shared.capturing.load(Ordering::Relaxed) {
                    if is_down && !is_modifier_key(vk) {
                        let result = if vk == VK_ESCAPE { None } else { Some((win::get_modifiers(), vk)) };
                        shared.capturing.store(false, Ordering::Relaxed);
                        if let Ok(slot) = shared.capture_tx.lock() {
                            if let Some(tx) = slot.as_ref() { let _ = tx.send(result); }
                        }
                    }
                } else {
                    let mods = win::get_modifiers();
                    // 「这个键是不是已经按着」必须只按 vk 记，**不能带修饰键**：
                    // 按下时 mods 和松开时 mods 可能不一样（游戏里 Shift/Ctrl 常年按着，
                    // 中途按一下或松一下就变了），键不同就配不上对，
                    // 松开的记录删不掉，这个键从此被当成「一直按着」，再也不会触发。
                    // 这就是「后台按键突然不响，重启才好」的根因。
                    if is_down {
                        let now = now_ms();
                        let already_down = DOWN_KEYS.with(|dk| {
                            let mut dk = dk.borrow_mut();
                            // 兜底：万一漏收了 key-up（切到安全桌面、钩子被临时摘掉等），
                            // 超过 STALE_MS 没再收到该键任何事件就视为已松开。
                            dk.retain(|_, t| now.saturating_sub(*t) < STALE_MS);
                            // insert 返回旧值：有旧值 = 之前就按着（自动重复），
                            // 同时刷新时间戳，所以一直按着的键不会被上面的清理误伤。
                            dk.insert(vk, now).is_some()
                        });
                        if !already_down {
                            shared.fire_if_bound(mods, vk);
                        }
                    } else if is_up {
                        DOWN_KEYS.with(|dk| { dk.borrow_mut().remove(&vk); });
                    }
                }
            }
        }
    }
    unsafe { win::CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}
/// 多久没再收到某个键的事件就认为它已经松开（漏收 key-up 时的兜底）。
#[cfg(windows)]
const STALE_MS: u64 = 5_000;

// 当前物理按下的键：vk -> 最近一次 key-down 的时刻。只在钩子线程里访问。
#[cfg(windows)]
thread_local! {
    static DOWN_KEYS: std::cell::RefCell<HashMap<u32, u64>> =
        std::cell::RefCell::new(HashMap::new());
}
#[cfg(windows)]
fn hook_thread(shared: Arc<HookShared>) {
    let _ = SHARED.set(shared.clone());
    unsafe {
        let hook = match win::install_hook() {
            Some(h) => h,
            None => {
                shared.active.store(false, Ordering::Relaxed);
                return;
            }
        };
        shared.hook_installed.store(true, Ordering::Relaxed);
        log::info!("低级键盘钩子已安装");
        win::run_message_loop();
        win::remove_hook(hook);
        log::info!("低级键盘钩子已卸载");
    }
    shared.active.store(false, Ordering::Relaxed);
}
#[cfg(not(windows))]
fn hook_thread(_shared: Arc<HookShared>) {}
