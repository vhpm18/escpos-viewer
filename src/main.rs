#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod app_icon;
mod escpos;
mod hex_dump;
mod model;
mod printer_setup;
mod tcp_capture;
mod tray;
mod window_control;

use eframe::egui;

#[cfg(target_os = "windows")]
fn try_focus_existing_instance_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, FindWindowW, SetForegroundWindow, SetWindowPos, ShowWindow,
        HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW,
    };

    fn wide_null_terminated(s: &str) -> Vec<u16> {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.push(0);
        v
    }

    // Intentar por títulos conocidos.
    for title in ["Visor ESC-POS", "Visor ESC/POS"] {
        let title_w = wide_null_terminated(title);
        let hwnd = unsafe { FindWindowW(core::ptr::null(), title_w.as_ptr()) };
        if hwnd.is_null() {
            continue;
        }

        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = ShowWindow(hwnd, SW_RESTORE);

            // TOPMOST -> NOTOPMOST para forzar foco.
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            let _ = SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );

            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }
        break;
    }
}

fn main() -> eframe::Result<()> {
    // Modo instalador/CLI (Windows): permite que un instalador cree la impresora virtual.
    // Requiere ejecutar como Administrador.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--install-printer") {
        match printer_setup::install_printer() {
            Ok(()) => {
                println!("OK: impresora instalada");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|a| a == "--uninstall-printer") {
        match printer_setup::uninstall_printer() {
            Ok(()) => {
                println!("OK: impresora desinstalada");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(1);
            }
        }
    }

    // Single instance: evita que una segunda instancia intente abrir el puerto 9100.
    let instance = single_instance::SingleInstance::new("visor-escpos-viewer")
        .expect("single-instance init failed");
    if !instance.is_single() {
        #[cfg(target_os = "windows")]
        {
            try_focus_existing_instance_window();
        }
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([480.0, 600.0])
            .with_title("Visor ESC-POS")
            .with_icon(app_icon::eframe_icon_data().unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native(
        "Visor ESC/POS",
        options,
        Box::new(|cc| {
            // Registrar fuente de impresora térmica (DotFont - estilo dot matrix)
            let mut fonts = egui::FontDefinitions::default();

            // Cargar fuente DotFont personalizada
            fonts.font_data.insert(
                "dotfont".to_owned(),
                egui::FontData::from_static(include_bytes!("../assets/fonts/dotfont.ttf")),
            );

            // Registrar como familia "DotMatrix"
            fonts.families.insert(
                egui::FontFamily::Name("DotMatrix".into()),
                vec!["dotfont".to_owned()],
            );

            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(app::EscPosViewer::default()))
        }),
    )
}
