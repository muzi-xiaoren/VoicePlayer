//! 平台相关的小工具：开机自启、打开文件夹 / 网页。
//! 非 Windows 平台提供空实现，保证跨平台可编译（本机 Mac 上开发时能过编译）。

/// 起子进程时不要弹控制台窗口（否则会闪一下 cmd 黑窗）。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// VB-CABLE 官方下载页。
pub const VBCABLE_URL: &str = "https://vb-audio.com/Cable/";

/// 计划任务 / 注册表 Run 项里用的名字。
#[cfg(windows)]
const AUTOSTART_NAME: &str = "VoicePlayer";

/// 设置 / 取消开机自启。
///
/// 优先建一个「登录时以最高权限运行」的计划任务：游戏多半是管理员权限跑的，
/// 本程序不提权就收不到它在前台时的按键，而注册表 Run 项拉起来的进程是不提权的 ——
/// 自启起来的那一次热键在游戏里就是哑的。计划任务可以免 UAC 弹窗直接提权启动。
///
/// 建任务本身需要管理员权限。当前没提权时建不了，就退回 Run 项（至少能自启）；
/// 下次以管理员身份运行时会自动换成计划任务。两种方式不会同时留着，否则会起两个进程。
#[cfg(windows)]
pub fn set_autostart(enable: bool) -> anyhow::Result<()> {
    if enable {
        let exe = std::env::current_exe()?;
        if create_logon_task(&exe) {
            return set_run_key(None);
        }
        if logon_task_exists() {
            // 没权限改它，但以前建的任务还在，照样会自启；别再加一个 Run 项。
            return Ok(());
        }
        set_run_key(Some(&exe))
    } else {
        let had_task = logon_task_exists();
        let deleted = schtasks(&["/Delete", "/TN", AUTOSTART_NAME, "/F"]);
        set_run_key(None)?;
        if had_task && !deleted {
            anyhow::bail!("删除开机自启的计划任务失败（需要以管理员身份运行本程序再取消一次）");
        }
        Ok(())
    }
}

/// 跑一条 schtasks 命令，返回是否成功。不弹控制台窗口。
#[cfg(windows)]
fn schtasks(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    std::process::Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn logon_task_exists() -> bool {
    schtasks(&["/Query", "/TN", AUTOSTART_NAME])
}

/// 建（或覆盖）登录时提权启动的计划任务。没有管理员权限时会失败，返回 false。
///
/// 用 XML 而不是 `schtasks /SC ONLOGON` 的命令行参数，是因为命令行建出来的任务带两个
/// 改不掉的默认值：用电池时不启动（笔记本上等于没自启）、运行 72 小时后强制结束。
#[cfg(windows)]
fn create_logon_task(exe: &std::path::Path) -> bool {
    let (Ok(domain), Ok(user)) = (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) else {
        return false;
    };
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let user = esc(&format!("{domain}\\{user}"));
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
    </Exec>
  </Actions>
</Task>
"#,
        exe = esc(&exe.display().to_string()),
    );
    // schtasks 要求 XML 文件是带 BOM 的 UTF-16。
    let bytes: Vec<u8> = std::iter::once(0xFEFFu16)
        .chain(xml.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    let file = std::env::temp_dir().join("voiceplayer-autostart.xml");
    if std::fs::write(&file, bytes).is_err() {
        return false;
    }
    let ok = schtasks(&["/Create", "/TN", AUTOSTART_NAME, "/XML", &file.display().to_string(), "/F"]);
    let _ = std::fs::remove_file(&file);
    ok
}

/// 写入（Some）或删除（None）注册表 Run 项。
#[cfg(windows)]
fn set_run_key(exe: Option<&std::path::Path>) -> anyhow::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegDeleteValueW, RegSetValueExW,
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
    };

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let subkey = to_wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = to_wide(AUTOSTART_NAME);

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
        if let Some(exe) = exe {
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
