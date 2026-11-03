#!/usr/bin/env bash
# Build a Windows release: portable zip + NSIS installer, cross-compiled on Linux.
# Needs: rustup target x86_64-pc-windows-gnu, mingw-w64, makensis (apt install nsis), python3.
# Usage: scripts/build-release.sh            (outputs into $OUT_DIR, default dist-experimental/)
set -euo pipefail
cd "$(dirname "$0")/.."

# ---- edition identity (this branch: experimental; stable would use its own values and dist/)
APP_NAME="FFmpeg Encoder (Experimental)"
APP_ID="FFmpegEncoderExperimental"            # Add/Remove Programs key; differs from stable
APP_DIRNAME="FFmpeg Encoder Experimental"     # install folder name
FILE_STEM="FFmpegEncoder-Experimental"        # exe / zip / setup names
DATA_DIR="FFmpegEncoder-Experimental"         # %APPDATA% / %LOCALAPPDATA% folder (src/lib.rs DATA_DIR_NAME)
PUBLISHER="FFmpeg Encoder"
OUT_DIR="${OUT_DIR:-dist-experimental}"
TARGET=x86_64-pc-windows-gnu

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# 0.6.0-dev.1 -> 0.6.0.1 (Windows needs four numbers)
VERSION4=$(python3 -c "import re,sys; v=sys.argv[1]; m=re.match(r'(\d+)\.(\d+)\.(\d+)(?:-[a-z]+\.(\d+))?',v); print('.'.join(x or '0' for x in m.groups()))" "$VERSION")
EXE="$FILE_STEM.exe"

echo "== $APP_NAME $VERSION ($VERSION4)"
grep -q "pub const DATA_DIR_NAME: &str = \"$DATA_DIR\"" src/lib.rs || { echo "DATA_DIR differs from src/lib.rs" >&2; exit 1; }
cargo build --release --target "$TARGET"
BIN="target/$TARGET/release/$EXE"
[ -f "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }

mkdir -p "$OUT_DIR"
cp "$BIN" "$OUT_DIR/$EXE"

# ---- portable zip (exe + docs; settings are created next to the exe on first start)
ZIP="$OUT_DIR/$FILE_STEM-$VERSION-win64.zip"
rm -f "$ZIP"
python3 - "$ZIP" "$OUT_DIR/$EXE" "$EXE" <<'PY'
import sys, zipfile
z, exe, name = sys.argv[1:4]
with zipfile.ZipFile(z, "w", zipfile.ZIP_DEFLATED) as f:
    f.write(exe, name)
    f.write("README.md", "README.md")
    f.write("NOTES.md", "NOTES.md")
PY

# ---- installer
command -v makensis >/dev/null || { echo "makensis not found (apt install nsis)" >&2; exit 1; }
SETUP="$OUT_DIR/$FILE_STEM-$VERSION-setup.exe"
makensis -V2 -INPUTCHARSET UTF8 \
  -DAPP_NAME="$APP_NAME" -DAPP_ID="$APP_ID" -DAPP_DIRNAME="$APP_DIRNAME" -DAPP_EXE="$EXE" \
  -DAPP_VERSION="$VERSION" -DVERSION4="$VERSION4" -DPUBLISHER="$PUBLISHER" -DDATA_DIR="$DATA_DIR" \
  -DMARKER="$FILE_STEM.installed" -DICON="$PWD/assets/app.ico" -DEXE_PATH="$PWD/$BIN" -DSRC_DIR="$PWD" \
  -DOUTFILE="$PWD/$SETUP" installer/installer.nsi

( cd "$OUT_DIR" && sha256sum "$(basename "$ZIP")" "$(basename "$SETUP")" > "$FILE_STEM-$VERSION-SHA256SUMS.txt" )
ls -la "$OUT_DIR"
