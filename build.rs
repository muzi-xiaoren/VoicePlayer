//! 把 Windows 可执行文件图标嵌入 exe（仅 release 构建）。
//! 图标来自 assets/icon.ico。

fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // 这个会写入 exe 的版本信息里，不影响功能。
        res.set("FileDescription", "VoicePlayer");
        if let Err(e) = res.compile() {
            println!("cargo::warning=cargo-winres failed: {e}");
        }
    }
}
