#!/usr/bin/env bash
set -euo pipefail

# Build script for Linux release artifact.
# Produces: packaging/release/escpos-viewer-<version>-x86_64-linux.tar.gz

VERSION="${1:-$(git describe --tags --always 2>/dev/null || echo "dev")}"
RELEASE_DIR="packaging/release/escpos-viewer-${VERSION}-x86_64-linux"

echo "Building escpos-viewer v${VERSION} for Linux..."

# 1. Build release binary
cargo build --release

# 2. Create release directory
mkdir -p "${RELEASE_DIR}"

# 3. Copy binary
cp "target/release/escpos_viewer" "${RELEASE_DIR}/escpos-viewer"

# 4. Copy desktop file and icon
cp packaging/escpos-viewer.desktop "${RELEASE_DIR}/"
cp packaging/escpos-viewer.png "${RELEASE_DIR}/"

# 5. Copy docs
mkdir -p "${RELEASE_DIR}/docs"
cp docs/CONFIGURAR_IMPRESORA_TCP_9100.txt "${RELEASE_DIR}/docs/" 2>/dev/null || true

# 6. Copy README and license
cp README.md "${RELEASE_DIR}/"
cp LICENSE.md "${RELEASE_DIR}/" 2>/dev/null || true

# 7. Create tarball
cd packaging/release
tar czf "escpos-viewer-${VERSION}-x86_64-linux.tar.gz" "escpos-viewer-${VERSION}-x86_64-linux"
cd ../..

echo "Done: packaging/release/escpos-viewer-${VERSION}-x86_64-linux.tar.gz"
