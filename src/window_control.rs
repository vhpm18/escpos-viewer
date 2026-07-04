#[cfg(target_os = "windows")]
mod imp {
    use std::sync::{atomic::{AtomicIsize, Ordering}, Arc};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetWindowLongPtrW, SetForegroundWindow,
        SetWindowLongPtrW, SetWindowPos, ShowWindow,
        GWL_EXSTYLE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_RESTORE,
        SW_SHOW, SWP_SHOWWINDOW, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, HWND_NOTOPMOST, HWND_TOPMOST,
    };

    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

    #[derive(Clone, Default)]
    pub struct WindowControl {
        hwnd: Arc<AtomicIsize>,
    }

    impl WindowControl {
        pub fn set_ctx(&mut self, _ctx: &eframe::egui::Context) {
            // No-op: Windows usa HWND, no necesita egui::Context.
        }

        pub fn try_update_from_frame(&self, frame: &mut eframe::Frame) {
            let Ok(window_handle) = frame.window_handle() else {
                return;
            };
            let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
                return;
            };
            let hwnd = handle.hwnd.get() as isize;
            if hwnd != 0 {
                self.hwnd.store(hwnd, Ordering::Relaxed);
            }
        }

        fn hwnd_ptr(&self) -> *mut core::ffi::c_void {
            let hwnd = self.hwnd.load(Ordering::Relaxed);
            hwnd as *mut core::ffi::c_void
        }

        fn set_taskbar_visible(&self, visible: bool) {
            let hwnd = self.hwnd_ptr();
            if hwnd.is_null() {
                return;
            }

            unsafe {
                let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as isize;
                if visible {
                    ex &= !(WS_EX_TOOLWINDOW as isize);
                    ex |= WS_EX_APPWINDOW as isize;
                } else {
                    ex |= WS_EX_TOOLWINDOW as isize;
                    ex &= !(WS_EX_APPWINDOW as isize);
                }
                let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
                let _ = SetWindowPos(
                    hwnd,
                    core::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            }
        }

        pub fn hide_to_tray(&self) {
            let hwnd = self.hwnd_ptr();
            if hwnd.is_null() {
                return;
            }
            self.set_taskbar_visible(false);
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }

        pub fn show_and_focus(&self) {
            let hwnd = self.hwnd_ptr();
            if hwnd.is_null() {
                return;
            }
            self.set_taskbar_visible(true);
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = ShowWindow(hwnd, SW_RESTORE);

                // Truco común en Windows para forzar que suba al frente:
                // poner TOPMOST y luego volver a NOTOPMOST (no queda siempre encima).
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
        }

        pub fn snap_near_right(&self, margin_px: i32) {
            let hwnd = self.hwnd_ptr();
            if hwnd.is_null() {
                return;
            }

            unsafe {
                let mut rect: RECT = core::mem::zeroed();
                if GetWindowRect(hwnd, &mut rect) == 0 {
                    return;
                }

                let w = (rect.right - rect.left).max(1);
                let h = (rect.bottom - rect.top).max(1);

                let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                if monitor.is_null() {
                    return;
                }

                let mut mi: MONITORINFO = core::mem::zeroed();
                mi.cbSize = core::mem::size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(monitor, &mut mi) == 0 {
                    return;
                }

                // Usar work area (sin taskbar) y alinear abajo a la derecha.
                let work = mi.rcWork;
                let margin = margin_px.max(0);
                let mut x = work.right - w - margin;
                let mut y = work.bottom - h - margin;

                // Clamp básico para no salir del work area.
                x = x.max(work.left + margin).min(work.right - w - margin);
                y = y.max(work.top + margin).min(work.bottom - h - margin);

                let _ = SetWindowPos(
                    hwnd,
                    core::ptr::null_mut(),
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
                );
            }
        }

        pub fn center_on_screen(&self) {
            let hwnd = self.hwnd_ptr();
            if hwnd.is_null() {
                return;
            }

            unsafe {
                let mut rect: RECT = core::mem::zeroed();
                if GetWindowRect(hwnd, &mut rect) == 0 {
                    return;
                }

                let w = (rect.right - rect.left).max(1);
                let h = (rect.bottom - rect.top).max(1);

                let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                if monitor.is_null() {
                    return;
                }

                let mut mi: MONITORINFO = core::mem::zeroed();
                mi.cbSize = core::mem::size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(monitor, &mut mi) == 0 {
                    return;
                }

                let work = mi.rcWork;
                let work_w = (work.right - work.left).max(1);
                let work_h = (work.bottom - work.top).max(1);

                let x = work.left + ((work_w - w) / 2);
                let y = work.top + ((work_h - h) / 2);

                let _ = SetWindowPos(
                    hwnd,
                    core::ptr::null_mut(),
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
                );
            }
        }
    }

    pub use WindowControl as WindowControlExport;
}

