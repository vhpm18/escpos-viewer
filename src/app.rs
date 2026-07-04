use crate::escpos::parse_escpos;
use crate::hex_dump::pretty_hex;
use crate::model::{
    Align, BarcodeHriPosition, CodePage, CommandType, Control, PaperWidth, PrinterState,
};
use crate::tcp_capture::TcpCapture;
use crate::tray::SystemTray;
use crate::window_control::WindowControl;
use eframe::egui;
use qrcode::types::Color;
use qrcode::{EcLevel, QrCode};
use rfd::FileDialog;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::mem;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiMode {
    Preview,
    Full,
}

#[derive(Debug, Clone)]
struct JobEntry {
    id: u64,
    label: String,
    created_at: Instant,

    full_bytes: Vec<u8>,
    display_bytes: Vec<u8>,
    parsed_commands: Vec<(PrinterState, CommandType)>,

    sim_active: bool,
    sim_started_at: Option<Instant>,
    sim_sent: usize,
}

pub struct EscPosViewer {
    jobs: Vec<JobEntry>,
    active_job_idx: Option<usize>,
    next_job_id: u64,

    max_jobs: usize,
    auto_prune_by_age: bool,
    prune_after: Duration,
    auto_scroll_on_print: bool,
    paper_width: PaperWidth,
    last_paper_width: PaperWidth,
    did_apply_initial_window_size: bool,
    did_apply_initial_window_position: bool,
    show_debug_controls: bool,
    show_debug_panels: bool,
    show_settings: bool,
    ui_mode: UiMode,
    last_ui_mode: UiMode,
    codepage: CodePage,
    texture_cache: HashMap<u64, egui::TextureHandle>,

    tcp_capture: Option<TcpCapture>,
    tcp_last_error: Option<String>,
    tcp_enabled: bool,
    ignore_noise_jobs: bool,
    ignore_noise_jobs_max_bytes: usize,

    tray: Option<SystemTray>,
    tray_error: Option<String>,
    pending_hide_to_tray: bool,
    hidden_to_tray: bool,

    window: WindowControl,

    simulate_printing: bool,
    sim_bytes_per_sec: usize,

    // Realistic thermal paper effects
    realistic_effects: bool,
    use_thermal_font: bool,
}

impl Default for EscPosViewer {
    fn default() -> Self {
        Self {
            jobs: Vec::new(),
            active_job_idx: None,
            next_job_id: 1,

            max_jobs: 25,
            auto_prune_by_age: false,
            prune_after: Duration::from_secs(60 * 60 * 2),
            auto_scroll_on_print: true,
            paper_width: PaperWidth::W58mm,
            last_paper_width: PaperWidth::W58mm,
            did_apply_initial_window_size: false,
            did_apply_initial_window_position: false,
            show_debug_controls: false,
            show_debug_panels: false,
            show_settings: false,
            ui_mode: UiMode::Preview,
            last_ui_mode: UiMode::Preview,
            codepage: CodePage::Utf8Lossy,
            texture_cache: HashMap::new(),
            tcp_capture: None,
            tcp_last_error: None,
            tcp_enabled: true,
            ignore_noise_jobs: true,
            ignore_noise_jobs_max_bytes: 32,

            tray: None,
            tray_error: None,
            pending_hide_to_tray: false,
            hidden_to_tray: false,

            window: WindowControl::default(),

            simulate_printing: true,
            sim_bytes_per_sec: 1_000,

            realistic_effects: true,
            use_thermal_font: true,
        }
    }
}

impl EscPosViewer {
    /// Maximum number of textures to keep in GPU cache. Beyond this, the cache
    /// is cleared entirely (textures are cheap to recreate on next render).
    const MAX_TEXTURE_CACHE: usize = 100;

    fn should_ignore_tcp_job(&self, bytes: &[u8]) -> bool {
        if !self.ignore_noise_jobs {
            return false;
        }

        if bytes.is_empty() {
            return true;
        }

        if bytes.len() > self.ignore_noise_jobs_max_bytes {
            return false;
        }

        // Heurística: si el job no produce salida visible (texto/imagen/qr/barcode/corte), lo ignoramos.
        // Esto evita tabs "fantasma" de 10-20 bytes que algunos POS envían como consulta de estado.
        let parsed = parse_escpos(bytes, self.codepage);
        for (_state, cmd) in parsed {
            match cmd {
                CommandType::Text(t) => {
                    if t.chars().any(|c| !c.is_whitespace()) {
                        return false;
                    }
                }
                CommandType::Control(control) => match control {
                    Control::RasterImage { .. }
                    | Control::Qr { .. }
                    | Control::Barcode { .. }
                    | Control::Cut => {
                        return false;
                    }
                    _ => {}
                },
                CommandType::Unknown(_) => {}
            }
        }

        true
    }
    fn format_age_short(d: Duration) -> String {
        let secs = d.as_secs();
        if secs < 60 {
            return format!("{}s", secs);
        }
        let mins = secs / 60;
        if mins < 60 {
            return format!("{}m", mins);
        }
        let hours = mins / 60;
        format!("{}h", hours)
    }

