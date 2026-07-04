#[derive(Clone, Debug)]
pub enum CommandType {
    Text(String),
    Control(Control),
    Unknown(u8),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Control {
    Newline,
    Tab, // HT (0x09) - Horizontal Tab
    Init,
    Bold(bool),
    Align(Align),
    /// Cambio de tabla de caracteres (ESC t n) interpretado a CodePage.
    CodePage(CodePage),
    /// Raw size byte as received by GS ! n.
    Size {
        raw: u8,
        width: u8,
        height: u8,
    },
    Cut,

    /// Raster bit image: GS v 0
    /// width_bytes = bytes por fila (ancho en bits = width_bytes * 8)
    RasterImage {
        m: u8,
        width_bytes: u16,
        height: u16,
        data: Vec<u8>,
    },

    /// QR generado con comandos GS ( k (Model/Size/ECC/Store/Print)
    Qr {
        model: u8,
        module_size: u8,
        ecc: u8,
        data: Vec<u8>,
    },

    /// Barcode: GS k
    Barcode {
        m: u8,
        data: Vec<u8>,
    },

    /// Configuración de barcode (HRI/alto/ancho/fuente).
    BarcodeHriPosition(BarcodeHriPosition),
    BarcodeHeight(u8),
    BarcodeModuleWidth(u8),
    BarcodeHriFont(u8),

    /// ESC $ nL nH - Posición absoluta de impresión (en puntos/dots)
    AbsolutePosition {
        x: u16,
    },
    /// ESC \ nL nH - Posición relativa de impresión
    RelativePosition {
        offset: i16,
    },

    /// ESC - n - Subrayado (0=off, 1=1dot, 2=2dot)
    Underline(bool),
    /// GS B n - Impresión invertida (blanco sobre negro)
    Reverse(bool),
    /// ESC ! n - Master select (combinación de bold, underline, size)
    MasterSelect(u8),

    /// ESC 2 - Interlineado por defecto
    LineSpacingDefault,
    /// ESC 3 n - Interlineado en puntos
    LineSpacing(u8),

    /// ESC * m nL nH d... - Bit image mode (8/24 pines legacy)
    BitImage {
        mode: u8,
        width: u16,
        data: Vec<u8>,
    },

    /// ESC M n - Seleccionar tipo de fuente (0=Font A, 1=Font B)
    FontSelect(bool),
    /// ESC p m t1 t2 - Apertura del cajón portamonedas
    OpenDrawer,

    EscUnknown(u8),
    GsUnknown(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarcodeHriPosition {
    None,
    Above,
    Below,
    Both,
}

#[derive(Clone, Debug)]
pub struct PrinterState {
    pub is_bold: bool,
    pub is_underline: bool,
    pub is_reverse: bool,
    pub is_font_b: bool,
    pub alignment: Align,
    pub font_scale: f32,
    pub char_width_mul: u8,
    pub char_height_mul: u8,

    /// Posición horizontal del cursor en puntos (dots). None = inicio de línea.
    pub cursor_x: Option<u16>,
    /// Interlineado en puntos. None = default (~30 dots).
    pub line_spacing: Option<u8>,

    pub barcode_hri: BarcodeHriPosition,
    pub barcode_height: u8,
    pub barcode_module_width: u8,
    pub barcode_hri_font: u8,
}

impl Default for PrinterState {
    fn default() -> Self {
        Self {
            is_bold: false,
            is_underline: false,
            is_reverse: false,
            is_font_b: false,
            alignment: Align::Left,
            font_scale: 1.0,
            char_width_mul: 1,
            char_height_mul: 1,

            cursor_x: None,
            line_spacing: None,

            barcode_hri: BarcodeHriPosition::None,
            // Valores típicos (pueden variar por impresora, pero sirven para preview).
            barcode_height: 80,
            barcode_module_width: 3,
            barcode_hri_font: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaperWidth {
    W58mm,
    W80mm,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodePage {
    Utf8Lossy,
    Cp437,       // n=0: USA, Standard Europe
    Cp850,       // n=2: Multilingual (Latin-1)
    Windows1252, // n=16: Windows Western European
    Pc858,       // n=19: CP850 + Euro symbol (€)
    Iso88591,    // n=6: ISO-8859-1 Latin-1
    Cp866,       // n=17: Cyrillic (Russian)
    Cp860,       // n=3: Portuguese
    Cp865,       // n=4: Nordic
}
