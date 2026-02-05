#!/usr/bin/env bash
set -euo pipefail

APP_NAME="orbitshell"
BIN_SRC="$(pwd)/target/release/orbitshell"
BIN_DEST="${HOME}/.local/bin/orbitshell"
DESKTOP_DIR="${HOME}/.local/share/applications"
DESKTOP_FILE="${DESKTOP_DIR}/orbitshell.desktop"

if [[ ! -f "${BIN_SRC}" ]]; then
  echo "Missing ${BIN_SRC}. Build first: cargo build --release"
  exit 1
fi

mkdir -p "${HOME}/.local/bin"
mkdir -p "${DESKTOP_DIR}"

install -m 755 "${BIN_SRC}" "${BIN_DEST}"

cat > "${DESKTOP_FILE}" <<EOF
[Desktop Entry]
Name=OrbitShell
Exec=${BIN_DEST}
Type=Application
Terminal=false
Categories=Utility;Development;
EOF

echo "Installed OrbitShell to ${BIN_DEST}"
