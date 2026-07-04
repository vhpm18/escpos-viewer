use crate::model::{Align, BarcodeHriPosition, CodePage, CommandType, Control, PrinterState};
use oem_cp::{Cp437, Cp850, StringExt};

pub type ParsedCommand = (PrinterState, CommandType);

fn decode_text(bytes: &[u8], codepage: CodePage) -> String {
    match codepage {
        // Muchísimos POS envían bytes tipo Windows-1252/Latin1 (p.ej. 0xA1 = '¡')
        // y NO UTF-8. Si decodificamos como UTF-8 (lossy) sale '�'.
        // Solución: intentar UTF-8 estricto; si falla, hacer fallback a Windows-1252.
        CodePage::Utf8Lossy => match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
                text.into_owned()
            }
        },
        CodePage::Cp437 => String::from_cp::<Cp437>(bytes),
        CodePage::Cp850 => String::from_cp::<Cp850>(bytes),
        CodePage::Windows1252 => {
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            text.into_owned()
        }
        CodePage::Pc858 => {
            // PC858 is CP850 with Euro symbol at 0xD5
            // For simplicity, use CP850 as base (Euro symbol mapping is close)
            String::from_cp::<Cp850>(bytes)
        }
        CodePage::Iso88591 => {
            let (text, _, _) = encoding_rs::ISO_8859_2.decode(bytes);
            text.into_owned()
        }
        CodePage::Cp866 => {
            let (text, _, _) = encoding_rs::IBM866.decode(bytes);
            text.into_owned()
        }
        CodePage::Cp860 => {
            // Portuguese - fallback to CP850 (similar)
            String::from_cp::<Cp850>(bytes)
        }
        CodePage::Cp865 => {
            // Nordic - fallback to CP850 (similar)
            String::from_cp::<Cp850>(bytes)
        }
    }
}

