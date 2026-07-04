#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
#  install.sh — escpos-viewer: verificación, build e instalación
# =============================================================================
#  Uso:
#    ./install.sh                    → build + test + instala en /usr/local
#    ./install.sh --prefix /opt      → instalación personalizada
#    ./install.sh --check-only       → solo verifica prerequisitos
#    ./install.sh --help             → esta ayuda
#
#  Requiere: Rust (cargo, rustc), Linux con las dependencias del sistema.
# =============================================================================

VERSION="1.7.0"
PREFIX="${1:-/usr/local}"  # fallback para --prefix

# --- Funciones auxiliares ---------------------------------------------------

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

pass()  { echo -e "  ${GREEN}✅${NC} $1"; }
warn()  { echo -e "  ${YELLOW}⚠️${NC} $1"; }
fail()  { echo -e "  ${RED}❌${NC} $1"; }
info()  { echo -e "  ${CYAN}•${NC} $1"; }
header(){ echo -e "\n${CYAN}══════════════════════════════════════════════════${NC}"; }
subheader(){ echo -e "${CYAN}---${NC} $1"; }

# --- Parseo de argumentos ---------------------------------------------------

CHECK_ONLY=false
SHOW_HELP=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check-only)    CHECK_ONLY=true; shift ;;
    --prefix)        PREFIX="$2"; shift 2 ;;
    --help|-h)       SHOW_HELP=true; shift ;;
    *)               PREFIX="$1"; shift ;;  # primer positional = prefix
  esac
done

if $SHOW_HELP; then
  sed -n '3,11p' "$0"
  exit 0
fi

BINDIR="${PREFIX}/bin"
DATADIR="${PREFIX}/share"

# =============================================================================
#  FASE 1 — Verificación de prerequisitos
# =============================================================================

header
echo -e "  ${CYAN}escpos-viewer v${VERSION} — Verificación de prerequisitos${NC}"
header

ALL_OK=true

# --- Rust toolchain ----------------------------------------------------------
subheader "Rust toolchain"

if command -v cargo &>/dev/null; then
  CARGO_VER=$(cargo --version | head -1)
  pass "cargo encontrado: ${CARGO_VER}"
else
  fail "cargo no encontrado. Instalalo via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  ALL_OK=false
fi

if command -v rustc &>/dev/null; then
  RUSTC_VER=$(rustc --version | head -1)
  pass "rustc encontrado: ${RUSTC_VER}"
else
  fail "rustc no encontrado"
  ALL_OK=false
fi

# --- Dependencias del sistema (Linux) ----------------------------------------
subheader "Dependencias del sistema"

# tray-icon necesita una lib de app indicator
APP_INDICATOR_FOUND=false
for pkg in libayatana-appindicator3-dev libappindicator3-dev; do
  if dpkg -s "$pkg" &>/dev/null 2>&1; then
    pass "sistema: ${pkg} instalado"
    APP_INDICATOR_FOUND=true
    break
  fi
done

if ! $APP_INDICATOR_FOUND; then
  warn "No se encontró libappindicator. El tray icon puede fallar."
  warn "  Instalá con: sudo apt install libayatana-appindicator3-dev"
  warn "  (requerido por tray-icon en Linux)"
fi

# Otras deps comunes que puede necesitar eframe/egui
for pkg in libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev \
           libxcb-xinput-dev libxcb-randr0-dev libxcb-xinerama0-dev \
           libxkbcommon-x11-dev; do
  if dpkg -s "$pkg" &>/dev/null 2>&1; then
    pass "sistema: ${pkg} instalado"
  fi
done

# --- desktop-file-validate (opcional) ----------------------------------------
if command -v desktop-file-validate &>/dev/null; then
  pass "desktop-file-validate disponible"
else
  warn "desktop-file-validate no instalado (opcional, solo para validar .desktop)"
fi

# --- Verificación de archivos del proyecto -----------------------------------
subheader "Archivos del proyecto"

EXPECTED_FILES=(
  "Cargo.toml"
  "src/main.rs"
  "packaging/escpos-viewer.desktop"
  "packaging/escpos-viewer.png"
  "packaging/Makefile"
  "packaging/linux-release.sh"
)

