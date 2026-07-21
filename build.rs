fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon_with_id("assets/app.ico", "32512");
        res.set("InternalName", "codexplus.exe");
        res.set("FileDescription", "CodexPlus — Codex usage overlay");
        res.set("ProductName", "CodexPlus");
        res.set("LegalCopyright", "MIT License");
        if let Err(err) = res.compile() {
            eprintln!("warning: failed to attach Windows resource: {err}");
        }
    }
}