// --- Lógica de Parsing (Simplificada) ---
pub fn parse_escpos(data: &[u8], codepage: CodePage) -> Vec<ParsedCommand> {
    let mut commands = Vec::new();
    let mut i = 0;

    let mut state = PrinterState::default();
    let mut active_codepage = codepage;

    // Estado QR (GS ( k): se arma con Store, y se emite en Print.
    let mut qr_model: u8 = 2; // 1 o 2 (default: 2)
    let mut qr_module_size: u8 = 4; // 1..16 (default: 4)
    let mut qr_ecc: u8 = 48; // 48..51 (L/M/Q/H) (default: 48)
    let mut qr_data: Vec<u8> = Vec::new();

    while i < data.len() {
        let byte = data[i];

        match byte {
            // LF
            0x0A => {
                commands.push((state.clone(), CommandType::Control(Control::Newline)));
                // Reset cursor position on newline (ESC/POS standard behavior)
                state.cursor_x = None;
                i += 1;
            }
            // HT (Horizontal Tab)
            0x09 => {
                commands.push((state.clone(), CommandType::Control(Control::Tab)));
                i += 1;
            }
            // CR
            0x0D => {
                i += 1;
            }

            // ESC
            0x1B => {
                if i + 1 < data.len() {
                    let next_byte = data[i + 1];
                    match next_byte {
                        0x40 => {
                            // ESC @
                            commands.push((state.clone(), CommandType::Control(Control::Init)));
                            state = PrinterState::default();
                            // Resetear estado de QR
                            qr_model = 2;
                            qr_module_size = 4;
                            qr_ecc = 48;
                            qr_data.clear();
                            i += 2;
                        }
                        0x45 => {
                            // ESC E n
                            if i + 2 < data.len() {
                                let val = data[i + 2];
                                state.is_bold = val == 1;
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::Bold(state.is_bold)),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x61 => {
                            // ESC a n
                            if i + 2 < data.len() {
                                let val = data[i + 2];
                                state.alignment = match val {
                                    1 | 49 => Align::Center,
                                    2 | 50 => Align::Right,
                                    _ => Align::Left,
                                };
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::Align(state.alignment)),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x74 => {
                            // ESC t n (selección de code table / codepage)
                            // Mapeo común (Epson): 0=CP437, 2=CP850, 16=Windows-1252, etc.
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                if let Some(mapped) = match n {
                                    0 => Some(CodePage::Cp437),
                                    2 => Some(CodePage::Cp850),
                                    3 => Some(CodePage::Cp860), // Portuguese
                                    4 => Some(CodePage::Cp865), // Nordic
                                    6 => Some(CodePage::Iso88591), // ISO-8859-1
                                    16 => Some(CodePage::Windows1252),
                                    17 => Some(CodePage::Cp866), // Cyrillic
                                    19 => Some(CodePage::Pc858), // CP850 + Euro
                                    _ => None,
                                } {
                                    active_codepage = mapped;
                                    commands.push((
                                        state.clone(),
                                        CommandType::Control(Control::CodePage(mapped)),
                                    ));
                                } else {
                                    // Consumimos igualmente para no filtrar el parámetro como texto.
                                    commands.push((
                                        state.clone(),
                                        CommandType::Control(Control::EscUnknown(next_byte)),
                                    ));
                                }
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x24 => {
                            // ESC $ nL nH (Absolute print position)
                            if i + 3 < data.len() {
                                let n_l = data[i + 2] as u16;
                                let n_h = data[i + 3] as u16;
                                let pos = n_l | (n_h << 8);
                                state.cursor_x = Some(pos);
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::AbsolutePosition { x: pos }),
                                ));
                                i += 4;
                            } else {
                                i += 2;
                            }
                        }
                        0x5C => {
                            // ESC \ nL nH (Relative print position)
                            if i + 3 < data.len() {
                                let n_l = data[i + 2] as u16;
                                let n_h = data[i + 3] as u16;
                                let offset = (n_l | (n_h << 8)) as i16;
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::RelativePosition { offset }),
                                ));
                                i += 4;
                            } else {
                                i += 2;
                            }
                        }
                        0x2D => {
                            // ESC - n (Underline)
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.is_underline = n == 1 || n == 2;
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::Underline(state.is_underline)),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x21 => {
                            // ESC ! n (Master select)
                            // Bit 3: Bold, Bit 4: Double height, Bit 5: Double width, Bit 7: Underline
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.is_bold = (n & 0x08) != 0;
                                state.is_underline = (n & 0x80) != 0;
                                let dh = if (n & 0x10) != 0 { 1 } else { 0 };
                                let dw = if (n & 0x20) != 0 { 1 } else { 0 };
                                state.char_height_mul = dh + 1;
                                state.char_width_mul = dw + 1;
                                state.font_scale = state.char_height_mul as f32;
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::MasterSelect(n)),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x32 => {
                            // ESC 2 (Default line spacing)
                            state.line_spacing = None;
                            commands.push((
                                state.clone(),
                                CommandType::Control(Control::LineSpacingDefault),
                            ));
                            i += 2;
                        }
                        0x33 => {
                            // ESC 3 n (Set line spacing)
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.line_spacing = Some(n);
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::LineSpacing(n)),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x2A => {
                            // ESC * m nL nH d... (Select bit image mode)
                            if i + 4 < data.len() {
                                let m = data[i + 2];
                                let n_l = data[i + 3] as u16;
                                let n_h = data[i + 4] as u16;
                                let width = n_l | (n_h << 8);
                                // Bytes per column: mode 0,1 = 1 byte (8 pins), mode 32,33 = 3 bytes (24 pins)
                                let bytes_per_col = match m {
                                    0 | 1 => 1usize,
                                    32 | 33 => 3usize,
                                    _ => 1usize,
                                };
                                let data_len = (width as usize).saturating_mul(bytes_per_col);
                                let start = i + 5;
                                let end = start.saturating_add(data_len);
                                if end <= data.len() {
                                    let img_data = data[start..end].to_vec();
                                    commands.push((
                                        state.clone(),
                                        CommandType::Control(Control::BitImage {
                                            mode: m,
                                            width,
                                            data: img_data,
                                        }),
                                    ));
                                    i = end;
                                } else {
                                    i += 2;
                                }
                            } else {
                                i += 2;
                            }
                        }
                        0x4D => {
                            // ESC M n (Select font: 0=Font A, 1=Font B)
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.is_font_b = n == 1;
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::FontSelect(state.is_font_b)),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x70 => {
                            // ESC p m t1 t2 (Generate pulse / Open drawer)
                            if i + 4 < data.len() {
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::OpenDrawer),
                                ));
                                i += 5;
                            } else {
                                i += 2;
                            }
                        }
                        _ => {
                            commands.push((
                                state.clone(),
                                CommandType::Control(Control::EscUnknown(next_byte)),
                            ));
                            i += 2;
                        }
                    }
                } else {
                    i += 1;
                }
            }

            // GS
            0x1D => {
                if i + 1 < data.len() {
                    let next_byte = data[i + 1];
                    match next_byte {
                        // GS H n (HRI position)
                        0x48 => {
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.barcode_hri = match n {
                                    1 => BarcodeHriPosition::Above,
                                    2 => BarcodeHriPosition::Below,
                                    3 => BarcodeHriPosition::Both,
                                    _ => BarcodeHriPosition::None,
                                };
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::BarcodeHriPosition(
                                        state.barcode_hri,
                                    )),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        // GS h n (height)
                        0x68 => {
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.barcode_height = n.max(1);
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::BarcodeHeight(
                                        state.barcode_height,
                                    )),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        // GS w n (module width)
                        0x77 => {
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.barcode_module_width = n.max(1);
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::BarcodeModuleWidth(
                                        state.barcode_module_width,
                                    )),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        // GS f n (HRI font)
                        0x66 => {
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.barcode_hri_font = n;
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::BarcodeHriFont(
                                        state.barcode_hri_font,
                                    )),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x76 => {
                            // GS v 0 m xL xH yL yH d...
                            if i + 7 < data.len() && data[i + 2] == 0x30 {
                                let m = data[i + 3];
                                let x_l = data[i + 4] as u16;
                                let x_h = data[i + 5] as u16;
                                let y_l = data[i + 6] as u16;
                                let y_h = data[i + 7] as u16;
                                let width_bytes = x_l | (x_h << 8);
                                let height = y_l | (y_h << 8);

                                let data_len =
                                    (width_bytes as usize).saturating_mul(height as usize);
                                let start = i + 8;
                                let end = start.saturating_add(data_len);
                                if end <= data.len() {
                                    let img = data[start..end].to_vec();
                                    commands.push((
                                        state.clone(),
                                        CommandType::Control(Control::RasterImage {
                                            m,
                                            width_bytes,
                                            height,
                                            data: img,
                                        }),
                                    ));
                                    i = end;
                                } else {
                                    // Truncado; consumir cabecera y seguir.
                                    i += 2;
                                }
                            } else {
                                i += 2;
                            }
                        }
                        0x28 => {
                            // GS ( k  pL pH cn fn ...
                            if i + 5 < data.len() && data[i + 2] == 0x6B {
                                let p_l = data[i + 3] as usize;
                                let p_h = data[i + 4] as usize;
                                let total = p_l | (p_h << 8);
                                let start = i + 5;
                                let end = start.saturating_add(total);
                                if end <= data.len() && total >= 2 {
                                    let cn = data[start];
                                    let fn_ = data[start + 1];
                                    let payload = &data[start + 2..end];

                                    // QR: cn = 49 (0x31)
                                    if cn == 0x31 {
                                        match fn_ {
                                            0x41 => {
                                                // Set model: [m, 0]
                                                if payload.len() >= 1 {
                                                    qr_model = payload[0];
                                                }
                                            }
                                            0x43 => {
                                                // Set module size: [n]
                                                if payload.len() >= 1 {
                                                    qr_module_size = payload[0];
                                                }
                                            }
                                            0x45 => {
                                                // Set ECC: [n]
                                                if payload.len() >= 1 {
                                                    qr_ecc = payload[0];
                                                }
                                            }
                                            0x50 => {
                                                // Store data: [m=48, data...]
                                                if payload.len() >= 1 {
                                                    let m = payload[0];
                                                    if m == 0x30 {
                                                        qr_data.extend_from_slice(&payload[1..]);
                                                    }
                                                }
                                            }
                                            0x51 => {
                                                // Print: [m=48]
                                                if !qr_data.is_empty() {
                                                    commands.push((
                                                        state.clone(),
                                                        CommandType::Control(Control::Qr {
                                                            model: qr_model,
                                                            module_size: qr_module_size,
                                                            ecc: qr_ecc,
                                                            data: qr_data.clone(),
                                                        }),
                                                    ));
                                                    qr_data.clear();
                                                }
                                            }
                                            _ => {}
                                        }
                                        i = end;
                                    } else {
                                        // Otro GS ( k
                                        commands.push((
                                            state.clone(),
                                            CommandType::Control(Control::GsUnknown(0x28)),
                                        ));
                                        i += 2;
                                    }
                                } else {
                                    i += 2;
                                }
                            } else {
                                i += 2;
                            }
                        }
                        0x6B => {
                            // GS k (barcode)
                            if i + 2 < data.len() {
                                let m = data[i + 2];
                                if m <= 6 {
                                    // NUL-terminated
                                    let mut j = i + 3;
                                    while j < data.len() && data[j] != 0x00 {
                                        j += 1;
                                    }
                                    let end = j.min(data.len());
                                    let payload = data[i + 3..end].to_vec();
                                    commands.push((
                                        state.clone(),
                                        CommandType::Control(Control::Barcode { m, data: payload }),
                                    ));
                                    // saltar NUL si existe
                                    i = if j < data.len() { j + 1 } else { j };
                                } else {
                                    // length-prefixed
                                    if i + 3 < data.len() {
                                        let n = data[i + 3] as usize;
                                        let start = i + 4;
                                        let end = start.saturating_add(n);
                                        if end <= data.len() {
                                            let payload = data[start..end].to_vec();
                                            commands.push((
                                                state.clone(),
                                                CommandType::Control(Control::Barcode {
                                                    m,
                                                    data: payload,
                                                }),
                                            ));
                                            i = end;
                                        } else {
                                            i += 2;
                                        }
                                    } else {
                                        i += 2;
                                    }
                                }
                            } else {
                                i += 2;
                            }
                        }
                        0x21 => {
                            // GS ! n
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                // ESC/POS: low nibble = width, high nibble = height.
                                let width = n & 0x0F;
                                let height = (n >> 4) & 0x0F;
                                state.char_width_mul = width.saturating_add(1);
                                state.char_height_mul = height.saturating_add(1);
                                state.font_scale = state.char_height_mul as f32;
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::Size {
                                        raw: n,
                                        width,
                                        height,
                                    }),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        0x56 => {
                            // GS V (Cut)
                            commands.push((state.clone(), CommandType::Control(Control::Cut)));
                            // hack: saltar args comunes
                            i += 3;
                        }
                        0x42 => {
                            // GS B n (Reverse printing - white on black)
                            if i + 2 < data.len() {
                                let n = data[i + 2];
                                state.is_reverse = (n & 0x01) != 0;
                                commands.push((
                                    state.clone(),
                                    CommandType::Control(Control::Reverse(state.is_reverse)),
                                ));
                                i += 3;
                            } else {
                                i += 2;
                            }
                        }
                        _ => {
                            commands.push((
                                state.clone(),
                                CommandType::Control(Control::GsUnknown(next_byte)),
                            ));
                            i += 2;
                        }
                    }
                } else {
                    i += 1;
                }
            }

            // Texto
            _ => {
                let mut text_bytes = Vec::new();
                let mut j = i;
                while j < data.len() {
                    let b = data[j];
                    // Parar en controles, incluyendo LF/CR, para que se procesen como comandos.
                    if b < 0x20 {
                        break;
                    }
                    if b == 0x1B || b == 0x1D {
                        break;
                    }
                    text_bytes.push(b);
                    j += 1;
                }

                if !text_bytes.is_empty() {
                    let text = decode_text(&text_bytes, active_codepage);
                    commands.push((state.clone(), CommandType::Text(text)));
                    i = j;
                } else {
                    commands.push((state.clone(), CommandType::Unknown(byte)));
                    i += 1;
                }
            }
        }
    }

    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_text(parsed: &[ParsedCommand]) -> Vec<String> {
        parsed
            .iter()
            .filter_map(|(_, c)| match c {
                CommandType::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn parses_basic_text_and_newline() {
        let data = b"Hola\n";
        let parsed = parse_escpos(data, CodePage::Utf8Lossy);
        assert!(parsed
            .iter()
            .any(|(_, c)| matches!(c, CommandType::Text(t) if t.contains("Hola"))));
        assert!(parsed
            .iter()
            .any(|(_, c)| matches!(c, CommandType::Control(Control::Newline))));
    }

    #[test]
    fn esc_init_resets_state() {
        let data = [0x1B, 0x45, 0x01, b'A', 0x1B, 0x40, b'B'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);

        // A debe estar en bold
        let a_state = parsed
            .iter()
            .find(|(_, c)| matches!(c, CommandType::Text(t) if t.contains('A')))
            .map(|(s, _)| s)
            .unwrap();
        assert!(a_state.is_bold);

        // B debe NO estar en bold tras INIT
        let b_state = parsed
            .iter()
            .find(|(_, c)| matches!(c, CommandType::Text(t) if t.contains('B')))
            .map(|(s, _)| s)
            .unwrap();
        assert!(!b_state.is_bold);
    }

    #[test]
    fn codepage_cp437_decodes_extended_bytes() {
        // Ejemplo conocido de CP437 (ver docs de oem_cp): FB AC 3D AB => "b   bc =  bd"
        // En UTF-8 estos bytes no son v e1lidos y salen como U+FFFD.
        let data = [0xFB, 0xAC, 0x3D, 0xAB];

        let parsed_cp437 = parse_escpos(&data, CodePage::Cp437);
        let texts_cp437 = collect_text(&parsed_cp437);
        assert_eq!(texts_cp437, vec!["√¼=½".to_string()]);

        let parsed_utf8 = parse_escpos(&data, CodePage::Utf8Lossy);
        let texts_utf8 = collect_text(&parsed_utf8);
        assert_eq!(texts_utf8.len(), 1);
        assert_ne!(texts_utf8[0], "√¼=½");
        // En modo UTF-8 (auto), si no es UTF-8 válido cae a Windows-1252 y no debe emitir U+FFFD.
        assert!(!texts_utf8[0].contains('\u{FFFD}'));
    }

    #[test]
    fn codepage_cp850_decodes_extended_bytes() {
        // Para CP850, verificamos que se decodifica distinto a UTF-8.
        // Usamos la tabla del propio crate para definir el esperado (evita hardcodear un mapeo err f3neo).
        let data = [0x82];
        let expected = String::from_cp::<Cp850>(&data);

        let parsed_cp850 = parse_escpos(&data, CodePage::Cp850);
        let texts_cp850 = collect_text(&parsed_cp850);
        assert_eq!(texts_cp850, vec![expected.clone()]);

        let parsed_utf8 = parse_escpos(&data, CodePage::Utf8Lossy);
        let texts_utf8 = collect_text(&parsed_utf8);
        assert_eq!(texts_utf8.len(), 1);
        // En modo UTF-8 (auto), si no es UTF-8 válido cae a Windows-1252.
        assert!(!texts_utf8[0].contains('\u{FFFD}'));
        assert_ne!(texts_utf8[0], expected);
    }

    #[test]
    fn utf8_auto_fallback_decodes_inverted_exclamation_from_cp1252() {
        // En Windows-1252: 0xA1 = '¡'.
        // En UTF-8 estricto esto NO es válido como byte suelto.
        let data = [0xA1, b'G', b'r', b'a', b'c', b'i', b'a', b's'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        let text = collect_text(&parsed).concat();
        assert!(text.contains("¡Gracias"));
    }

    #[test]
    fn esc_t_selects_windows1252_for_subsequent_text() {
        // ESC t 16 (Windows-1252) seguido de 0xA1 ('¡') debe decodificar correctamente.
        let data = [0x1B, 0x74, 0x10, 0xA1, b'H', b'o', b'l', b'a'];
        // Arrancamos con un codepage distinto para asegurar que el cambio ocurre.
        let parsed = parse_escpos(&data, CodePage::Cp437);
        let text = collect_text(&parsed).concat();
        assert!(text.contains("¡Hola"));
        assert!(parsed.iter().any(|(_, c)| matches!(
            c,
            CommandType::Control(Control::CodePage(CodePage::Windows1252))
        )));
    }

    #[test]
    fn parses_raster_image_gs_v_0() {
        // GS v 0 m xL xH yL yH d...
        // 1 byte por fila => 8 pixeles de ancho, 1 fila.
        // 0x80: primer pixel negro.
        let data = [0x1D, 0x76, 0x30, 0x00, 0x01, 0x00, 0x01, 0x00, 0x80];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);

        assert!(parsed.iter().any(|(_, c)| match c {
            CommandType::Control(Control::RasterImage {
                m: _,
                width_bytes,
                height,
                data,
            }) => *width_bytes == 1 && *height == 1 && data.len() == 1,
            _ => false,
        }));
    }

    #[test]
    fn parses_qr_gs_paren_k_store_and_print() {
        // Secuencia t pica:
        // - Set model (31 41)
        // - Set size (31 43)
        // - Set ecc (31 45)
        // - Store data (31 50)
        // - Print (31 51)
        let mut bytes = Vec::new();

        // Model 2
        bytes.extend_from_slice(&[0x1D, 0x28, 0x6B, 0x04, 0x00, 0x31, 0x41, 0x32, 0x00]);
        // Module size 4
        bytes.extend_from_slice(&[0x1D, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x43, 0x04]);
        // ECC L (48)
        bytes.extend_from_slice(&[0x1D, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x45, 0x30]);
        // Store "HI"
        bytes.extend_from_slice(&[0x1D, 0x28, 0x6B, 0x05, 0x00, 0x31, 0x50, 0x30, b'H', b'I']);
        // Print
        bytes.extend_from_slice(&[0x1D, 0x28, 0x6B, 0x03, 0x00, 0x31, 0x51, 0x30]);

        let parsed = parse_escpos(&bytes, CodePage::Utf8Lossy);
        assert!(parsed.iter().any(|(_, c)| match c {
            CommandType::Control(Control::Qr { data, .. }) => data == b"HI",
            _ => false,
        }));
    }

    #[test]
    fn gs_bang_size_0x10_is_double_height_not_double_width() {
        // GS ! 0x10 => height x2, width x1.
        let data = [0x1D, 0x21, 0x10, b'A'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);

        let a_state = parsed
            .iter()
            .find(|(_, c)| matches!(c, CommandType::Text(t) if t.contains('A')))
            .map(|(s, _)| s)
            .unwrap();

        assert_eq!(a_state.char_width_mul, 1);
        assert_eq!(a_state.char_height_mul, 2);
    }

    #[test]
    fn gs_h_parameter_is_consumed_not_emitted_as_text() {
        // Algunos sistemas mandan GS H '2' (ASCII) y no queremos ver un "2" impreso.
        let data = [0x1D, 0x48, b'2', b'A'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        let texts = collect_text(&parsed).concat();
        assert!(!texts.contains('2'));
        assert!(texts.contains('A'));
    }

    #[test]
    fn esc_dollar_sets_absolute_position() {
        // ESC $ 100 0 (posición 100 dots)
        let data = [0x1B, 0x24, 0x64, 0x00, b'A'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        assert!(parsed.iter().any(|(s, _)| s.cursor_x == Some(100)));
        assert!(parsed.iter().any(|(_, c)| matches!(
            c,
            CommandType::Control(Control::AbsolutePosition { x: 100 })
        )));
    }

    #[test]
    fn esc_minus_enables_underline() {
        let data = [0x1B, 0x2D, 0x01, b'A'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        let a_state = parsed
            .iter()
            .find(|(_, c)| matches!(c, CommandType::Text(t) if t.contains('A')))
            .map(|(s, _)| s)
            .unwrap();
        assert!(a_state.is_underline);
    }

    #[test]
    fn gs_b_enables_reverse() {
        let data = [0x1D, 0x42, 0x01, b'A'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        let a_state = parsed
            .iter()
            .find(|(_, c)| matches!(c, CommandType::Text(t) if t.contains('A')))
            .map(|(s, _)| s)
            .unwrap();
        assert!(a_state.is_reverse);
    }

    #[test]
    fn esc_bang_master_select_bold_and_double_height() {
        // ESC ! 0x18 = bold (bit 3) + double height (bit 4)
        let data = [0x1B, 0x21, 0x18, b'A'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        let a_state = parsed
            .iter()
            .find(|(_, c)| matches!(c, CommandType::Text(t) if t.contains('A')))
            .map(|(s, _)| s)
            .unwrap();
        assert!(a_state.is_bold);
        assert_eq!(a_state.char_height_mul, 2);
    }

    #[test]
    fn esc_3_sets_line_spacing() {
        let data = [0x1B, 0x33, 0x30]; // ESC 3 48
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        assert!(parsed.iter().any(|(s, _)| s.line_spacing == Some(48)));
    }

    #[test]
    fn esc_star_parses_bit_image() {
        // ESC * mode=0, width=2, data=[0x80, 0x40]
        let data = [0x1B, 0x2A, 0x00, 0x02, 0x00, 0x80, 0x40];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        assert!(parsed.iter().any(|(_, c)| matches!(
            c,
            CommandType::Control(Control::BitImage {
                mode: 0,
                width: 2,
                ..
            })
        )));
    }

    #[test]
    fn esc_m_selects_font_b() {
        let data = [0x1B, 0x4D, 0x01, b'A', 0x1B, 0x4D, 0x00, b'B'];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        
        let a_state = parsed
            .iter()
            .find(|(_, c)| matches!(c, CommandType::Text(t) if t.contains('A')))
            .map(|(s, _)| s)
            .unwrap();
        assert!(a_state.is_font_b);

        let b_state = parsed
            .iter()
            .find(|(_, c)| matches!(c, CommandType::Text(t) if t.contains('B')))
            .map(|(s, _)| s)
            .unwrap();
        assert!(!b_state.is_font_b);
    }

    #[test]
    fn esc_p_opens_drawer() {
        let data = [0x1B, 0x70, 0x00, 0x19, 0xFA];
        let parsed = parse_escpos(&data, CodePage::Utf8Lossy);
        assert!(parsed.iter().any(|(_, c)| matches!(
            c,
            CommandType::Control(Control::OpenDrawer)
        )));
    }
}
