#!/usr/bin/env bash
# Build a Windows release: portable zip + NSIS installer, cross-compiled on Linux.
# Needs: rustup target x86_64-pc-windows-gnu, mingw-w64, makensis (apt install nsis), python3.
# Usage: scripts/build-release.sh            (outputs into $OUT_DIR, default dist/)
set -euo pipefail
cd "$(dirname "$0")/.."

# ---- edition identity (stable edition, master)
APP_NAME="Encode FFS"
APP_ID="FFmpegEncoder"                        # Add/Remove Programs key; the experimental edition uses FFmpegEncoderExperimental
APP_DIRNAME="Encode FFS"                    # fresh-install folder; upgrades keep the existing InstallLocation
ARTIFACT_STEM="EncodeFFS"                    # zip / setup names (no spaces): EncodeFFS-v<version>-...
MARKER="FFmpegEncoder.installed"             # keep the old marker so existing installs are still recognised
FILE_STEM="FFmpegEncoder"                    # crate bin / portable settings prefix (unchanged)
DATA_PUBLISHER="PrimeCrew Software"
DATA_DIR="Encode FFS"                       # leaf under %APPDATA%/%LOCALAPPDATA%\$DATA_PUBLISHER (src/lib.rs DATA_DIR_NAME)
LEGACY_DATA_DIR="FFmpegEncoder"              # pre-0.6.0-rc.3 data folder
PUBLISHER="PrimeCrew Software"
PREV_APP_NAME="FFmpeg Encoder"          # original Start Menu / desktop shortcut; removed on upgrade
PREV_BRAND_APP_NAME="FFS Just Encode"   # 0.6.0-rc.3 visible name; brand-folder and root shortcuts removed on upgrade
OUT_DIR="${OUT_DIR:-dist}"
INSTALLER="${INSTALLER:-1}"
TARGET=x86_64-pc-windows-gnu

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# 0.6.0-dev.1 -> 0.6.0.1 (Windows needs four numbers)
VERSION4=$(python3 -c "import re,sys; v=sys.argv[1]; m=re.match(r'(\d+)\.(\d+)\.(\d+)(?:-[a-z]+\.(\d+))?',v); print('.'.join(x or '0' for x in m.groups()))" "$VERSION")
# Cargo still emits FFmpegEncoder.exe (the crate bin name is unchanged so Linux cargo test keeps a simple name).
CARGO_EXE="$FILE_STEM.exe"
APP_EXE="Encode FFS.exe"
OLD_APP_EXE="FFmpegEncoder.exe"

case "$OUT_DIR" in
  /*) ;;
  *) OUT_DIR="$PWD/$OUT_DIR" ;;
esac

echo "== $APP_NAME $VERSION ($VERSION4)"
grep -q "pub const DATA_DIR_NAME: &str = \"$DATA_DIR\"" src/lib.rs || { echo "DATA_DIR differs from src/lib.rs" >&2; exit 1; }
grep -q "pub const LEGACY_DATA_DIR_NAME: &str = \"$LEGACY_DATA_DIR\"" src/lib.rs || { echo "LEGACY_DATA_DIR differs from src/lib.rs" >&2; exit 1; }
grep -q "pub const PUBLISHER: &str = \"$DATA_PUBLISHER\"" src/lib.rs || { echo "DATA_PUBLISHER differs from src/lib.rs" >&2; exit 1; }
grep -q "pub const EXE_FILE: &str = \"$APP_EXE\"" src/lib.rs || { echo "APP_EXE differs from src/lib.rs EXE_FILE" >&2; exit 1; }
grep -q "pub const LEGACY_EXE_FILE: &str = \"$OLD_APP_EXE\"" src/lib.rs || { echo "OLD_APP_EXE differs from src/lib.rs LEGACY_EXE_FILE" >&2; exit 1; }
# Don't embed this machine's paths in panic messages (release only)
src_root="$(pwd)"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=${src_root}=. --remap-path-prefix=${cargo_home}=cargo"
cargo build --release --target "$TARGET"
BIN="target/$TARGET/release/$CARGO_EXE"
[ -f "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }

# ---- offline user guide (docs/*.md -> HTML + images; Help › User Guide opens docs\index.html next to the exe)
DOCS_HTML="$PWD/target/guide-html"
rm -rf "$DOCS_HTML"
python3 scripts/docs_to_html.py docs "$DOCS_HTML"

mkdir -p "$OUT_DIR"
rm -f "$OUT_DIR/$OLD_APP_EXE"
cp "$BIN" "$OUT_DIR/$APP_EXE"

# ---- portable zip (exe + README/NOTES + the user guide in docs\; settings are created next to the exe on first start)
ZIP="$OUT_DIR/$ARTIFACT_STEM-v$VERSION-win64.zip"
rm -f "$ZIP"
python3 - "$ZIP" "$OUT_DIR/$APP_EXE" "$APP_EXE" "$DOCS_HTML" <<'PY'
import os, sys, zipfile
z, exe, name, docs = sys.argv[1:5]
with zipfile.ZipFile(z, "w", zipfile.ZIP_DEFLATED) as f:
    f.write(exe, name)
    f.write("README.md", "README.md")
    f.write("NOTES.md", "NOTES.md")
    f.write("LICENSE", "LICENSE")
    f.write("COPYRIGHT", "COPYRIGHT")
    for root, _, files in sorted(os.walk(docs)):
        for n in sorted(files):
            p = os.path.join(root, n)
            f.write(p, "docs/" + os.path.relpath(p, docs).replace(os.sep, "/"))
PY

# ---- installer
if [ "$INSTALLER" != 1 ]; then
  ( cd "$OUT_DIR" && sha256sum "$(basename "$ZIP")" > "$ARTIFACT_STEM-v$VERSION-SHA256SUMS.txt" )
  ls -la "$OUT_DIR"
  exit 0
fi
command -v makensis >/dev/null || { echo "makensis not found (apt install nsis)" >&2; exit 1; }
SETUP="$OUT_DIR/$ARTIFACT_STEM-v$VERSION-setup.exe"
makensis -V2 -INPUTCHARSET UTF8 \
  -DAPP_NAME="$APP_NAME" -DAPP_ID="$APP_ID" -DAPP_DIRNAME="$APP_DIRNAME" -DAPP_EXE="$APP_EXE" -DOLD_APP_EXE="$OLD_APP_EXE" \
  -DAPP_VERSION="$VERSION" -DVERSION4="$VERSION4" -DPUBLISHER="$PUBLISHER" -DPREV_APP_NAME="$PREV_APP_NAME" -DPREV_BRAND_APP_NAME="$PREV_BRAND_APP_NAME" -DDATA_PUBLISHER="$DATA_PUBLISHER" -DDATA_DIR="$DATA_DIR" -DLEGACY_DATA_DIR="$LEGACY_DATA_DIR" \
  -DMARKER="$MARKER" -DICON="$PWD/assets/app.ico" -DEXE_PATH="$OUT_DIR/$APP_EXE" -DSRC_DIR="$PWD" -DDOCS_DIR="$DOCS_HTML" \
  -DOUTFILE="$SETUP" installer/installer.nsi

( cd "$OUT_DIR" && sha256sum "$(basename "$ZIP")" "$(basename "$SETUP")" > "$ARTIFACT_STEM-v$VERSION-SHA256SUMS.txt" )
ls -la "$OUT_DIR"
