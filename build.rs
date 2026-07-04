fn main() {
    // Embebe icon.ico como recurso del .exe en Windows (Explorer / Taskbar / Alt-Tab).
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
