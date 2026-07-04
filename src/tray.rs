use std::thread;

use crate::app_icon;
use crate::window_control::WindowControl;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

pub struct SystemTray {
    _tray: TrayIcon,
    _menu: Menu,
    _open: MenuItem,
    _exit: MenuItem,
}

impl SystemTray {
    pub fn new(window: WindowControl) -> Result<Self, String> {
        // En Linux, tray-icon usa GTK internamente y debe estar inicializado.
        // Si eframe ya lo inició, init() falla silenciosamente.
        #[cfg(all(unix, not(target_os = "macos")))]
        let _ = gtk::init();

        let icon: Icon = app_icon::tray_icon().unwrap_or_else(|| {
            // Fallback: icono mínimo 16x16 RGBA (gris con borde negro).
            let (w, h) = (16u32, 16u32);
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    let border = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                    let (r, g, b) = if border { (0, 0, 0) } else { (210, 210, 210) };
                    rgba[i] = r;
                    rgba[i + 1] = g;
                    rgba[i + 2] = b;
                    rgba[i + 3] = 255;
                }
            }

            Icon::from_rgba(rgba, w, h).expect("fallback tray icon")
        });

        let menu = Menu::new();
        let open = MenuItem::new("Abrir Visor", true, None);
        let exit = MenuItem::new("Salir", true, None);
        menu.append(&open).map_err(|e| format!("menu open: {e:?}"))?;
        menu.append(&exit).map_err(|e| format!("menu exit: {e:?}"))?;

        let open_id_thread = open.id().clone();
        let exit_id_thread = exit.id().clone();

        // Hilo: recibe clicks del menú y abre/cierra la ventana.
        thread::spawn(move || {
            let ev_rx = MenuEvent::receiver();
            while let Ok(ev) = ev_rx.recv() {
                if ev.id == open_id_thread {
                    window.show_and_focus();
                } else if ev.id == exit_id_thread {
                    std::process::exit(0);
                }
            }
        });

        // El builder se queda con el menú; guardamos clones para mantener vivos los items.
        let tray = TrayIconBuilder::new()
            .with_tooltip("Visor ESC-POS")
            .with_icon(icon)
            .with_menu(Box::new(menu.clone()))
            .build()
            .map_err(|e| format!("tray build: {e:?}"))?;

        Ok(Self {
            _tray: tray,
            _menu: menu,
            _open: open,
            _exit: exit,
        })
    }
}

#[cfg(test)]
mod tests {
    use tray_icon::Icon;

    #[test]
    fn fallback_tray_icon_is_valid_rgba() {
        // Reproduce the fallback icon construction from SystemTray::new()
        let (w, h) = (16u32, 16u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let border = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                let (r, g, b) = if border { (0, 0, 0) } else { (210, 210, 210) };
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 255;
            }
        }

        let icon = Icon::from_rgba(rgba, w, h);
        assert!(icon.is_ok(), "fallback icon should be valid");
    }
}