    fn ui_job_tabs(&mut self, ui: &mut egui::Ui) {
        if self.jobs.is_empty() {
            return;
        }

        let mut to_close: Option<usize> = None;
        ui.separator();
        egui::ScrollArea::horizontal()
            .id_salt("job_tabs_scroll")
            .max_height(34.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Más espacio entre pestañas (antes quedaban muy pegadas).
                    ui.spacing_mut().item_spacing.x = 10.0;

                    let now = Instant::now();
                    for (idx, job) in self.jobs.iter().enumerate() {
                        let selected = self.active_job_idx == Some(idx);
                        let age = now.duration_since(job.created_at);
                        let mut title = job.label.clone();
                        const MAX: usize = 26;
                        if title.chars().count() > MAX {
                            title = title.chars().take(MAX).collect::<String>();
                            title.push('…');
                        }

                        let tab_text = format!(
                            "#{} {} ({} · {}b)",
                            job.id,
                            title,
                            Self::format_age_short(age),
                            job.full_bytes.len()
                        );

                        let tab_btn = egui::Button::new(tab_text)
                            .selected(selected)
                            .min_size(egui::vec2(0.0, 24.0));
                        if ui.add(tab_btn).clicked() {
                            self.active_job_idx = Some(idx);
                        }

                        // Usar 'X' ASCII (evita el cuadrito por falta de glyph).
                        let close_btn = egui::Button::new(
                            egui::RichText::new("X")
                                .strong()
                                .color(egui::Color32::from_gray(220)),
                        )
                        .fill(egui::Color32::from_gray(70))
                        .min_size(egui::vec2(24.0, 24.0));

                        if ui.add(close_btn).on_hover_text("Cerrar").clicked() {
                            to_close = Some(idx);
                        }
                    }
                });
            });

        if let Some(idx) = to_close {
            self.jobs.remove(idx);
            if self.jobs.is_empty() {
                self.active_job_idx = None;
            } else if let Some(active) = self.active_job_idx {
                if idx == active {
                    self.active_job_idx = Some(active.saturating_sub(1).min(self.jobs.len() - 1));
                } else if idx < active {
                    self.active_job_idx = Some(active - 1);
                }
            }
        }
    }

    fn ui_settings_modal(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

        let mut open = self.show_settings;
        egui::Window::new("Configuración")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Ajustes del visor").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cerrar").clicked() {
                            self.show_settings = false;
                        }
                    });
                });
                ui.separator();

                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        // Captura TCP
                        ui.label(egui::RichText::new("Captura").strong());
                        ui.vertical(|ui| {
                            let enabled_before = self.tcp_enabled;
                            ui.checkbox(&mut self.tcp_enabled, "Escuchar impresora (TCP 9100)");
                            if self.tcp_enabled != enabled_before {
                                if self.tcp_enabled {
                                    self.set_tcp_capture(true, Some(ctx.clone()));
                                } else {
                                    self.set_tcp_capture(false, None);
                                }
                            }
                            if let Some(err) = &self.tcp_last_error {
                                ui.label(
                                    egui::RichText::new(err).color(egui::Color32::RED).small(),
                                );
                            } else {
                                ui.label(egui::RichText::new("127.0.0.1:9100").weak().small());
                            }

                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut self.ignore_noise_jobs,
                                "Ignorar jobs pequeños (ruido)",
                            );
                            if self.ignore_noise_jobs {
                                ui.add(
                                    egui::Slider::new(
                                        &mut self.ignore_noise_jobs_max_bytes,
                                        8..=128,
                                    )
                                    .text("bytes"),
                                );
                            }
                        });
                        ui.end_row();

                        // Simulación
                        ui.label(egui::RichText::new("Impresión").strong());
                        ui.vertical(|ui| {
                            let before_sim = self.simulate_printing;
                            ui.checkbox(&mut self.simulate_printing, "Simular impresión");
                            ui.add(
                                egui::Slider::new(&mut self.sim_bytes_per_sec, 1_000..=200_000)
                                    .text("bytes/s"),
                            );
                            if before_sim && !self.simulate_printing {
                                self.stop_active_simulation_show_full();
                            }
                            if let Some(job) = self.active_job() {
                                if job.sim_active {
                                    let total = job.full_bytes.len().max(1);
                                    let pct = (job.sim_sent as f32 / total as f32) * 100.0;
                                    ui.label(
                                        egui::RichText::new(format!("Progreso: {pct:.0}%"))
                                            .color(egui::Color32::DARK_GRAY)
                                            .small(),
                                    );
                                }
                            }
                        });
                        ui.end_row();

                        // Papel
                        ui.label(egui::RichText::new("Papel").strong());
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.paper_width, PaperWidth::W58mm, "58mm");
                            ui.selectable_value(&mut self.paper_width, PaperWidth::W80mm, "80mm");
                        });
                        ui.end_row();

                        // Codepage
                        ui.label(egui::RichText::new("Codificación").strong());
                        ui.vertical(|ui| {
                            let before = self.codepage;
                            egui::ComboBox::from_label("Codepage")
                                .selected_text(match self.codepage {
                                    CodePage::Utf8Lossy => "UTF-8 (auto)",
                                    CodePage::Cp437 => "CP437",
                                    CodePage::Cp850 => "CP850",
                                    CodePage::Windows1252 => "Windows-1252",
                                    CodePage::Pc858 => "PC858 (€)",
                                    CodePage::Iso88591 => "ISO-8859-1",
                                    CodePage::Cp866 => "CP866 (Cyrillic)",
                                    CodePage::Cp860 => "CP860 (Portuguese)",
                                    CodePage::Cp865 => "CP865 (Nordic)",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Utf8Lossy,
                                        "UTF-8 (auto: fallback Win-1252)",
                                    );
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Cp437,
                                        "CP437 (USA/Europe)",
                                    );
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Cp850,
                                        "CP850 (Multilingual)",
                                    );
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Windows1252,
                                        "Windows-1252",
                                    );
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Pc858,
                                        "PC858 (CP850 + €)",
                                    );
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Iso88591,
                                        "ISO-8859-1 (Latin-1)",
                                    );
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Cp866,
                                        "CP866 (Cyrillic/Russian)",
                                    );
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Cp860,
                                        "CP860 (Portuguese)",
                                    );
                                    ui.selectable_value(
                                        &mut self.codepage,
                                        CodePage::Cp865,
                                        "CP865 (Nordic)",
                                    );
                                });
                            if self.codepage != before {
                                self.reparse_all_jobs();
                            }
                        });
                        ui.end_row();

                        // Historial
                        ui.label(egui::RichText::new("Historial").strong());
                        ui.vertical(|ui| {
                            ui.checkbox(&mut self.auto_scroll_on_print, "Auto-scroll al imprimir");
                            ui.add(egui::Slider::new(&mut self.max_jobs, 1..=100).text("Máx jobs"));
                            ui.checkbox(&mut self.auto_prune_by_age, "Autolimpieza por edad");
                            if self.auto_prune_by_age {
                                let mut mins = (self.prune_after.as_secs() / 60).max(1);
                                ui.add(egui::Slider::new(&mut mins, 1..=24 * 60).text("min"));
                                self.prune_after = Duration::from_secs(mins * 60);
                            }
                            ui.horizontal(|ui| {
                                if ui.button("🧹 Limpiar historial").clicked() {
                                    self.jobs.clear();
                                    self.active_job_idx = None;
                                }
                                ui.label(
                                    egui::RichText::new(format!("Jobs: {}", self.jobs.len()))
                                        .weak(),
                                );
                            });
                        });
                        ui.end_row();

                        // Apariencia
                        ui.label(egui::RichText::new("Apariencia").strong());
                        ui.vertical(|ui| {
                            ui.checkbox(&mut self.realistic_effects, "🎫 Ticket realista");
                            if self.realistic_effects {
                                ui.label(
                                    egui::RichText::new("Bordes ondulados, textura, sombra curvada")
                                        .weak()
                                        .small(),
                                );
                            }
                            ui.checkbox(&mut self.use_thermal_font, "🔤 Fuente térmica");
                            if self.use_thermal_font {
                                ui.label(
                                    egui::RichText::new("DotMatrix (estilo impresora)")
                                        .weak()
                                        .small(),
                                );
                            }
                        });
                        ui.end_row();

                        // Debug
                        ui.label(egui::RichText::new("Debug").strong());
                        ui.vertical(|ui| {
                            ui.checkbox(&mut self.show_debug_panels, "Mostrar Hex/Log");
                            ui.checkbox(&mut self.show_debug_controls, "Debug comandos");
                        });
                        ui.end_row();
                    });
            });

        self.show_settings = open;
    }
    fn active_job(&self) -> Option<&JobEntry> {
        self.active_job_idx.and_then(|idx| self.jobs.get(idx))
    }

    fn active_job_mut(&mut self) -> Option<&mut JobEntry> {
        let idx = self.active_job_idx?;
        self.jobs.get_mut(idx)
    }

    fn stop_active_simulation_show_full(&mut self) {
        let codepage = self.codepage;
        let Some(job) = self.active_job_mut() else {
            return;
        };
        if !job.sim_active {
            return;
        }
        job.sim_active = false;
        job.sim_started_at = None;
        job.display_bytes = job.full_bytes.clone();
        job.parsed_commands = parse_escpos(&job.display_bytes, codepage);
        job.sim_sent = job.display_bytes.len();
    }

    fn prune_jobs(&mut self) {
        let active_id = self.active_job().map(|j| j.id);

        if self.jobs.is_empty() {
            self.active_job_idx = None;
            return;
        }

        // Primero por edad (opcional)
        if self.auto_prune_by_age {
            let now = Instant::now();
            self.jobs
                .retain(|j| now.duration_since(j.created_at) <= self.prune_after);
        }

        // Luego por límite de cantidad (siempre)
        if self.jobs.len() > self.max_jobs {
            let remove_count = self.jobs.len() - self.max_jobs;
            self.jobs.drain(0..remove_count);
        }

        // Reajustar active_job_idx intentando mantener el mismo id.
        if self.jobs.is_empty() {
            self.active_job_idx = None;
            return;
        }

        if let Some(id) = active_id {
            if let Some(idx) = self.jobs.iter().position(|j| j.id == id) {
                self.active_job_idx = Some(idx);
                return;
            }
        }

        self.active_job_idx = Some(self.jobs.len() - 1);
    }

    fn push_new_job(&mut self, label: String, full_data: Vec<u8>) {
        // Si hay una simulación activa, la cerramos mostrando el job completo.
        self.stop_active_simulation_show_full();

        let id = self.next_job_id;
        self.next_job_id = self.next_job_id.saturating_add(1);

        let mut job = JobEntry {
            id,
            label,
            created_at: Instant::now(),
            full_bytes: full_data,
            display_bytes: Vec::new(),
            parsed_commands: Vec::new(),
            sim_active: false,
            sim_started_at: None,
            sim_sent: 0,
        };

        if self.simulate_printing {
            job.sim_active = true;
            job.sim_started_at = Some(Instant::now());
            job.display_bytes = Vec::with_capacity(job.full_bytes.len());
            job.parsed_commands.clear();
            job.sim_sent = 0;
        } else {
            job.display_bytes = job.full_bytes.clone();
            job.parsed_commands = parse_escpos(&job.display_bytes, self.codepage);
            job.sim_sent = job.display_bytes.len();
        }

        self.jobs.push(job);
        self.active_job_idx = Some(self.jobs.len() - 1);
        self.prune_jobs();
    }

    fn target_window_width_px(paper_width: PaperWidth) -> f32 {
        match paper_width {
            PaperWidth::W58mm => 375.0,
            PaperWidth::W80mm => 480.0,
        }
    }

    fn request_window_width(ctx: &egui::Context, width_px: f32) {
        // Mantener la altura actual cuando sea posible.
        let height_px: f32 = ctx
            .input(|i| i.viewport().inner_rect.map(|r| r.height()))
            .unwrap_or(600.0);

        // Nota: en egui 0.29 el comando se llama InnerSize.
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            width_px, height_px,
        )));
    }

    // ===== REALISTIC THERMAL PAPER EFFECTS =====

    /// Color del papel térmico (crema sutil en lugar de blanco puro)
    const THERMAL_PAPER_COLOR: egui::Color32 = egui::Color32::from_rgb(254, 250, 245);

    /// Dibuja el borde superior dentado (efecto de papel arrancado del rollo)
    fn draw_torn_paper_edge(painter: &egui::Painter, rect: egui::Rect, _paper_color: egui::Color32) {
        let wave_height = 4.0;
        let num_teeth = 25;
        let tooth_width = rect.width() / num_teeth as f32;
        
        // Color gris para simular el borde irregular del papel
        let edge_color = egui::Color32::from_gray(230);
        let shadow_color = egui::Color32::from_gray(200);
        
        // Dibujar dientes triangulares en el borde superior
        for i in 0..num_teeth {
            let x_start = rect.left() + i as f32 * tooth_width;
            let x_mid = x_start + tooth_width / 2.0;
            let x_end = x_start + tooth_width;
            
            // Variación en altura para irregularidad
            let height_var = ((i as f32 * 1.7).sin() * 0.5 + 0.5) * wave_height;
            
            // Triángulo del diente
            let points = vec![
                egui::pos2(x_start, rect.top()),
                egui::pos2(x_mid, rect.top() - height_var - 2.0),
                egui::pos2(x_end, rect.top()),
            ];
            
            painter.add(egui::Shape::convex_polygon(
                points.clone(),
                edge_color,
                egui::Stroke::new(0.5, shadow_color),
            ));
        }
        
        // Línea de sombra sutil debajo del borde dentado
        painter.line_segment(
            [egui::pos2(rect.left(), rect.top() + 1.0), egui::pos2(rect.right(), rect.top() + 1.0)],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 15)),
        );
    }

    /// Dibuja la línea de corte (guillotina) en la parte inferior
    fn draw_cut_line(painter: &egui::Painter, rect: egui::Rect) {
        let y = rect.bottom() + 8.0;
        let dash_length = 8.0;
        let gap_length = 4.0;
        let color = egui::Color32::from_gray(180);

        let mut x = rect.left();
        let mut is_dash = true;

        while x < rect.right() {
            if is_dash {
                let end_x = (x + dash_length).min(rect.right());
                painter.line_segment(
                    [egui::pos2(x, y), egui::pos2(end_x, y)],
                    egui::Stroke::new(1.5, color),
                );
                x = end_x;
            } else {
                x += gap_length;
            }
            is_dash = !is_dash;
        }

        // Símbolo de tijeras
        let scissors_x = rect.left() - 15.0;
        painter.text(
            egui::pos2(scissors_x, y),
            egui::Align2::CENTER_CENTER,
            "✂",
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(140),
        );
    }

    /// Dibuja textura de papel sutil (ruido/grano)
    fn draw_paper_texture(painter: &egui::Painter, rect: egui::Rect) {
        // Puntos sutiles para simular textura de papel térmico
        let dot_spacing = 15.0;
        let dot_color = egui::Color32::from_rgba_unmultiplied(200, 195, 190, 25);

        let mut y = rect.top() + 5.0;
        let mut row = 0;
        while y < rect.bottom() {
            let mut x = rect.left() + 5.0 + (row % 2) as f32 * (dot_spacing / 2.0);
            while x < rect.right() {
                // Variación sutil en posición
                let offset_x = ((x * 0.1).sin() * 2.0) as f32;
                let offset_y = ((y * 0.1).cos() * 2.0) as f32;
                painter.circle_filled(egui::pos2(x + offset_x, y + offset_y), 0.5, dot_color);
                x += dot_spacing;
            }
            y += dot_spacing;
            row += 1;
        }
    }

    /// Dibuja indicador de fin de rollo (línea rosa/roja en el costado)
    fn draw_end_of_roll_indicator(painter: &egui::Painter, rect: egui::Rect, ticket_height: f32) {
        // Solo mostrar si el ticket es "largo" (> 600px de contenido)
        if ticket_height < 600.0 {
            return;
        }

        // Línea vertical rosa en el costado derecho
        let indicator_color = egui::Color32::from_rgb(255, 182, 193); // Light pink
        let stripe_width = 3.0;

        let indicator_rect = egui::Rect::from_min_max(
            egui::pos2(rect.right() - stripe_width - 2.0, rect.top() + 10.0),
            egui::pos2(rect.right() - 2.0, rect.bottom() - 10.0),
        );

        painter.rect_filled(indicator_rect, 1.0, indicator_color);
    }

    /// Crea una sombra curvada más realista para el ticket
    fn get_curved_shadow() -> egui::Shadow {
        egui::Shadow {
            offset: egui::vec2(4.0, 6.0),
            blur: 16.0,
            spread: 0.0,
            color: egui::Color32::from_black_alpha(40),
        }
    }

    /// Dibuja efecto de imperfecciones sutiles (opcional)
    fn draw_print_imperfections(painter: &egui::Painter, rect: egui::Rect) {
        // Manchas muy sutiles y aleatorias (deterministas basadas en posición)
        let imperfection_color = egui::Color32::from_rgba_unmultiplied(180, 175, 170, 15);

        // Solo unas pocas manchas
        let positions = [
            (0.23, 0.15),
            (0.67, 0.34),
            (0.12, 0.78),
            (0.89, 0.56),
            (0.45, 0.92),
        ];

        for (tx, ty) in positions {
            let x = egui::lerp(rect.left()..=rect.right(), tx);
            let y = egui::lerp(rect.top()..=rect.bottom(), ty);
            let size = 1.5 + (tx * ty * 3.0);
            painter.circle_filled(egui::pos2(x, y), size, imperfection_color);
        }
    }

    // ===== END REALISTIC EFFECTS =====

    fn draw_printing_reveal_effect(ui: &mut egui::Ui, ticket_rect: egui::Rect, progress: f32) {
        let progress = progress.max(0.0).min(1.0);

        let y = egui::lerp(ticket_rect.top()..=ticket_rect.bottom(), progress);
        let y = y.max(ticket_rect.top()).min(ticket_rect.bottom());

        let painter = ui.painter();

        // Máscara sutil debajo de la barra (zona aún "no impresa").
        let mask_base = ui.visuals().faint_bg_color;
        let mask =
            egui::Color32::from_rgba_unmultiplied(mask_base.r(), mask_base.g(), mask_base.b(), 110);
        let mask_rect = egui::Rect::from_min_max(
            egui::pos2(ticket_rect.left(), y),
            egui::pos2(ticket_rect.right(), ticket_rect.bottom()),
        );
        painter.rect_filled(mask_rect, 0.0, mask);

        // Barra de "escaneo".
        let bar_h = 6.0;
        let bar_base = ui.visuals().selection.bg_fill;
        let bar =
            egui::Color32::from_rgba_unmultiplied(bar_base.r(), bar_base.g(), bar_base.b(), 220);

        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(ticket_rect.left(), (y - bar_h * 0.5).max(ticket_rect.top())),
            egui::pos2(
                ticket_rect.right(),
                (y + bar_h * 0.5).min(ticket_rect.bottom()),
            ),
        );
        painter.rect_filled(bar_rect, 0.0, bar);

        let stroke = ui.visuals().widgets.active.fg_stroke;
        painter.rect_stroke(bar_rect, 0.0, egui::Stroke::new(1.0, stroke.color));
    }
    fn hide_to_tray(&mut self, _ctx: &egui::Context) {
        // Windows: ocultar de verdad (sale del taskbar) via Win32 + WS_EX_TOOLWINDOW.
        // Otros OS: usar ViewportCommand para ocultar la ventana.
        // En ambos casos delegamos a WindowControl que tiene su propio contexto almacenado.
        #[cfg(target_os = "windows")]
        {
            self.window.hide_to_tray();
        }

        #[cfg(not(target_os = "windows"))]
        {
            self.window.hide_to_tray();
        }
    }

    fn tick_simulation(&mut self) {
        let bytes_per_sec = self.sim_bytes_per_sec;
        let codepage = self.codepage;
        let Some(job) = self.active_job_mut() else {
            return;
        };
        if !job.sim_active {
            return;
        }
        let Some(start) = job.sim_started_at else {
            return;
        };

        let elapsed = start.elapsed().as_secs_f32();
        let target = (elapsed * bytes_per_sec as f32) as usize;
        let target = target.min(job.full_bytes.len());

        if target > job.sim_sent {
            job.display_bytes
                .extend_from_slice(&job.full_bytes[job.sim_sent..target]);
            job.sim_sent = target;
            job.parsed_commands = parse_escpos(&job.display_bytes, codepage);
        }

        if job.sim_sent >= job.full_bytes.len() {
            job.sim_active = false;
            job.sim_started_at = None;
        }
    }

    fn set_tcp_capture(&mut self, enabled: bool, repaint_ctx: Option<egui::Context>) {
        if enabled {
            if self.tcp_capture.is_some() {
                return;
            }
            match TcpCapture::start("127.0.0.1:9100", repaint_ctx, Some(self.window.clone())) {
                Ok(capture) => {
                    self.tcp_capture = Some(capture);
                    self.tcp_last_error = None;
                    // Al empezar a escuchar, ponemos la impresora ONLINE
                    let _ = crate::printer_setup::set_printer_offline(false);
                }
                Err(e) => {
                    self.tcp_last_error =
                        Some(format!("No se pudo escuchar 127.0.0.1:9100 ({})", e));
                    self.tcp_capture = None;
                }
            }
        } else if let Some(mut cap) = self.tcp_capture.take() {
            cap.stop();
            self.tcp_capture = None;
            // Al dejar de escuchar, ponemos la impresora OFFLINE para retener trabajos
            let _ = crate::printer_setup::set_printer_offline(true);
        }
    }

    fn try_load_path(&mut self, path: &Path) {
        if let Ok(data) = fs::read(path) {
            self.push_new_job(path.display().to_string(), data);
        }
    }

    fn reparse_all_jobs(&mut self) {
        for job in &mut self.jobs {
            if job.display_bytes.is_empty() {
                job.parsed_commands.clear();
                continue;
            }
            job.parsed_commands = parse_escpos(&job.display_bytes, self.codepage);
        }
    }

    /// Evict the GPU texture cache when it exceeds the maximum size.
    /// A simple clear-all strategy is used: textures are lazily recreated
    /// on the next render pass, so this is safe and keeps memory bounded.
    fn evict_texture_cache(&mut self) {
        if self.texture_cache.len() > Self::MAX_TEXTURE_CACHE {
            self.texture_cache.clear();
        }
    }

    fn debug_label_for_control(control: &Control) -> String {
        match control {
            Control::Newline => "LF".to_string(),
            Control::Tab => "HT (TAB)".to_string(),
            Control::Init => "ESC @ (INIT)".to_string(),
            Control::Bold(on) => format!("ESC E (BOLD={})", on),
            Control::Align(align) => format!("ESC a (ALIGN={:?})", align),
            Control::CodePage(cp) => format!("ESC t (CODEPAGE={:?})", cp),
            Control::Size { raw, width, height } => {
                format!("GS ! (SIZE raw={:02X} w={} h={})", raw, width, height)
            }
            Control::Cut => "GS V (CUT)".to_string(),
            Control::RasterImage {
                m,
                width_bytes,
                height,
                data,
            } => {
                format!(
                    "GS v 0 (IMG m={:02X} {}x{} bytes={})",
                    m,
                    (*width_bytes as usize) * 8,
                    *height as usize,
                    data.len()
                )
            }
            Control::Qr {
                model,
                module_size,
                ecc,
                data,
            } => format!(
                "QR (model={} size={} ecc={} bytes={})",
                model,
                module_size,
                ecc,
                data.len()
            ),
            Control::Barcode { m, data } => {
                format!("GS k (BARCODE m={:02X} bytes={})", m, data.len())
            }
            Control::BarcodeHriPosition(pos) => format!("GS H (HRI={:?})", pos),
            Control::BarcodeHeight(n) => format!("GS h (BARCODE HEIGHT={})", n),
            Control::BarcodeModuleWidth(n) => format!("GS w (BARCODE WIDTH={})", n),
            Control::BarcodeHriFont(n) => format!("GS f (HRI FONT={})", n),
            Control::AbsolutePosition { x } => format!("ESC $ (POS={})", x),
            Control::RelativePosition { offset } => format!("ESC \\ (OFFSET={})", offset),
            Control::Underline(on) => format!("ESC - (UNDERLINE={})", on),
            Control::Reverse(on) => format!("GS B (REVERSE={})", on),
            Control::MasterSelect(n) => format!("ESC ! (MASTER={:02X})", n),
            Control::LineSpacingDefault => "ESC 2 (LINE SPACING DEFAULT)".to_string(),
            Control::LineSpacing(n) => format!("ESC 3 (LINE SPACING={})", n),
            Control::BitImage { mode, width, data } => {
                format!("ESC * (BIT IMAGE mode={} w={} bytes={})", mode, width, data.len())
            }
            Control::FontSelect(on) => format!("ESC M (FONT SELECT font_b={})", on),
            Control::OpenDrawer => "ESC p (OPEN DRAWER)".to_string(),
            Control::EscUnknown(b) => format!("ESC {:02X} (?)", b),
            Control::GsUnknown(b) => format!("GS {:02X} (?)", b),
        }
    }

    fn base_columns(paper_width: PaperWidth, is_font_b: bool) -> usize {
        match (paper_width, is_font_b) {
            (PaperWidth::W58mm, false) => 32,
            (PaperWidth::W58mm, true) => 42,
            (PaperWidth::W80mm, false) => 48,
            (PaperWidth::W80mm, true) => 64,
        }
    }

    fn effective_columns(paper_width: PaperWidth, state: &PrinterState) -> usize {
        let base = Self::base_columns(paper_width, state.is_font_b);
        // Solo dividir por width_mul (ancho de caracteres)
        // El height_mul solo afecta la altura visual, no el ancho de columnas
        let div = state.char_width_mul.max(1) as usize;
        (base / div).max(1)
    }

    fn same_line_style(a: &PrinterState, b: &PrinterState) -> bool {
        a.is_bold == b.is_bold
            && a.is_underline == b.is_underline
            && a.is_reverse == b.is_reverse
            && a.alignment == b.alignment
            && a.char_width_mul == b.char_width_mul
            && a.char_height_mul == b.char_height_mul
            && a.is_font_b == b.is_font_b
    }

    fn nbsp_pad(count: usize) -> String {
        // NBSP para que egui no "coma" el padding inicial.
        "\u{00A0}".repeat(count)
    }

    fn split_and_wrap(text: &str, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![text.to_string()];
        }

        let mut out = Vec::new();
        let mut current = String::new();
        let mut col = 0usize;

        for ch in text.chars() {
            if ch == '\n' {
                out.push(current);
                current = String::new();
                col = 0;
                continue;
            }

            if col >= width {
                out.push(current);
                current = String::new();
                col = 0;
            }

            current.push(ch);
            col += 1;
        }

        if !current.is_empty() {
            out.push(current);
        }

        if out.is_empty() {
            out.push(String::new());
        }

        out
    }

    fn emit_text_with_columns(
        ui: &mut egui::Ui,
        paper_width: PaperWidth,
        state: &PrinterState,
        text: &str,
        use_thermal_font: bool,
    ) {
        let cols = Self::effective_columns(paper_width, state);
        let lines = Self::split_and_wrap(text, cols);
        let lines_len = lines.len();

        for (idx, line) in lines.into_iter().enumerate() {
            let len = line.chars().count();
            
            // Calculate padding based on alignment only
            // (cursor_x is handled by inserting spaces in the text buffer directly)
            let pad = if len >= cols {
                0
            } else {
                match state.alignment {
                    Align::Left => 0,
                    Align::Center => (cols - len) / 2,
                    Align::Right => cols - len,
                }
            };

            let mut display = String::new();
            display.push_str(&Self::nbsp_pad(pad));
            display.push_str(&line);

            // Usar fuente DotMatrix si está habilitada, sino Monospace del sistema
            let font_family = if use_thermal_font {
                egui::FontFamily::Name("DotMatrix".into())
            } else {
                egui::FontFamily::Monospace
            };

            // Tamaño de fuente calculado para que el texto ocupe correctamente el ancho del papel
            // Para 58mm: 300px - 30px padding = 270px ÷ 32 cols ≈ 8.4px por carácter
            // La fuente monospace a 14px tiene aproximadamente 8.4px de ancho por carácter
            let mut base_size = 14.0_f32;
            if state.is_font_b {
                base_size *= 0.75; // Simular Fuente B compacta (25% más pequeña)
            }
            let height_mul = state.char_height_mul.max(1) as f32;
            let width_mul = state.char_width_mul.max(1) as f32;
            // Escalar por el multiplicador de altura para texto grande
            let font_size = base_size * height_mul.max(width_mul);
            
            let mut rich_text = egui::RichText::new(display)
                .color(egui::Color32::BLACK)
                .family(font_family)
                .size(font_size);

            if state.is_bold {
                rich_text = rich_text.strong();
            }

            if state.is_underline {
                rich_text = rich_text.underline();
            }

            if state.is_reverse {
                // Invertir colores: texto blanco sobre fondo negro
                rich_text = rich_text
                    .background_color(egui::Color32::BLACK)
                    .color(egui::Color32::WHITE);
            }

            ui.add(egui::Label::new(rich_text));

            // Añadir el interlineado configurado entre líneas envueltas de un mismo bloque de texto
            if idx < lines_len - 1 {
                let px_width = match paper_width {
                    PaperWidth::W58mm => 240.0,
                    PaperWidth::W80mm => 340.0,
                };
                let total_dots = match paper_width {
                    PaperWidth::W58mm => 384.0,
                    PaperWidth::W80mm => 576.0,
                };
                let dots_to_pixels = px_width / total_dots;
                let n = state.line_spacing.unwrap_or(30) as f32;
                let line_spacing_px = n * dots_to_pixels;
                
                let text_height_px = font_size * 1.15;
                let item_spacing_y = ui.spacing().item_spacing.y;
                let extra_space = (line_spacing_px - text_height_px - item_spacing_y).max(0.0);
                if extra_space > 0.0 {
                    ui.add_space(extra_space);
                }
            }
        }
    }

    fn hash_key<T: Hash>(value: &T) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn raster_to_image(width_bytes: u16, height: u16, data: &[u8]) -> Option<egui::ColorImage> {
        let width_bits = (width_bytes as usize).checked_mul(8)?;
        let height = height as usize;
        if width_bits == 0 || height == 0 {
            return None;
        }
        let expected = (width_bytes as usize).saturating_mul(height);
        if data.len() < expected {
            return None;
        }

        let mut pixels = vec![egui::Color32::WHITE; width_bits * height];

        for y in 0..height {
            let row = &data[y * width_bytes as usize..(y + 1) * width_bytes as usize];
            for (xb, byte) in row.iter().enumerate() {
                for bit in 0..8 {
                    let is_black = (byte & (1 << (7 - bit))) != 0;
                    let x = xb * 8 + bit;
                    let idx = y * width_bits + x;
                    if is_black {
                        pixels[idx] = egui::Color32::BLACK;
                    }
                }
            }
        }

        Some(egui::ColorImage {
            size: [width_bits, height],
            pixels,
        })
    }

    /// Convert ESC * bit image (8/24-pin legacy format) to egui ColorImage.
    /// mode 0,1 = 8-dot vertical (1 byte per column)
    /// mode 32,33 = 24-dot vertical (3 bytes per column)
    fn bitimage_to_image(mode: u8, width: u16, data: &[u8]) -> Option<egui::ColorImage> {
        let width = width as usize;
        if width == 0 {
            return None;
        }

        // 8-pin modes: 1 byte per column = 8 vertical dots
        // 24-pin modes: 3 bytes per column = 24 vertical dots
        let (bytes_per_col, height) = match mode {
            0 | 1 => (1usize, 8usize),
            32 | 33 => (3usize, 24usize),
            _ => (1usize, 8usize),
        };

        let expected = width.saturating_mul(bytes_per_col);
        if data.len() < expected {
            return None;
        }

        let mut pixels = vec![egui::Color32::WHITE; width * height];

        for col in 0..width {
            let col_data = &data[col * bytes_per_col..(col + 1) * bytes_per_col];
            for (byte_idx, &byte) in col_data.iter().enumerate() {
                for bit in 0..8 {
                    let y = byte_idx * 8 + bit;
                    if y >= height {
                        break;
                    }
                    let is_black = (byte & (1 << (7 - bit))) != 0;
                    let idx = y * width + col;
                    if is_black {
                        pixels[idx] = egui::Color32::BLACK;
                    }
                }
            }
        }

        Some(egui::ColorImage {
            size: [width, height],
            pixels,
        })
    }

    fn ecc_to_level(ecc: u8) -> EcLevel {
        match ecc {
            48 => EcLevel::L,
            49 => EcLevel::M,
            50 => EcLevel::Q,
            51 => EcLevel::H,
            _ => EcLevel::M,
        }
    }

    fn qr_to_image(data: &[u8], ecc: u8, module_size: u8) -> Option<egui::ColorImage> {
        let ec_level = Self::ecc_to_level(ecc);
        let code = QrCode::with_error_correction_level(data, ec_level).ok()?;
        let width = code.width();
        if width == 0 {
            return None;
        }

        let module = (module_size as usize).clamp(1, 16);
        let quiet = 4usize;
        let out_w = (width + 2 * quiet) * module;
        let out_h = out_w;

        let mut pixels = vec![egui::Color32::WHITE; out_w * out_h];

        let colors = code.to_colors();
        for y in 0..width {
            for x in 0..width {
                let c = colors[y * width + x];
                if c == Color::Dark {
                    let base_x = (x + quiet) * module;
                    let base_y = (y + quiet) * module;
                    for dy in 0..module {
                        for dx in 0..module {
                            let idx = (base_y + dy) * out_w + (base_x + dx);
                            pixels[idx] = egui::Color32::BLACK;
                        }
                    }
                }
            }
        }

        Some(egui::ColorImage {
            size: [out_w, out_h],
            pixels,
        })
    }

    fn show_image_scaled(
        ui: &mut egui::Ui,
        cache: &mut HashMap<u64, egui::TextureHandle>,
        key: u64,
        image: egui::ColorImage,
        target_width: f32,
    ) {
        let tex = cache
            .entry(key)
            .or_insert_with(|| {
                ui.ctx()
                    .load_texture(format!("tex_{key}"), image, egui::TextureOptions::NEAREST)
            })
            .clone();

        let size = tex.size_vec2();
        let (w, h) = (size.x.max(1.0), size.y.max(1.0));
        let scale = target_width / w;
        let display = egui::vec2(target_width, h * scale);
        ui.image((tex.id(), display));
    }

    fn runs_to_image(
        runs: &[u8],
        start_with_black: bool,
        module_px: usize,
        height_px: usize,
        quiet_zone_modules: usize,
    ) -> Option<egui::ColorImage> {
        if runs.is_empty() || module_px == 0 || height_px == 0 {
            return None;
        }

        let total_modules: usize =
            runs.iter().map(|&r| r as usize).sum::<usize>() + quiet_zone_modules.saturating_mul(2);
        if total_modules == 0 {
            return None;
        }

        let width_px = total_modules.saturating_mul(module_px).max(1);
        let height_px = height_px.max(1);

        let mut pixels = vec![egui::Color32::WHITE; width_px * height_px];

        // Quiet zone a la izquierda.
        let mut x_px = quiet_zone_modules.saturating_mul(module_px);
        let mut black = start_with_black;

        for &run in runs {
            let run_px = (run as usize).saturating_mul(module_px);
            if black && run_px > 0 {
                let x0 = x_px.min(width_px);
                let x1 = (x_px + run_px).min(width_px);
                for y in 0..height_px {
                    let row = y * width_px;
                    for x in x0..x1 {
                        pixels[row + x] = egui::Color32::BLACK;
                    }
                }
            }

            x_px = x_px.saturating_add(run_px);
            black = !black;
            if x_px >= width_px {
                break;
            }
        }

        Some(egui::ColorImage {
            size: [width_px, height_px],
            pixels,
        })
    }

    fn bits01_to_runs(bits: &[u8]) -> Option<(Vec<u8>, bool)> {
        if bits.is_empty() {
            return None;
        }
        let mut runs: Vec<u8> = Vec::new();
        let mut current = bits[0];
        let mut len: usize = 0;
        for &b in bits {
            if b == current {
                len += 1;
            } else {
                runs.push(len.min(255) as u8);
                current = b;
                len = 1;
            }
        }
        runs.push(len.min(255) as u8);
        let start_with_black = bits[0] == 1;
        Some((runs, start_with_black))
    }

    fn clean_code128_hri(data: &[u8]) -> String {
        // ESC/POS suele enviar prefijos como "{B" y escapes "{{".
        let s = String::from_utf8_lossy(data);
        let mut out = String::new();
        let mut chars = s.chars().peekable();

        // Consumir prefijo inicial {A/{B/{C}
        if let Some('{') = chars.peek().copied() {
            let mut clone = chars.clone();
            let _ = clone.next();
            if let Some(next) = clone.next() {
                if matches!(next, 'A' | 'B' | 'C') {
                    let _ = chars.next();
                    let _ = chars.next();
                }
            }
        }

        while let Some(ch) = chars.next() {
            if ch == '{' {
                match chars.peek().copied() {
                    Some('{') => {
                        let _ = chars.next();
                        out.push('{');
                    }
                    Some('A' | 'B' | 'C') => {
                        let _ = chars.next();
                        // cambio de code set: no se imprime
                    }
                    Some('1' | '2' | '3' | '4') => {
                        let _ = chars.next();
                        // FNC*: omitimos en HRI
                    }
                    _ => {
                        // Si no reconocemos, imprimimos el '{'
                        out.push('{');
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn encode_code128_runs(data: &[u8]) -> Option<(Vec<u8>, String)> {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum CodeSet {
            A,
            B,
            C,
        }

        // Tabla Code128 (widths alternando bar/space). Stop (106) tiene 7 dígitos.
        const PATTERNS: [&str; 107] = [
            "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312",
            "132212", "221213", "221312", "231212", "112232", "122132", "122231", "113222",
            "123122", "123221", "223211", "221132", "221231", "213212", "223112", "312131",
            "311222", "321122", "321221", "312212", "322112", "322211", "212123", "212321",
            "232121", "111323", "131123", "131321", "112313", "132113", "132311", "211313",
            "231113", "231311", "112133", "112331", "132131", "113123", "113321", "133121",
            "313121", "211331", "231131", "213113", "213311", "213131", "311123", "311321",
            "331121", "312113", "312311", "332111", "314111", "221411", "431111", "111224",
            "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
            "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111",
            "111242", "121142", "121241", "114212", "124112", "124211", "411212", "421112",
            "421211", "212141", "214121", "412121", "111143", "111341", "131141", "114113",
            "114311", "411113", "411311", "113141", "114131", "311141", "411131", "211412",
            "211214", "211232", "2331112",
        ];

        let s = String::from_utf8_lossy(data);
        let mut bytes = s.as_bytes();

        let mut set = CodeSet::B;
        if bytes.len() >= 2 && bytes[0] == b'{' {
            match bytes[1] {
                b'A' => {
                    set = CodeSet::A;
                    bytes = &bytes[2..];
                }
                b'B' => {
                    set = CodeSet::B;
                    bytes = &bytes[2..];
                }
                b'C' => {
                    set = CodeSet::C;
                    bytes = &bytes[2..];
                }
                _ => {}
            }
        }

        let start_code: u8 = match set {
            CodeSet::A => 103,
            CodeSet::B => 104,
            CodeSet::C => 105,
        };

        let hri = Self::clean_code128_hri(bytes);

        let mut codes: Vec<u8> = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'{' && i + 1 < bytes.len() {
                let n = bytes[i + 1];
                match n {
                    b'{' => {
                        // literal '{'
                        match set {
                            CodeSet::B => {
                                codes.push((b'{' - 32) as u8);
                            }
                            CodeSet::A => {
                                codes.push((b'{' - 32) as u8);
                            }
                            CodeSet::C => {
                                // en C no cabe, cambiamos a B
                                codes.push(100);
                                set = CodeSet::B;
                                codes.push((b'{' - 32) as u8);
                            }
                        }
                        i += 2;
                        continue;
                    }
                    b'A' => {
                        codes.push(101);
                        set = CodeSet::A;
                        i += 2;
                        continue;
                    }
                    b'B' => {
                        codes.push(100);
                        set = CodeSet::B;
                        i += 2;
                        continue;
                    }
                    b'C' => {
                        codes.push(99);
                        set = CodeSet::C;
                        i += 2;
                        continue;
                    }
                    b'1' => {
                        // FNC1
                        codes.push(102);
                        i += 2;
                        continue;
                    }
                    _ => {}
                }
            }

            match set {
                CodeSet::C => {
                    if i + 1 < bytes.len()
                        && bytes[i].is_ascii_digit()
                        && bytes[i + 1].is_ascii_digit()
                    {
                        let v = (bytes[i] - b'0') * 10 + (bytes[i + 1] - b'0');
                        codes.push(v);
                        i += 2;
                    } else {
                        // Cambiar a B para seguir.
                        codes.push(100);
                        set = CodeSet::B;
                    }
                }
                CodeSet::B => {
                    // Code B: ASCII 32..127
                    if b >= 32 && b <= 127 {
                        codes.push((b - 32) as u8);
                    } else {
                        codes.push((b'?' - 32) as u8);
                    }
                    i += 1;
                }
                CodeSet::A => {
                    // Code A: 0..95
                    let v: u8 = if b < 32 {
                        b + 64
                    } else if b <= 95 {
                        b - 32
                    } else {
                        (b'?' - 32) as u8
                    };
                    codes.push(v);
                    i += 1;
                }
            }
        }

        // Checksum
        let mut sum: u32 = start_code as u32;
        for (pos, &c) in codes.iter().enumerate() {
            sum = sum.wrapping_add((c as u32) * ((pos + 1) as u32));
        }
        let checksum: u8 = (sum % 103) as u8;

        let mut all_codes: Vec<u8> = Vec::with_capacity(2 + codes.len());
        all_codes.push(start_code);
        all_codes.extend_from_slice(&codes);
        all_codes.push(checksum);
        all_codes.push(106);

        let mut runs: Vec<u8> = Vec::new();
        for &code in &all_codes {
            let pat = PATTERNS.get(code as usize)?;
            for ch in pat.chars() {
                let d = ch.to_digit(10)? as u8;
                runs.push(d);
            }
        }

        Some((runs, hri))
    }

    fn encode_ean_runs(digits: &str) -> Option<(Vec<u8>, String)> {
        // Devuelve runs (módulos) para EAN-13 o EAN-8, según longitud.
        let mut s: String = digits.chars().filter(|c| c.is_ascii_digit()).collect();
        if s.len() == 7 || s.len() == 12 {
            // calcular checksum y anexar
            let sum: u32 = s
                .chars()
                .rev()
                .enumerate()
                .map(|(i, c)| {
                    let d = c.to_digit(10).unwrap_or(0);
                    let w = if i % 2 == 0 { 3 } else { 1 };
                    d * w
                })
                .sum();
            let chk = (10 - (sum % 10)) % 10;
            s.push(char::from(b'0' + (chk as u8)));
        }

        if s.len() == 13 {
            const L: [&str; 10] = [
                "0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111",
                "0111011", "0110111", "0001011",
            ];
            const G: [&str; 10] = [
                "0100111", "0110011", "0011011", "0100001", "0011101", "0111001", "0000101",
                "0010001", "0001001", "0010111",
            ];
            const R: [&str; 10] = [
                "1110010", "1100110", "1101100", "1000010", "1011100", "1001110", "1010000",
                "1000100", "1001000", "1110100",
            ];
            const PAR: [&str; 10] = [
                "LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG",
                "LGLGGL", "LGGLGL",
            ];

            let first = s.chars().next()?.to_digit(10)? as usize;
            let parity = PAR[first];
            let left = &s[1..7];
            let right = &s[7..13];

            let mut bits: Vec<u8> = Vec::with_capacity(95);
            // start guard
            bits.extend_from_slice(&[1, 0, 1]);
            // left
            for (i, ch) in left.chars().enumerate() {
                let d = ch.to_digit(10)? as usize;
                let pat = match parity.chars().nth(i)? {
                    'L' => L[d],
                    'G' => G[d],
                    _ => L[d],
                };
                for b in pat.bytes() {
                    bits.push((b == b'1') as u8);
                }
            }
            // middle guard
            bits.extend_from_slice(&[0, 1, 0, 1, 0]);
            // right
            for ch in right.chars() {
                let d = ch.to_digit(10)? as usize;
                let pat = R[d];
                for b in pat.bytes() {
                    bits.push((b == b'1') as u8);
                }
            }
            // end guard
            bits.extend_from_slice(&[1, 0, 1]);

            let (runs, start_black) = Self::bits01_to_runs(&bits)?;
            if !start_black {
                return None;
            }
            return Some((runs, s));
        }

        if s.len() == 8 {
            const L: [&str; 10] = [
                "0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111",
                "0111011", "0110111", "0001011",
            ];
            const R: [&str; 10] = [
                "1110010", "1100110", "1101100", "1000010", "1011100", "1001110", "1010000",
                "1000100", "1001000", "1110100",
            ];

            let left = &s[0..4];
            let right = &s[4..8];

            let mut bits: Vec<u8> = Vec::with_capacity(67);
            bits.extend_from_slice(&[1, 0, 1]);
            for ch in left.chars() {
                let d = ch.to_digit(10)? as usize;
                for b in L[d].bytes() {
                    bits.push((b == b'1') as u8);
                }
            }
            bits.extend_from_slice(&[0, 1, 0, 1, 0]);
            for ch in right.chars() {
                let d = ch.to_digit(10)? as usize;
                for b in R[d].bytes() {
                    bits.push((b == b'1') as u8);
                }
            }
            bits.extend_from_slice(&[1, 0, 1]);

            let (runs, start_black) = Self::bits01_to_runs(&bits)?;
            if !start_black {
                return None;
            }
            return Some((runs, s));
        }

        None
    }

    /// Encode Code39 barcode. Supports digits, uppercase letters, and special chars: - . $ / + % SPACE
    fn encode_code39_runs(data: &str) -> Option<(Vec<u8>, String)> {
        // Code39 patterns: 9 elements per character (5 bars, 4 spaces)
        // 1 = wide, 0 = narrow. Pattern is: BSBSBSBSB (bar-space alternating)
        const PATTERNS: &[(char, &str)] = &[
            ('0', "101001101101"), ('1', "110100101011"), ('2', "101100101011"),
            ('3', "110110010101"), ('4', "101001101011"), ('5', "110100110101"),
            ('6', "101100110101"), ('7', "101001011011"), ('8', "110100101101"),
            ('9', "101100101101"), ('A', "110101001011"), ('B', "101101001011"),
            ('C', "110110100101"), ('D', "101011001011"), ('E', "110101100101"),
            ('F', "101101100101"), ('G', "101010011011"), ('H', "110101001101"),
            ('I', "101101001101"), ('J', "101011001101"), ('K', "110101010011"),
            ('L', "101101010011"), ('M', "110110101001"), ('N', "101011010011"),
            ('O', "110101101001"), ('P', "101101101001"), ('Q', "101010110011"),
            ('R', "110101011001"), ('S', "101101011001"), ('T', "101011011001"),
            ('U', "110010101011"), ('V', "100110101011"), ('W', "110011010101"),
            ('X', "100101101011"), ('Y', "110010110101"), ('Z', "100110110101"),
            ('-', "100101011011"), ('.', "110010101101"), (' ', "100110101101"),
            ('$', "100100100101"), ('/', "100100101001"), ('+', "100101001001"),
            ('%', "101001001001"), ('*', "100101101101"), // Start/stop character
        ];

        fn get_pattern(c: char) -> Option<&'static str> {
            PATTERNS.iter()
                .find(|(ch, _)| *ch == c.to_ascii_uppercase())
                .map(|(_, p)| *p)
        }

        let hri: String = data.chars()
            .filter(|c| get_pattern(*c).is_some() && *c != '*')
            .collect();
        
        if hri.is_empty() {
            return None;
        }

        let mut bits: Vec<u8> = Vec::new();
        
        // Start character (*)
        let start = get_pattern('*')?;
        for b in start.bytes() {
            bits.push((b == b'1') as u8);
        }
        bits.push(0); // Inter-character gap
        
        // Data characters
        for c in hri.chars() {
            let pattern = get_pattern(c)?;
            for b in pattern.bytes() {
                bits.push((b == b'1') as u8);
            }
            bits.push(0); // Inter-character gap
        }
        
        // Stop character (*)
        let stop = get_pattern('*')?;
        for b in stop.bytes() {
            bits.push((b == b'1') as u8);
        }
        
        let (runs, start_black) = Self::bits01_to_runs(&bits)?;
        if !start_black {
            return None;
        }
        
        Some((runs, hri))
    }

    fn encode_itf_runs(digits: &str) -> Option<(Vec<u8>, String)> {
        let mut s: String = digits.chars().filter(|c| c.is_ascii_digit()).collect();
        if s.is_empty() {
            return None;
        }
        if s.len() % 2 == 1 {
            s.insert(0, '0');
        }

        fn pat(d: u8) -> [u8; 5] {
            match d {
                0 => [1, 1, 3, 3, 1],
                1 => [3, 1, 1, 1, 3],
                2 => [1, 3, 1, 1, 3],
                3 => [3, 3, 1, 1, 1],
                4 => [1, 1, 3, 1, 3],
                5 => [3, 1, 3, 1, 1],
                6 => [1, 3, 3, 1, 1],
                7 => [1, 1, 1, 3, 3],
                8 => [3, 1, 1, 3, 1],
                _ => [1, 3, 1, 3, 1],
            }
        }

        let bytes = s.as_bytes();
        let mut runs: Vec<u8> = Vec::new();
        // Start: 1010 => [1,1,1,1]
        runs.extend_from_slice(&[1, 1, 1, 1]);

        let mut i = 0usize;
        while i + 1 < bytes.len() {
            let a = (bytes[i] - b'0') as u8;
            let b = (bytes[i + 1] - b'0') as u8;
            let pa = pat(a);
            let pb = pat(b);
            for k in 0..5 {
                runs.push(pa[k]); // bar
                runs.push(pb[k]); // space
            }
            i += 2;
        }

        // Stop: wide bar, narrow space, narrow bar => [3,1,1]
        runs.extend_from_slice(&[3, 1, 1]);
        Some((runs, s))
    }

    fn render_barcode(
        state: &PrinterState,
        m: u8,
        data: &[u8],
        target_width: f32,
    ) -> Option<(egui::ColorImage, Option<String>)> {
        // módulo/ancho en "módulos" (no confundir con píxeles)
        let module_px = (state.barcode_module_width as usize).clamp(1, 6);
        // altura: aproximamos dots a px
        let height_px = ((state.barcode_height as f32) * 0.9).round() as usize;
        let height_px = height_px.clamp(28, 220);
        let quiet = 10usize;

        // m según Epson ESC/POS (GS k):
        // 67 EAN13, 68 EAN8, 70 ITF, 73 CODE128
        let (runs, start_black, hri) = match m {
            0x49 => {
                let (runs, hri) = Self::encode_code128_runs(data)?;
                (runs, true, Some(hri))
            }
            0x43 | 0x44 => {
                let digits = String::from_utf8_lossy(data);
                let (runs, hri) = Self::encode_ean_runs(&digits)?;
                (runs, true, Some(hri))
            }
            0x46 => {
                let digits = String::from_utf8_lossy(data);
                let (runs, hri) = Self::encode_itf_runs(&digits)?;
                (runs, true, Some(hri))
            }
            0x45 => {
                // Code39
                let text = String::from_utf8_lossy(data);
                let (runs, hri) = Self::encode_code39_runs(&text)?;
                (runs, true, Some(hri))
            }
            _ => {
                // No soportado aún
                return None;
            }
        };

        let img = Self::runs_to_image(&runs, start_black, module_px, height_px, quiet)?;

        // Si el barcode queda demasiado pequeño, egui lo escalará con show_image_scaled.
        let _ = target_width;
        Some((img, hri))
    }
}

impl Drop for EscPosViewer {
    fn drop(&mut self) {
        // Al cerrar la aplicación, forzamos que la impresora quede en OFFLINE.
        // Esto permite que los trabajos se acumulen en el Spooler de Windows.
        let _ = crate::printer_setup::set_printer_offline(true);
    }
}

impl eframe::App for EscPosViewer {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Atajo rápido: alternar modo Preview/Completo.
        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.ui_mode = match self.ui_mode {
                UiMode::Preview => UiMode::Full,
                UiMode::Full => UiMode::Preview,
            };
        }

        // Cachear HWND (Windows) o egui::Context (Linux) lo antes posible.
        self.window.try_update_from_frame(frame);
        self.window.set_ctx(ctx);

        // Al entrar a modo Preview: mover la ventana cerca del borde derecho con un pequeño margen.
        if self.ui_mode == UiMode::Preview && self.last_ui_mode != UiMode::Preview {
            self.window.snap_near_right(14);
        }

        // Aplicar tamaño inicial una sola vez.
        if !self.did_apply_initial_window_size {
            self.did_apply_initial_window_size = true;
            let w = Self::target_window_width_px(self.paper_width);
            Self::request_window_width(ctx, w);
        }

        // Si arrancamos en Preview, centrar la ventana una sola vez.
        if !self.did_apply_initial_window_position {
            self.did_apply_initial_window_position = true;
            if self.ui_mode == UiMode::Preview {
                self.window.center_on_screen();
            }
        }

        // Si cambió el papel, ajustar ancho de ventana.
        if self.paper_width != self.last_paper_width {
            self.last_paper_width = self.paper_width;
            let w = Self::target_window_width_px(self.paper_width);
            Self::request_window_width(ctx, w);
        }

        // Inicializar System Tray una sola vez.
        if self.tray.is_none() && self.tray_error.is_none() {
            match SystemTray::new(self.window.clone()) {
                Ok(tray) => self.tray = Some(tray),
                Err(e) => self.tray_error = Some(e),
            }
        }

        // Si el usuario intenta cerrar la ventana (X), ocultamos a bandeja.
        // Nota: esto solo aplica si el tray existe; si falló, dejamos que cierre normal.
        if self.tray.is_some() {
            let close_requested = ctx.input(|i| i.viewport().close_requested());
            if close_requested {
                self.pending_hide_to_tray = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        // Si se pidió ocultar a bandeja (por cerrar ventana), lo aplicamos aquí.
        if self.pending_hide_to_tray {
            self.pending_hide_to_tray = false;
            self.hidden_to_tray = true;
            self.hide_to_tray(ctx);
        }

        self.tick_simulation();
        if self.active_job().is_some_and(|j| j.sim_active) {
            // Forzar repaints para animar la simulación.
            ctx.request_repaint();
        }

        // Autolimpieza / límites del historial.
        self.prune_jobs();

        // Mantener el listener TCP 9100 sincronizado con el checkbox.
        // Evita reintentos automáticos constantes si el puerto está ocupado.
        if self.tcp_enabled {
            if self.tcp_capture.is_none() && self.tcp_last_error.is_none() {
                self.set_tcp_capture(true, Some(ctx.clone()));
            }
        } else if self.tcp_capture.is_some() {
            self.set_tcp_capture(false, None);
        }

        // Captura TCP (impresora virtual 9100)
        if let Some(cap) = &self.tcp_capture {
            let jobs = cap.try_recv_all();
            for job in jobs {
                if self.should_ignore_tcp_job(&job.bytes) {
                    continue;
                }
                let label = format!("TCP 9100 ({})", job.source);
                self.push_new_job(label, job.bytes);

                // Si estaba oculto a la bandeja, el hilo TCP ya lo re-muestra (Windows).
                self.hidden_to_tray = false;
            }
        }

        // Drag & Drop
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            let dropped = ctx.input(|i| i.raw.dropped_files.clone());
            if let Some(file) = dropped.first() {
                if let Some(path) = &file.path {
                    self.try_load_path(path);
                }
            }
        }

        if self.ui_mode == UiMode::Full {
            egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("📂 Abrir").clicked() {
                        if let Some(path) = FileDialog::new()
                            .add_filter("Printer Files", &["prn", "bin", "txt"])
                            .pick_file()
                        {
                            self.try_load_path(&path);
                        }
                    }

                    ui.separator();
                    egui::ComboBox::from_label("Modo")
                        .selected_text(match self.ui_mode {
                            UiMode::Preview => "Preview",
                            UiMode::Full => "Completo",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.ui_mode, UiMode::Preview, "Preview");
                            ui.selectable_value(&mut self.ui_mode, UiMode::Full, "Completo");
                        });

                    ui.separator();
                    if ui.button("⚙ Configuración").clicked() {
                        self.show_settings = true;
                    }

                    if let Some(job) = self.active_job() {
                        ui.separator();
                        ui.label(egui::RichText::new(format!("📄 {}", job.label)).weak());
                    }
                });

                // Barra de jobs (historial / pestañas)
                self.ui_job_tabs(ui);
            });
        }

        if self.ui_mode == UiMode::Full && self.show_debug_panels {
            egui::SidePanel::left("debug_panels")
                .resizable(true)
                .min_width(260.0)
                .default_width(380.0)
                .show(ctx, |ui| {
                    ui.heading("Hex / Log");
                    ui.separator();

                    egui::CollapsingHeader::new("Hex Dump")
                        .default_open(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("hex_scroll")
                                .show(ui, |ui| {
                                    if let Some(job) = self.active_job() {
                                        ui.monospace(pretty_hex(&job.display_bytes));
                                    } else {
                                        ui.monospace("(sin datos)");
                                    }
                                });
                        });

                    ui.add_space(8.0);

                    egui::CollapsingHeader::new("Log (Comandos)")
                        .default_open(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("cmd_scroll")
                                .show(ui, |ui| {
                                    let Some(job) = self.active_job() else {
                                        ui.label(egui::RichText::new("(sin comandos)").weak());
                                        return;
                                    };
                                    for (idx, (_state, cmd)) in
                                        job.parsed_commands.iter().enumerate()
                                    {
                                        let line = match cmd {
                                            CommandType::Text(text) => {
                                                let mut snippet = text.replace(['\r', '\n'], " ");
                                                const MAX: usize = 60;
                                                if snippet.len() > MAX {
                                                    snippet.truncate(MAX);
                                                    snippet.push_str("…");
                                                }
                                                format!("TXT  {}", snippet)
                                            }
                                            CommandType::Control(control) => format!(
                                                "CTL  {}",
                                                Self::debug_label_for_control(control)
                                            ),
                                            CommandType::Unknown(byte) => {
                                                format!("UNK  {:02X}", byte)
                                            }
                                        };

                                        ui.label(
                                            egui::RichText::new(format!("{:04}: {}", idx, line))
                                                .monospace()
                                                .size(10.0),
                                        );
                                    }
                                });
                        });
                });
        }

        // En modo Preview: botón flotante para volver a mostrar menús.
        if self.ui_mode == UiMode::Preview {
            egui::Area::new("preview_menu_button".into())
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 10.0))
                .interactable(true)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("⚙ Config").clicked() {
                            self.show_settings = true;
                        }
                        if ui.button("Menú").clicked() {
                            self.ui_mode = UiMode::Full;
                        }
                        ui.label(egui::RichText::new("(F1)").weak().small());
                    });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let (job_id, stick_bottom) = match self.active_job() {
                Some(j) => (j.id, self.auto_scroll_on_print && j.sim_active),
                None => (0, false),
            };

            ui.push_id(job_id, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("render_scroll")
                    .stick_to_bottom(stick_bottom)
                    .show(ui, |ui| {
                    let desired: f32 = match self.paper_width {
                        PaperWidth::W58mm => 300.0,
                        PaperWidth::W80mm => 450.0,
                    };
                    let available: f32 = ui.available_width().max(0.0);
                    let paper_width: f32 = desired.min((available - 20.0).max(180.0));

                    // Centrar el ticket en la ventana, pero el contenido interno respetará la alineación ESC/POS
                    ui.horizontal(|ui| {
                        // Calcular margen para centrar (incluir padding del Frame: 15px * 2 lados + stroke)
                        let total_ticket_width = paper_width + 30.0 + 2.0; // inner_margin * 2 + stroke
                        let available = ui.available_width();
                        let margin = ((available - total_ticket_width) / 2.0).max(0.0);
                        ui.add_space(margin);
                        
                        // Determinar color y sombra basados en efectos realistas
                        let (paper_fill, shadow, stroke_color) = if self.realistic_effects {
                            (
                                Self::THERMAL_PAPER_COLOR,
                                Self::get_curved_shadow(),
                                egui::Color32::from_gray(210),
                            )
                        } else {
                            (
                                egui::Color32::WHITE,
                                egui::Shadow::default(),
                                egui::Color32::from_gray(200),
                            )
                        };
                        
                        let ticket = egui::Frame::none()
                            .fill(paper_fill)
                            .shadow(shadow)
                            .stroke(egui::Stroke::new(1.0, stroke_color))
                            .inner_margin(15.0)
                            .rounding(0.0) // Sin redondeo para parecer papel real
                            .show(ui, |ui| {
                                // Contenido vertical SIN centrado automático para respetar alineación ESC/POS
                                ui.vertical(|ui| {
                                ui.set_min_width(paper_width);
                                ui.set_max_width(paper_width);
                                ui.set_min_height(400.0);

                                let mut texture_cache = mem::take(&mut self.texture_cache);

                                let Some(job) = self.active_job() else {
                                    ui.label(
                                        egui::RichText::new("Arrastra un .prn/.bin o imprime por TCP 9100")
                                            .color(egui::Color32::GRAY)
                                            .size(12.0),
                                    );
                                    self.texture_cache = texture_cache;
                                    return;
                                };

                                let mut pending: Option<(PrinterState, String)> = None;
                                let use_thermal_font = self.use_thermal_font;
                                let flush_pending = |ui: &mut egui::Ui,
                                                     pending: &mut Option<(PrinterState, String)>| {
                                    if let Some((s, t)) = pending.take() {
                                        if !t.is_empty() {
                                            Self::emit_text_with_columns(
                                                ui,
                                                self.paper_width,
                                                &s,
                                                &t,
                                                use_thermal_font,
                                            );
                                        }
                                    }
                                };

                                for (state, cmd) in &job.parsed_commands {
                                    match cmd {
                                        CommandType::Text(text) => match &mut pending {
                                            Some((ps, buf)) => {
                                                // If cursor_x changed, add padding spaces
                                                if state.cursor_x != ps.cursor_x {
                                                    if let Some(cursor_x) = state.cursor_x {
                                                        // Convert dots to columns
                                                        let dots_per_col = 12u16;
                                                        let target_col = (cursor_x / dots_per_col) as usize;
                                                        let current_col = buf.chars().count();
                                                        if target_col > current_col {
                                                            let spaces = target_col - current_col;
                                                            buf.push_str(&" ".repeat(spaces));
                                                        }
                                                    }
                                                }
                                                
                                                if Self::same_line_style(ps, state) {
                                                    buf.push_str(text);
                                                } else {
                                                    flush_pending(ui, &mut pending);
                                                    pending = Some((state.clone(), text.clone()));
                                                }
                                            }
                                            None => {
                                                pending = Some((state.clone(), text.clone()));
                                            }
                                        },
                                        CommandType::Control(control) => {
                                            if self.show_debug_controls {
                                                let label =
                                                    Self::debug_label_for_control(control);
                                                ui.label(
                                                    egui::RichText::new(label)
                                                        .size(9.0)
                                                        .color(egui::Color32::GRAY)
                                                        .monospace(),
                                                );
                                            }

                                            match control {
                                                Control::Newline => {
                                                    flush_pending(ui, &mut pending);
                                                    
                                                    let total_dots = match self.paper_width {
                                                        PaperWidth::W58mm => 384.0,
                                                        PaperWidth::W80mm => 576.0,
                                                    };
                                                    let dots_to_pixels = paper_width / total_dots;
                                                    let n = state.line_spacing.unwrap_or(30) as f32;
                                                    let line_spacing_px = n * dots_to_pixels;
                                                    
                                                    let mut base_size = 14.0_f32;
                                                    if state.is_font_b {
                                                        base_size *= 0.75;
                                                    }
                                                    let height_mul = state.char_height_mul.max(1) as f32;
                                                    let font_size = base_size * height_mul;
                                                    let text_height_px = font_size * 1.15;
                                                    
                                                    let item_spacing_y = ui.spacing().item_spacing.y;
                                                    let extra_space = (line_spacing_px - text_height_px - item_spacing_y).max(0.0);
                                                    
                                                    ui.add_space(extra_space.max(1.0));
                                                }
                                                Control::Cut => {
                                                    flush_pending(ui, &mut pending);
                                                    ui.add_space(15.0);
                                                    ui.label(
                                                        egui::RichText::new(
                                                            "- - - - - - CORTE - - - - - -",
                                                        )
                                                        .size(10.0)
                                                        .color(egui::Color32::GRAY),
                                                    );
                                                    ui.add_space(15.0);
                                                }
                                                Control::RasterImage {
                                                    m: _,
                                                    width_bytes,
                                                    height,
                                                    data,
                                                } => {
                                                    flush_pending(ui, &mut pending);
                                                    if let Some(img) = Self::raster_to_image(
                                                        *width_bytes,
                                                        *height,
                                                        data,
                                                    ) {
                                                        let key = Self::hash_key(&(
                                                            "raster",
                                                            width_bytes,
                                                            height,
                                                            data,
                                                        ));
                                                        
                                                        // Calcular ancho visual proporcional real basado en dots
                                                        let total_dots = match self.paper_width {
                                                            PaperWidth::W58mm => 384.0,
                                                            PaperWidth::W80mm => 576.0,
                                                        };
                                                        let dots_to_pixels = paper_width / total_dots;
                                                        let img_display_width = ((*width_bytes as f32 * 8.0) * dots_to_pixels).min(paper_width);

                                                        match state.alignment {
                                                            Align::Center => {
                                                                ui.vertical_centered(|ui| {
                                                                    Self::show_image_scaled(
                                                                        ui,
                                                                        &mut texture_cache,
                                                                        key,
                                                                        img,
                                                                        img_display_width,
                                                                    );
                                                                });
                                                            }
                                                            Align::Right => {
                                                                ui.with_layout(
                                                                    egui::Layout::right_to_left(egui::Align::Center),
                                                                    |ui| {
                                                                        Self::show_image_scaled(
                                                                            ui,
                                                                            &mut texture_cache,
                                                                            key,
                                                                            img,
                                                                            img_display_width,
                                                                        );
                                                                    },
                                                                );
                                                            }
                                                            Align::Left => {
                                                                Self::show_image_scaled(
                                                                    ui,
                                                                    &mut texture_cache,
                                                                    key,
                                                                    img,
                                                                    img_display_width,
                                                                );
                                                            }
                                                        }
                                                        ui.add_space(8.0);
                                                    }
                                                }
                                                Control::Qr {
                                                    model: _,
                                                    module_size,
                                                    ecc,
                                                    data,
                                                } => {
                                                    flush_pending(ui, &mut pending);
                                                    if let Some(img) = Self::qr_to_image(
                                                        data,
                                                        *ecc,
                                                        *module_size,
                                                    ) {
                                                        let key = Self::hash_key(&(
                                                            "qr",
                                                            ecc,
                                                            module_size,
                                                            data,
                                                        ));
                                                        let target =
                                                            paper_width.min(260.0);

                                                        match state.alignment {
                                                            Align::Center => {
                                                                ui.vertical_centered(|ui| {
                                                                    Self::show_image_scaled(
                                                                        ui,
                                                                        &mut texture_cache,
                                                                        key,
                                                                        img,
                                                                        target,
                                                                    );
                                                                });
                                                            }
                                                            Align::Right => {
                                                                ui.with_layout(
                                                                    egui::Layout::right_to_left(egui::Align::Center),
                                                                    |ui| {
                                                                        Self::show_image_scaled(
                                                                            ui,
                                                                            &mut texture_cache,
                                                                            key,
                                                                            img,
                                                                            target,
                                                                        );
                                                                    },
                                                                );
                                                            }
                                                            Align::Left => {
                                                                Self::show_image_scaled(
                                                                    ui,
                                                                    &mut texture_cache,
                                                                    key,
                                                                    img,
                                                                    target,
                                                                );
                                                            }
                                                        }
                                                        ui.add_space(8.0);
                                                    } else {
                                                        ui.label(
                                                            egui::RichText::new(
                                                                "[QR inválido]",
                                                            )
                                                            .color(egui::Color32::GRAY)
                                                            .monospace(),
                                                        );
                                                    }
                                                }
                                                Control::OpenDrawer => {
                                                    flush_pending(ui, &mut pending);
                                                    ui.add_space(8.0);
                                                    ui.group(|ui| {
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                egui::RichText::new("🔓 CAJÓN PORTAMONEDAS ABIERTO")
                                                                    .size(11.0)
                                                                    .color(egui::Color32::from_rgb(217, 119, 6)) // Amber-600
                                                                    .strong(),
                                                            );
                                                        });
                                                    });
                                                    ui.add_space(8.0);
                                                }
                                                Control::Barcode { m, data } => {
                                                    flush_pending(ui, &mut pending);
                                                    ui.add_space(6.0);
                                                    let hri_pos = state.barcode_hri;
                                                    let target = paper_width.min(360.0);
                                                    if let Some((img, hri)) =
                                                        Self::render_barcode(state, *m, data, target)
                                                    {
                                                        let key = Self::hash_key(&(
                                                            "barcode",
                                                            *m,
                                                            data.len(),
                                                            state.barcode_hri as u8,
                                                            state.barcode_height,
                                                            state.barcode_module_width,
                                                            Self::hash_key(data),
                                                        ));

                                                        let hri_text = hri.unwrap_or_else(|| String::from_utf8_lossy(data).to_string());

                                                        // Mostrar HRI arriba
                                                        if matches!(hri_pos, BarcodeHriPosition::Above | BarcodeHriPosition::Both) {
                                                            ui.label(
                                                                egui::RichText::new(hri_text.clone())
                                                                    .color(egui::Color32::BLACK)
                                                                    .family(egui::FontFamily::Monospace)
                                                                    .size(12.0),
                                                            );
                                                            ui.add_space(2.0);
                                                        }

                                                        match state.alignment {
                                                            Align::Center => {
                                                                ui.vertical_centered(|ui| {
                                                                    Self::show_image_scaled(
                                                                        ui,
                                                                        &mut texture_cache,
                                                                        key,
                                                                        img,
                                                                        target,
                                                                    );
                                                                });
                                                            }
                                                            Align::Right => {
                                                                ui.with_layout(
                                                                    egui::Layout::right_to_left(egui::Align::Center),
                                                                    |ui| {
                                                                        Self::show_image_scaled(
                                                                            ui,
                                                                            &mut texture_cache,
                                                                            key,
                                                                            img,
                                                                            target,
                                                                        );
                                                                    },
                                                                );
                                                            }
                                                            Align::Left => {
                                                                Self::show_image_scaled(
                                                                    ui,
                                                                    &mut texture_cache,
                                                                    key,
                                                                    img,
                                                                    target,
                                                                );
                                                            }
                                                        }

                                                        // Mostrar HRI abajo
                                                        if matches!(hri_pos, BarcodeHriPosition::Below | BarcodeHriPosition::Both) {
                                                            ui.add_space(2.0);
                                                            ui.label(
                                                                egui::RichText::new(hri_text)
                                                                    .color(egui::Color32::BLACK)
                                                                    .family(egui::FontFamily::Monospace)
                                                                    .size(12.0),
                                                            );
                                                        }
                                                    } else {
                                                        // Fallback: placeholder
                                                        let preview = String::from_utf8_lossy(data);
                                                        ui.label(
                                                            egui::RichText::new(format!(
                                                                "[BARCODE m={:02X}] {}",
                                                                m, preview
                                                            ))
                                                            .color(egui::Color32::BLACK)
                                                            .monospace()
                                                            .size(11.0),
                                                        );
                                                    }
                                                    ui.add_space(6.0);
                                                }
                                                Control::Tab => {
                                                    // Agregar tabulador al texto pendiente para simular columnas
                                                    if let Some((_, ref mut text)) = pending {
                                                        // Tab = saltar a siguiente posición de tabulador (cada 8 caracteres típicamente)
                                                        let current_len = text.chars().count();
                                                        let next_tab = ((current_len / 8) + 1) * 8;
                                                        let spaces = next_tab.saturating_sub(current_len);
                                                        text.push_str(&" ".repeat(spaces.max(1)));
                                                    }
                                                }
                                                Control::BitImage { mode, width, data } => {
                                                    flush_pending(ui, &mut pending);
                                                    if let Some(img) = Self::bitimage_to_image(
                                                        *mode,
                                                        *width,
                                                        data,
                                                    ) {
                                                        let key = Self::hash_key(&(
                                                            "bitimage",
                                                            mode,
                                                            width,
                                                            data,
                                                        ));
                                                        Self::show_image_scaled(
                                                            ui,
                                                            &mut texture_cache,
                                                            key,
                                                            img,
                                                            paper_width,
                                                        );
                                                        ui.add_space(4.0);
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        CommandType::Unknown(_) => {}
                                    }
                                }

                                flush_pending(ui, &mut pending);

                                self.texture_cache = texture_cache;
                                self.evict_texture_cache();
                                }); // fin ui.vertical
                            });

                        if let Some(job) = self.active_job() {
                            if job.sim_active && !job.full_bytes.is_empty() {
                                let progress = job.sim_sent as f32 / job.full_bytes.len() as f32;
                            Self::draw_printing_reveal_effect(ui, ticket.response.rect, progress);
                            }
                        }

                        // ===== REALISTIC EFFECTS =====
                        if self.realistic_effects {
                            let painter = ui.painter();
                            let rect = ticket.response.rect;
                            
                            // 1. Borde superior dentado (efecto papel arrancado)
                            Self::draw_torn_paper_edge(painter, rect, Self::THERMAL_PAPER_COLOR);
                            
                            // 2. Línea de corte inferior (guillotina con tijeras)
                            Self::draw_cut_line(painter, rect);
                            
                            // 3. Textura de papel (grano sutil)
                            Self::draw_paper_texture(painter, rect);
                            
                            // 4. Imperfecciones sutiles (manchas muy leves)
                            Self::draw_print_imperfections(painter, rect);
                            
                            // 5. Indicador de fin de rollo (línea rosa si ticket largo)
                            let ticket_height = rect.height();
                            Self::draw_end_of_roll_indicator(painter, rect, ticket_height);
                        }
                        // ===== END REALISTIC EFFECTS =====

                        if self.ui_mode == UiMode::Preview {
                            ticket.response.context_menu(|ui| {
                                ui.label("Modo");
                                ui.separator();
                                ui.selectable_value(&mut self.ui_mode, UiMode::Preview, "Preview");
                                ui.selectable_value(&mut self.ui_mode, UiMode::Full, "Completo");
                            });
                        }
                    });
                });
            });
        });

        // Modal de configuración (se muestra sobre Preview o Completo).
        self.ui_settings_modal(ctx);

        self.last_ui_mode = self.ui_mode;
    }
}
