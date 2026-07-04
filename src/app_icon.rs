use std::io::Cursor;

/// Carga `icon.ico` embebido y devuelve el mejor frame (el más grande) como RGBA.
fn load_best_ico_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let bytes = include_bytes!("../icon.ico");
    let dir = ico::IconDir::read(Cursor::new(bytes)).ok()?;

    let entry = dir
        .entries()
        .iter()
        .max_by_key(|e| (e.width() as u32) * (e.height() as u32))?;

    let img = entry.decode().ok()?;
    let w = img.width() as u32;
    let h = img.height() as u32;
    let rgba = img.rgba_data().to_vec();

    Some((rgba, w, h))
}

pub fn eframe_icon_data() -> Option<eframe::egui::IconData> {
    let (rgba, w, h) = load_best_ico_rgba()?;
    Some(eframe::egui::IconData {
        rgba,
        width: w,
        height: h,
    })
}

pub fn tray_icon() -> Option<tray_icon::Icon> {
    let (rgba, w, h) = load_best_ico_rgba()?;
    tray_icon::Icon::from_rgba(rgba, w, h).ok()
}
