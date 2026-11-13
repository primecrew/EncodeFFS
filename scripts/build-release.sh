#!/usr/bin/env bash
# Build a Windows release: portable zip + NSIS installer, cross-compiled on Linux.
# Needs: rustup target x86_64-pc-windows-gnu, mingw-w64, makensis (apt install nsis), python3.
# Usage: scripts/build-release.sh            (outputs into $OUT_DIR, default dist/)
set -euo pipefail
cd "$(dirname "$0")/.."

# ---- edition identity (codecs edition, branch `codecs`; portable zip only by default)
APP_NAME="Encode FFS (Codecs)"
APP_ID="FFmpegEncoderCodecs"                  # Add/Remove Programs key (only if INSTALLER=1)
APP_DIRNAME="FFmpeg Encoder (Codecs)"        # install folder name
FILE_STEM="FFmpegEncoder-Codecs"              # exe / zip / setup names
DATA_PUBLISHER="PrimeCrew Software"
DATA_DIR="Encode FFS (Codecs)"              # leaf under %APPDATA%/%LOCALAPPDATA%\$DATA_PUBLISHER
LEGACY_DATA_DIR="FFmpegEncoder-Codecs"       # pre-rebrand data folder
PUBLISHER="PrimeCrew Software"
PREV_APP_NAME="FFmpeg Encoder (Codecs)" # original Start Menu / desktop shortcut; removed on upgrade
PREV_BRAND_APP_NAME="FFS Just Encode (Codecs)"  # previous visible name; brand-folder and root shortcuts removed on upgrade
OUT_DIR="${OUT_DIR:-dist-codecs}"
INSTALLER="${INSTALLER:-0}"
TARGET=x86_64-pc-windows-gnu

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# 0.6.0-dev.1 -> 0.6.0.1 (Windows needs four numbers)
VERSION4=$(python3 -c "import re,sys; v=sys.argv[1]; m=re.match(r'(\d+)\.(\d+)\.(\d+)(?:-[a-z]+\.(\d+))?',v); print('.'.join(x or '0' for x in m.groups()))" "$VERSION")
EXE="$FILE_STEM.exe"

echo "== $APP_NAME $VERSION ($VERSION4)"
grep -q "pub const DATA_DIR_NAME: &str = \"$DATA_DIR\"" src/lib.rs || { echo "DATA_DIR differs from src/lib.rs" >&2; exit 1; }
grep -q "pub const LEGACY_DATA_DIR_NAME: &str = \"$LEGACY_DATA_DIR\"" src/lib.rs || { echo "LEGACY_DATA_DIR differs from src/lib.rs" >&2; exit 1; }
grep -q "pub const PUBLISHER: &str = \"$DATA_PUBLISHER\"" src/lib.rs || { echo "DATA_PUBLISHER differs from src/lib.rs" >&2; exit 1; }
# Don't embed this machine's paths in panic messages (release only)
src_root="$(pwd)"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=${src_root}=. --remap-path-prefix=${cargo_home}=cargo"
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

# ---- installer (off by default for the codecs edition)
if [ "$INSTALLER" != 1 ]; then
  ( cd "$OUT_DIR" && sha256sum "$(basename "$ZIP")" > "$FILE_STEM-$VERSION-SHA256SUMS.txt" )
  ls -la "$OUT_DIR"
  exit 0
fi
command -v makensis >/dev/null || { echo "makensis not found (apt install nsis)" >&2; exit 1; }
SETUP="$OUT_DIR/$FILE_STEM-$VERSION-setup.exe"
makensis -V2 -INPUTCHARSET UTF8 \
  -DAPP_NAME="$APP_NAME" -DAPP_ID="$APP_ID" -DAPP_DIRNAME="$APP_DIRNAME" -DAPP_EXE="$EXE" \
  -DAPP_VERSION="$VERSION" -DVERSION4="$VERSION4" -DPUBLISHER="$PUBLISHER" -DPREV_APP_NAME="$PREV_APP_NAME" -DPREV_BRAND_APP_NAME="$PREV_BRAND_APP_NAME" -DDATA_PUBLISHER="$DATA_PUBLISHER" -DDATA_DIR="$DATA_DIR" -DLEGACY_DATA_DIR="$LEGACY_DATA_DIR" \
  -DMARKER="$FILE_STEM.installed" -DICON="$PWD/assets/app.ico" -DEXE_PATH="$PWD/$BIN" -DSRC_DIR="$PWD" \
  -DOUTFILE="$PWD/$SETUP" installer/installer.nsi

( cd "$OUT_DIR" && sha256sum "$(basename "$ZIP")" "$(basename "$SETUP")" > "$FILE_STEM-$VERSION-SHA256SUMS.txt" )
ls -la "$OUT_DIR"
