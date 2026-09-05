//! 平台相关的小工具：开机自启、打开文件夹 / 网页。
//! 非 Windows 平台提供空实现，保证跨平台可编译（本机 Mac 上开发时能过编译）。

/// 起子进程时不要弹控制台窗口（否则会闪一下 cmd 黑窗）。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// VB-CABLE 官方下载页。
pub const VBCABLE_URL: &str = "https://vb-audio.com/Cable/";

/// 设置 / 取消开机自启。
#[cfg(windows)]
pub fn set_autostart(enable: bool) -> anyhow::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegDeleteValueW, RegSetValueExW,
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
    };

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    const NAME: &str = "VoicePlayer";
    let subkey = to_wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = to_wide(NAME);

    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        );
        if status != 0 {
            anyhow::bail!("打开注册表项失败，错误码 {status}");
        }
        if enable {
            let exe = std::env::current_exe()?;
            let val = format!("\"{}\"", exe.display());
            let val_wide = to_wide(&val);
            let status = RegSetValueExW(
                hkey,
                name.as_ptr(),
                0,
                1, // REG_SZ
                val_wide.as_ptr() as *const u8,
                (val_wide.len() * 2) as u32,
            );
            RegCloseKey(hkey);
            if status != 0 {
                anyhow::bail!("写入注册表失败，错误码 {status}");
            }
        } else {
            let _ = RegDeleteValueW(hkey, name.as_ptr());
            RegCloseKey(hkey);
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set_autostart(_enable: bool) -> anyhow::Result<()> {
    Ok(())
}

/// 在系统文件管理器里打开一个文件夹。
pub fn open_folder(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

/// 用默认浏览器打开一个网址。
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 用 cmd 的 start；空标题参数 "" 不能省。
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Open Windows "App volume and device preferences" settings page.
/// Users can assign per-app output devices here (e.g. route music player to CABLE Input).
#[cfg(windows)]
pub fn open_app_volume_settings() {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", "ms-settings:apps-volume"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(not(windows))]
pub fn open_app_volume_settings() {}
