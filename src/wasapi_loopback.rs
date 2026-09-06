//! WASAPI 系统音频环回捕获。
//!
//! 捕获默认播放设备正在播放的音频，推入环形缓冲区，
//! 供 LoopbackSource 通过 rodio 混入虚拟麦克风。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// 环形缓冲区 + 采样率信息。
///
/// 捕获到的样本要送给两路消费者（虚拟麦克风 + 耳机监听），而 `next()` 是「弹出」
/// 语义，两路共用一个队列会互相抢样本，所以各自一条队列，捕获线程同时写两份。
pub struct LoopbackBuf {
    /// 交错 f32 样本（始终 stereo），送往主输出（CABLE Input）。
    pub samples: Mutex<VecDeque<f32>>,
    /// 同样的样本，送往监听设备（耳机）。只在 `mon_enabled` 为真时写入。
    pub mon: Mutex<VecDeque<f32>>,
    /// 是否有监听消费者。没有时不写 `mon`，省一次拷贝。
    pub mon_enabled: AtomicBool,
    /// 捕获线程确定采样率后写入。
    pub sample_rate: OnceLock<u32>,
}

impl LoopbackBuf {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            samples: Mutex::new(VecDeque::with_capacity(Self::CAP)),
            mon: Mutex::new(VecDeque::new()),
            mon_enabled: AtomicBool::new(false),
            sample_rate: OnceLock::new(),
        })
    }
    const CAP: usize = 48_000 * 2 * 2;
}

/// 启动环回捕获线程。
///
/// `device` = 要捕获哪个**播放设备**的声音（按设备名匹配，和设置里的输出设备列表同一套名字）。
/// None = 系统默认播放设备。把某个程序（比如网易云）单独指到 CABLE Input 之后，
/// 它的声音就不在默认设备上了，这时候必须显式选 CABLE Input 才捕得到。
#[cfg(windows)]
pub fn start_loopback_thread(
    buf: Arc<LoopbackBuf>,
    stop: Arc<AtomicBool>,
    device: Option<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("wasapi-loopback".into())
        .spawn(move || {
            if let Err(e) = run_loopback(&buf, &stop, device.as_deref()) {
                log::error!("WASAPI loopback: {e}");
            }
        })
        .expect("spawn loopback")
}

#[cfg(not(windows))]
pub fn start_loopback_thread(
    _buf: Arc<LoopbackBuf>,
    _stop: Arc<AtomicBool>,
    _device: Option<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new().spawn(|| {}).unwrap()
}

#[cfg(windows)]
fn run_loopback(
    buf: &Arc<LoopbackBuf>, stop: &Arc<AtomicBool>, device: Option<&str>,
) -> Result<(), String> {
    unsafe { ffi::run_loopback(buf, stop, device) }
}