PROJECT_ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$PROJECT_ROOT"

for f in "${EXPECTED_FILES[@]}"; do
  if [[ -f "$f" ]]; then
    pass "archivo: ${f}"
  else
    warn "archivo faltante: ${f} (puede no ser crítico)"
  fi
done

# --- Resumen de verificación -------------------------------------------------
header
if $ALL_OK; then
  echo -e "  ${GREEN}Verificación completada: todo OK.${NC}"
else
  echo -e "  ${RED}Hay problemas que resolver antes de continuar.${NC}"
fi
header

if $CHECK_ONLY; then
  $ALL_OK && exit 0 || exit 1
fi

if ! $ALL_OK; then
  echo -e "\n  Corregí los errores marcados con ❌ y volvé a ejecutar.\n"
  exit 1
fi

# =============================================================================
#  FASE 2 — Build
# =============================================================================

header
echo -e "  ${CYAN}Fase 2 — Build${NC}"
header

info "Ejecutando: cargo build..."
cargo build 2>&1 | tail -5
pass "cargo build completado"

info "Ejecutando: cargo build --release..."
cargo build --release 2>&1 | tail -5
pass "cargo build --release completado"

# =============================================================================
#  FASE 3 — Tests
# =============================================================================

header
echo -e "  ${CYAN}Fase 3 — Tests${NC}"
header

info "Ejecutando: cargo test..."
cargo test 2>&1 | tail -10
pass "cargo test completado"

# =============================================================================
#  FASE 4 — Lint (advertencias, no blocking)
# =============================================================================

header
echo -e "  ${CYAN}Fase 4 — Lint (clippy)${NC}"
header

if cargo clippy 2>&1; then
  pass "cargo clippy: sin warnings"
else
  warn "cargo clippy reportó warnings (revisar si es necesario)"
fi

# =============================================================================
#  FASE 5 — Instalación
# =============================================================================

header
echo -e "  ${CYAN}Fase 5 — Instalación en ${PREFIX}${NC}"
header

if [[ ! -w "$PREFIX" ]] && [[ "$PREFIX" == "/usr/local" ]]; then
  warn "No tenés permisos de escritura en ${PREFIX}. Usando sudo..."
  SUDO="sudo"
else
  SUDO=""
fi

$SUDO install -d "${BINDIR}"
$SUDO install -d "${DATADIR}/applications"
$SUDO install -d "${DATADIR}/icons/hicolor/256x256/apps"

$SUDO install -m 755 "target/release/escpos_viewer" "${BINDIR}/escpos-viewer"
pass "binario instalado en ${BINDIR}/escpos-viewer"

if [[ -f "packaging/escpos-viewer.desktop" ]]; then
  $SUDO install -m 644 "packaging/escpos-viewer.desktop" "${DATADIR}/applications/"
  pass ".desktop instalado en ${DATADIR}/applications/"
fi

if [[ -f "packaging/escpos-viewer.png" ]]; then
  $SUDO install -m 644 "packaging/escpos-viewer.png" "${DATADIR}/icons/hicolor/256x256/apps/"
  pass "icono instalado en ${DATADIR}/icons/hicolor/256x256/apps/"
fi

# Refrescar base de datos de aplicaciones (si existe)
if command -v update-desktop-database &>/dev/null; then
  $SUDO update-desktop-database &>/dev/null || true
  pass "base de datos de aplicaciones actualizada"
fi

# =============================================================================
#  Resumen final
# =============================================================================

header
echo -e "  ${GREEN}✔ escpos-viewer v${VERSION} instalado correctamente${NC}"
header
echo ""
echo -e "  Ejecutalo con:  ${CYAN}escpos-viewer${NC}"
echo ""
echo -e "  Para desinstalar:"
echo -e "    ${YELLOW}sudo rm -f ${BINDIR}/escpos-viewer${NC}"
echo -e "    ${YELLOW}sudo rm -f ${DATADIR}/applications/escpos-viewer.desktop${NC}"
echo -e "    ${YELLOW}sudo rm -f ${DATADIR}/icons/hicolor/256x256/apps/escpos-viewer.png${NC}"
echo ""
header