#[cfg(not(target_os = "windows"))]
mod imp {
    #[derive(Clone, Default)]
    pub struct WindowControl {
        ctx: Option<eframe::egui::Context>,
    }

    impl WindowControl {
        /// Store the egui::Context so we can send viewport commands from any
        /// thread (tray listener, etc.). Clone is cheap (Context is internally
        /// Arc'd). Called from update() where &Context is available.
        pub fn set_ctx(&mut self, ctx: &eframe::egui::Context) {
            self.ctx = Some(ctx.clone());
        }

        pub fn try_update_from_frame(&mut self, frame: &mut eframe::Frame) {
            // En eframe 0.29, Frame::ctx() no está disponible como método público.
            // Usamos set_ctx() desde update() donde ctx se pasa directamente.
            let _ = frame;
        }

        pub fn hide_to_tray(&self) {
            if let Some(ref ctx) = self.ctx {
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
            }
        }

        pub fn show_and_focus(&self) {
            if let Some(ref ctx) = self.ctx {
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(false));
                ctx.request_repaint();
            }
        }

        pub fn snap_near_right(&self, margin_px: i32) {
            if let Some(ref ctx) = self.ctx {
                let margin = margin_px.max(0) as f32;

                let (win_w, win_h, monitor_w) = ctx.input(|i| {
                    let vp = i.viewport();
                    // Prefer outer_rect size para OuterPosition (incluye decoraciones)
                    let size = vp.outer_rect.or(vp.inner_rect).map(|r| r.size());
                    let mw = vp.monitor_size.map(|m| m.x);
                    (size.map(|s| s.x), size.map(|s| s.y), mw)
                });

                if let (Some(win_w), Some(_)) = (win_w, win_h) {
                    let screen_w = monitor_w.unwrap_or_else(|| ctx.screen_rect().width());
                    let x = screen_w - win_w - margin;
                    let y = margin;
                    ctx.send_viewport_cmd(
                        eframe::egui::ViewportCommand::OuterPosition(eframe::egui::pos2(x, y)),
                    );
                }
            }
        }

        pub fn center_on_screen(&self) {
            if let Some(ref ctx) = self.ctx {
                if let Some(cmd) = eframe::egui::ViewportCommand::center_on_screen(ctx) {
                    ctx.send_viewport_cmd(cmd);
                } else {
                    // Fallback para compositores que no reportan monitor_size
                    // (ej. algunos Wayland). Usamos screen_rect() que siempre está disponible.
                    let screen = ctx.screen_rect();
                    let win_size = ctx.input(|i| i.viewport().outer_rect.or(i.viewport().inner_rect).map(|r| r.size()));
                    if let Some(win_size) = win_size {
                        let x = (screen.width() - win_size.x) / 2.0;
                        let y = (screen.height() - win_size.y) / 2.0;
                        ctx.send_viewport_cmd(
                            eframe::egui::ViewportCommand::OuterPosition(eframe::egui::pos2(x, y)),
                        );
                    }
                }
            }
        }
    }

    pub use WindowControl as WindowControlExport;
}

pub use imp::WindowControlExport as WindowControl;