// ════════════════════════════════════════════════════════════
// Windows FFI 实现
// ════════════════════════════════════════════════════════════
#[cfg(windows)]
mod ffi {
    use super::*;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq)]
    struct Guid {
        d1: u32,
        d2: u16,
        d3: u16,
        d4: [u8; 8],
    }

    const CLSID_MM_ENUMERATOR: Guid = Guid {
        d1: 0xBCDE039, d2: 0xE52F, d3: 0x4670,
        d4: [0x8F, 0xDB, 0xCD, 0x73, 0x74, 0x40, 0x53, 0x78],
    };
    const IID_ENUMERATOR: Guid = Guid {
        d1: 0xA95664D2, d2: 0x96B4, d3: 0x4F02,
        d4: [0xB1, 0x06, 0x57, 0x74, 0xD0, 0x9E, 0x09, 0x27],
    };
    const IID_AUDIO_CLIENT: Guid = Guid {
        d1: 0x1CB9AD4C, d2: 0xDBFA, d3: 0x4C32,
        d4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
    };
    const IID_CAPTURE: Guid = Guid {
        d1: 0xC8ADBD64, d2: 0xE3E2, d3: 0x428A,
        d4: [0xA6, 0x1B, 0x2A, 0xEA, 0x69, 0x44, 0x6E, 0x61],
    };
    const SUBTYPE_FLOAT: Guid = Guid {
        d1: 0x00000003, d2: 0x0000, d3: 0x0010,
        d4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
    };
    /// PKEY_Device_FriendlyName 的 fmtid（pid = 14）。设备的显示名，
    /// 和 cpal 列出来的输出设备名是同一个来源，所以能直接按名字对上。
    const FMTID_DEVICE: Guid = Guid {
        d1: 0xA45C254E, d2: 0xDF1C, d3: 0x4EFD,
        d4: [0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0],
    };

    #[repr(C)]
    struct PropertyKey {
        fmtid: Guid,
        pid: u32,
    }
    const PKEY_DEVICE_FRIENDLY_NAME: PropertyKey =
        PropertyKey { fmtid: FMTID_DEVICE, pid: 14 };

    /// PROPVARIANT。只用到 vt 和 8 字节偏移处的联合体（VT_LPWSTR 时是宽字符串指针）。
    #[repr(C)]
    struct PropVariant {
        vt: u16,
        _r1: u16,
        _r2: u16,
        _r3: u16,
        val: *mut u16,
        _pad: usize,
    }
    const VT_LPWSTR: u16 = 31;

    #[repr(C)]
    struct WaveFormatEx {
        format_tag: u16,
        n_channels: u16,
        samples_per_sec: u32,
        avg_bytes_per_sec: u32,
        block_align: u16,
        bits_per_sample: u16,
        cb_size: u16,
    }

    extern "system" {
        fn CoInitializeEx(reserved: *const c_void, co_init: u32) -> i32;
        fn CoCreateInstance(
            clsid: *const Guid, outer: *const c_void, ctx: u32,
            iid: *const Guid, ppv: *mut *mut c_void,
        ) -> i32;
        fn CoTaskMemFree(ptr: *const c_void);
        fn CoUninitialize();
        fn PropVariantClear(pvar: *mut PropVariant) -> i32;
    }

    // COM vtable structs — 字段顺序必须与 C 头文件一致。
    #[repr(C)]
    struct EnumVtbl {
        _qi: unsafe extern "system" fn(),
        _ar: unsafe extern "system" fn(),
        _rl: unsafe extern "system" fn(),
        enum_endpoints: unsafe extern "system" fn(
            *mut c_void, u32, u32, *mut *mut c_void,
        ) -> i32,
        get_default: unsafe extern "system" fn(
            *mut c_void, u32, u32, *mut *mut c_void,
        ) -> i32,
        _gd: unsafe extern "system" fn(),
        _rg: unsafe extern "system" fn(),
        _ur: unsafe extern "system" fn(),
    }

    #[repr(C)]
    struct CollectionVtbl {
        _qi: unsafe extern "system" fn(),
        _ar: unsafe extern "system" fn(),
        _rl: unsafe extern "system" fn(),
        get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
        item: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32,
    }

    #[repr(C)]
    struct PropStoreVtbl {
        _qi: unsafe extern "system" fn(),
        _ar: unsafe extern "system" fn(),
        _rl: unsafe extern "system" fn(),
        _gc: unsafe extern "system" fn(),
        _ga: unsafe extern "system" fn(),
        get_value: unsafe extern "system" fn(
            *mut c_void, *const PropertyKey, *mut PropVariant,
        ) -> i32,
        _sv: unsafe extern "system" fn(),
        _cm: unsafe extern "system" fn(),
    }

    #[repr(C)]
    struct DeviceVtbl {
        _qi: unsafe extern "system" fn(),
        _ar: unsafe extern "system" fn(),
        _rl: unsafe extern "system" fn(),
        activate: unsafe extern "system" fn(
            *mut c_void, *const Guid, u32,
            *const c_void, *mut *mut c_void,
        ) -> i32,
        open_prop_store: unsafe extern "system" fn(
            *mut c_void, u32, *mut *mut c_void,
        ) -> i32,
        _gi: unsafe extern "system" fn(),
        _gs: unsafe extern "system" fn(),
    }

    #[repr(C)]
    struct ClientVtbl {
        _qi: unsafe extern "system" fn(),
        _ar: unsafe extern "system" fn(),
        _rl: unsafe extern "system" fn(),
        initialize: unsafe extern "system" fn(
            *mut c_void, u32, u32, i64, i64,
            *const WaveFormatEx, *const Guid,
        ) -> i32,
        _gbs: unsafe extern "system" fn(),
        _gsl: unsafe extern "system" fn(),
        _gcp: unsafe extern "system" fn(),
        _ifs: unsafe extern "system" fn(),
        get_mix_format: unsafe extern "system" fn(
            *mut c_void, *mut *mut WaveFormatEx,
        ) -> i32,
        _gdp: unsafe extern "system" fn(),
        start: unsafe extern "system" fn(*mut c_void) -> i32,
        stop: unsafe extern "system" fn(*mut c_void) -> i32,
        _rs: unsafe extern "system" fn(),
        _se: unsafe extern "system" fn(),
        get_service: unsafe extern "system" fn(
            *mut c_void, *const Guid, *mut *mut c_void,
        ) -> i32,
    }

    #[repr(C)]
    struct CaptureVtbl {
        _qi: unsafe extern "system" fn(),
        _ar: unsafe extern "system" fn(),
        _rl: unsafe extern "system" fn(),
        get_buffer: unsafe extern "system" fn(
            *mut c_void, *mut *mut u8,
            *mut u32, *mut u32, *mut u64, *mut u64,
        ) -> i32,
        release_buffer: unsafe extern "system" fn(*mut c_void, u32) -> i32,
        get_next_packet: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    }

    const AUDCLNT_SHAREMODE_SHARED: u32 = 0;
    const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x0002_0000;
    const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;
    const CLSCTX_ALL: u32 = 0x17;
    const COINIT_MULTITHREADED: u32 = 0;
    const E_RENDER: u32 = 0;
    const E_CONSOLE: u32 = 0;
    const DEVICE_STATE_ACTIVE: u32 = 0x1;
    const STGM_READ: u32 = 0;
    const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
    const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

    unsafe fn vtbl<'a, T>(obj: *mut c_void) -> &'a T {
        &*(*(obj as *const *const T))
    }

    unsafe fn release(obj: *mut c_void) {
        if obj.is_null() { return; }
        // obj -> vtable -> 第 3 个槽位（QueryInterface / AddRef / Release）里存的函数指针。
        // 注意要把槽位里的值读出来，而不是拿槽位自己的地址。
        let vptr = *(obj as *const *const usize);
        let slot = *vptr.add(2);
        let release_fn: unsafe extern "system" fn(*mut c_void) -> u32 =
            std::mem::transmute(slot);
        release_fn(obj);
    }

    unsafe fn is_extensible_float(wfx: *const WaveFormatEx) -> bool {
        // SubFormat GUID at byte offset 24 within WAVEFORMATEX
        let gp = (wfx as *const u8).add(24) as *const Guid;
        *gp == SUBTYPE_FLOAT
    }

    unsafe fn read_sample(data: &[u8], bytes: usize, is_float: bool) -> f32 {
        match bytes {
            4 if is_float => f32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            2 => i16::from_le_bytes([data[0], data[1]]) as f32 / i16::MAX as f32,
            4 => i32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f32 / i32::MAX as f32,
            _ => 0.0,
        }
    }

    unsafe fn push_packet(
        data: &[u8], num_frames: usize, channels: u16,
        bits: u16, is_float: bool, buf: &Arc<LoopbackBuf>,
    ) {
        let bps = (bits / 8) as usize;
        let ch = channels as usize;
        let fb = bps * ch;
        let Ok(mut b) = buf.samples.lock() else { return };
        for frame in 0..num_frames {
            let off = frame * fb;
            if off + fb > data.len() { break; }
            let (l, r) = if ch >= 2 {
                (read_sample(&data[off..], bps, is_float),
                 read_sample(&data[off + bps..], bps, is_float))
            } else {
                let s = read_sample(&data[off..], bps, is_float);
                (s, s)
            };
            b.push_back(l);
            b.push_back(r);
        }
        while b.len() > LoopbackBuf::CAP { b.pop_front(); }
        drop(b);
        if buf.mon_enabled.load(Ordering::Relaxed) {
            if let Ok(mut m) = buf.mon.lock() {
                for frame in 0..num_frames {
                    let off = frame * fb;
                    if off + fb > data.len() { break; }
                    let (l, r) = if ch >= 2 {
                        (read_sample(&data[off..], bps, is_float),
                         read_sample(&data[off + bps..], bps, is_float))
                    } else {
                        let s = read_sample(&data[off..], bps, is_float);
                        (s, s)
                    };
                    m.push_back(l);
                    m.push_back(r);
                }
                while m.len() > LoopbackBuf::CAP { m.pop_front(); }
            }
        }
    }

    /// 读设备的显示名。失败返回 None。
    unsafe fn friendly_name(device: *mut c_void) -> Option<String> {
        let mut store: *mut c_void = std::ptr::null_mut();
        if (vtbl::<DeviceVtbl>(device).open_prop_store)(device, STGM_READ, &mut store) < 0 {
            return None;
        }
        let mut pv = PropVariant { vt: 0, _r1: 0, _r2: 0, _r3: 0, val: std::ptr::null_mut(), _pad: 0 };
        let hr = (vtbl::<PropStoreVtbl>(store).get_value)(
            store, &PKEY_DEVICE_FRIENDLY_NAME, &mut pv,
        );
        let name = if hr >= 0 && pv.vt == VT_LPWSTR && !pv.val.is_null() {
            let mut len = 0usize;
            while *pv.val.add(len) != 0 { len += 1; }
            Some(String::from_utf16_lossy(std::slice::from_raw_parts(pv.val, len)))
        } else {
            None
        };
        PropVariantClear(&mut pv);
        release(store);
        name
    }

    /// 按名字打开一个播放设备用于环回捕获；`want` 为 None 或找不到时退回系统默认。
    ///
    /// 找不到时**故意退回默认而不是报错**：设备被拔掉/改名之后，
    /// 至少还能捕到点东西，不会整条功能静默失效。
    unsafe fn open_render_device(
        enum_obj: *mut c_void, want: Option<&str>,
    ) -> Result<*mut c_void, i32> {
        if let Some(want) = want.map(str::trim).filter(|w| !w.is_empty()) {
            let mut coll: *mut c_void = std::ptr::null_mut();
            let hr = (vtbl::<EnumVtbl>(enum_obj).enum_endpoints)(
                enum_obj, E_RENDER, DEVICE_STATE_ACTIVE, &mut coll,
            );
            if hr >= 0 && !coll.is_null() {
                let mut count: u32 = 0;
                if (vtbl::<CollectionVtbl>(coll).get_count)(coll, &mut count) >= 0 {
                    for i in 0..count {
                        let mut dev: *mut c_void = std::ptr::null_mut();
                        if (vtbl::<CollectionVtbl>(coll).item)(coll, i, &mut dev) < 0 || dev.is_null() {
                            continue;
                        }
                        // cpal 列出的名字可能被截断（老 Windows 上限 31 字符），两边互相包含就算命中。
                        let hit = friendly_name(dev)
                            .map(|n| n == want || n.starts_with(want) || want.starts_with(&n))
                            .unwrap_or(false);
                        if hit {
                            release(coll);
                            log::info!("环回捕获设备：{want}");
                            return Ok(dev);
                        }
                        release(dev);
                    }
                }
                release(coll);
            }
            log::warn!("找不到播放设备「{want}」，环回捕获退回系统默认设备");
        }
        let mut device: *mut c_void = std::ptr::null_mut();
        let hr = (vtbl::<EnumVtbl>(enum_obj)
            .get_default)(enum_obj, E_RENDER, E_CONSOLE, &mut device);
        if hr < 0 { Err(hr) } else { Ok(device) }
    }

    pub(super) unsafe fn run_loopback(
        buf: &Arc<LoopbackBuf>, stop: &Arc<AtomicBool>, want: Option<&str>,
    ) -> Result<(), String> {
        let hr = CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED);
        if hr < 0 { return Err(format!("CoInitializeEx 0x{hr:08X}")); }

        let mut enum_obj: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_MM_ENUMERATOR, std::ptr::null(), CLSCTX_ALL,
            &IID_ENUMERATOR, &mut enum_obj,
        );
        if hr < 0 { CoUninitialize(); return Err(format!("CoCreateInstance 0x{hr:08X}")); }

        let device = match open_render_device(enum_obj, want) {
            Ok(d) => d,
            Err(hr) => {
                release(enum_obj);
                CoUninitialize();
                return Err(format!("GetDefault 0x{hr:08X}"));
            }
        };

        let mut client: *mut c_void = std::ptr::null_mut();
        let hr = (vtbl::<DeviceVtbl>(device)
            .activate)(device, &IID_AUDIO_CLIENT, CLSCTX_ALL, std::ptr::null(), &mut client);
        if hr < 0 { release(device); release(enum_obj); CoUninitialize(); return Err(format!("Activate 0x{hr:08X}")); }

        let mut mix: *mut WaveFormatEx = std::ptr::null_mut();
        let hr = (vtbl::<ClientVtbl>(client).get_mix_format)(client, &mut mix);
        if hr < 0 || mix.is_null() {
            release(client); release(device); release(enum_obj); CoUninitialize();
            return Err(format!("GetMixFormat 0x{hr:08X}"));
        }

        let channels = (*mix).n_channels;
        let sample_rate = (*mix).samples_per_sec;
        let bits = (*mix).bits_per_sample;
        let tag = (*mix).format_tag;
        let is_float = tag == WAVE_FORMAT_IEEE_FLOAT
            || (tag == WAVE_FORMAT_EXTENSIBLE
                && (*mix).cb_size >= 22
                && is_extensible_float(mix));

        let _ = buf.sample_rate.set(sample_rate);

        // 共享模式下 pFormat 不能为 NULL（否则 E_POINTER）；环回捕获必须用设备的混音格式。
        let hr = (vtbl::<ClientVtbl>(client).initialize)(
            client, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
            0, 0, mix, std::ptr::null(),
        );
        if hr < 0 {
            CoTaskMemFree(mix as *const c_void);
            release(client); release(device); release(enum_obj); CoUninitialize();
            return Err(format!("Initialize 0x{hr:08X}"));
        }

        let mut capture: *mut c_void = std::ptr::null_mut();
        let hr = (vtbl::<ClientVtbl>(client).get_service)(client, &IID_CAPTURE, &mut capture);
        if hr < 0 {
            CoTaskMemFree(mix as *const c_void);
            release(client); release(device); release(enum_obj); CoUninitialize();
            return Err(format!("GetService 0x{hr:08X}"));
        }

        let hr = (vtbl::<ClientVtbl>(client).start)(client);
        if hr < 0 {
            CoTaskMemFree(mix as *const c_void);
            release(capture); release(client); release(device); release(enum_obj); CoUninitialize();
            return Err(format!("Start 0x{hr:08X}"));
        }

        while !stop.load(Ordering::Relaxed) {
            let mut packet: u32 = 0;
            if (vtbl::<CaptureVtbl>(capture).get_next_packet)(capture, &mut packet) < 0 { break; }
            while packet > 0 {
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                let hr = (vtbl::<CaptureVtbl>(capture).get_buffer)(
                    capture, &mut data, &mut frames, &mut flags,
                    std::ptr::null_mut(), std::ptr::null_mut(),
                );
                if hr < 0 { break; }
                if (flags & AUDCLNT_BUFFERFLAGS_SILENT) == 0 && !data.is_null() && frames > 0 {
                    let fb = (bits as usize / 8) * (channels as usize);
                    let total = frames as usize * fb;
                    push_packet(
                        std::slice::from_raw_parts(data, total),
                        frames as usize, channels, bits, is_float, buf,
                    );
                }
                (vtbl::<CaptureVtbl>(capture).release_buffer)(capture, frames);
                let _ = (vtbl::<CaptureVtbl>(capture).get_next_packet)(capture, &mut packet);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        (vtbl::<ClientVtbl>(client).stop)(client);
        CoTaskMemFree(mix as *const c_void);
        release(capture);
        release(client);
        release(device);
        release(enum_obj);
        CoUninitialize();
        Ok(())
    }
}
