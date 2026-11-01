# FFmpeg Encoder

Portable Windows GUI (Rust + egui) that replaces the PowerShell script v22.0.
One `FFmpegEncoder.exe`, no installer, no runtime. Needs `ffmpeg.exe` + `ffprobe.exe`
(the app can download them for you).

## Run
1. Copy `FFmpegEncoder.exe` anywhere (e.g. `C:\Apps\FFmpegEncoder\`) and run it.
2. If FFmpeg isn't found you get a prompt: **Download newest FFmpeg**, **Browse to existing folder…**, or **Later**.
3. Pick a mode on the left, fill the form (Browse… or drag-and-drop files onto the window), press **Preview & run…**.
   The preview shows the exact ffmpeg command(s) and any overwrite warnings; **Run** queues them.
4. The queue shows progress per job and can cancel one or all. Cancel sends `q` to ffmpeg, kills it after 4 s,
   and deletes the partial output.

FFmpeg lookup order: Settings folder (default `C:\ffmpeg\bin`) → `ffmpeg\` or `ffmpeg\bin\` next to
the exe → app-managed copy `%LOCALAPPDATA%\FFmpegEncoder\ffmpeg\bin` → `PATH`.
On start the app checks the build has `libx264` and `aac`.

## Modes
| # | Mode | What it does |
|---|------|--------------|
| 1 | Re-encode video | libx264 + AAC. Trim start/end, duration override, resize (keeps aspect, even dims), full→limited range + BT.709 tags, bwdif deinterlace (auto when interlaced), gain + compressor (light/medium/strong/custom), chapters (off / every N min / manual list / both), passthrough video or audio, faststart. Keeps all audio tracks with titles/languages, subtitles, attachments where the container allows. |
| 2 | MP4 from still image | Image + audio → H.264 MP4 (tune stillimage, RGB→BT.709 limited). Length = audio (`-shortest`) or fixed seconds. |
| 3 | Add commentary track | Copies video + all original audio, adds a WAV/audio file as a new AAC track named “Commentary Track” (configurable). |
| 4 | Extract audio to WAV | Picks a track; PCM s16le, original sample rate (fallback 48000), stereo. |
| 5 | Replace primary audio | Video copied, audio replaced by a WAV encoded to AAC, optional gain/compressor. |
| 6 | Mix audio tracks (MKV) | amix of ≥2 tracks into one AAC track; video copied; chapters kept. |
| 7 | Batch re-encode folder | Scans a folder, probes every file, include/exclude per file, same options as mode 1, per-file progress + summary. |
| 8 | Copy subtitle tracks | Re-mux or re-encode with chosen subtitle tracks from a second file; MP4 converts text subs to mov_text (or try copy). |
| 9 | Remux MKV → MP4 | Stream copy, never overwrites, optional delete of source only after the MP4 verifies; summary in the script's wording. |

## Settings
Stored as JSON. Portable first: `FFmpegEncoder.settings.json` next to the exe if that folder is writable,
otherwise `%APPDATA%\FFmpegEncoder\settings.json`. Logs go to `logs\` next to the settings file
(app log rotates at 5 MB). **Help › Open settings file / Open logs folder** jump there.
File › Settings… edits the defaults (script values: CRF 22, preset slow, AAC 192k, chapters 5 min, compressor
-18 dB / 2:1 / knee 4 / +2 dB / 20 ms / 250 ms, commentary name, fallback 48000 Hz, input `D:\Videos\Inbox`,
output `D:\Videos\Out`). Each mode remembers its last values; Settings has “reset” buttons.

Per-job ffmpeg stderr goes to `ffmpeg_error_<name>.log` in the output folder; it is deleted on success unless
“keep logs on success” is ticked.

## FFmpeg updates (Help › Check for FFmpeg updates…)
- **Sources:** BtbN (GitHub, win64 GPL, default) or gyan.dev (essentials). **Channels:** Stable (latest release branch,
  e.g. n9.0 / 9.0.2) or Latest (git master).
- Shows installed vs newest, links to release notes and build page.
- Downloads over HTTPS (rustls), verifies SHA-256 (BtbN: GitHub asset digest; gyan: published `.sha256`), extracts
  only `ffmpeg.exe`/`ffprobe.exe`/`ffplay.exe` (zip or 7z), tests the new ffmpeg has libx264+aac, then swaps
  files atomically with restore-on-failure.
- Default target is the app-managed folder (the Settings path is switched to it). Updating your configured folder
  in place is optional and needs a confirmation tick. Blocked while jobs run.
- The previous build is kept in `<bin>.previous`; **Help › Roll back FFmpeg** swaps back (one generation).
- Optional silent daily check (Settings) shows a banner when something newer exists. Nothing installs without a click.
- **Check for app updates…** is a stub for now.

## Build
```
# Linux cross-compile (what produced dist/):
sudo apt install gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
# or natively on Windows with the MSVC toolchain:
cargo build --release
```
Tests: `cargo test` (unit + integration; integration needs `ffmpeg`/`ffprobe` on PATH).
Live network tests: `cargo test --lib live -- --ignored` (`FFENC_LIVE=btbn-stable|btbn-latest|gyan-stable|gyan-latest`).

## Code layout
- `src/cmd/` – pure argument builders, one file per mode (no UI, unit-tested). Args are `Vec<String>`, never a shell string.
- `src/ffmpeg/` – locate, probe (ffprobe JSON), version/encoder check, progress parsing, job queue/runner.
- `src/updater/` – source lookup (BtbN/gyan), download+verify, extract, install/rollback.
- `src/ui/` – egui app, dialogs, updater window; `src/ui/modes/` – one `Mode` per screen.

## Adding a mode
1. Add `src/cmd/<name>.rs` with a params struct and `build(&Params) -> Result<BuiltCommand, String>`; unit-test the args.
2. Add `src/ui/modes/<name>.rs` with a struct implementing `Mode`
   (`key`, `title`, `description`, `ui`, `build -> Plan`, `on_drop`, `save`/`load` for remembered values).
   Use helpers in `src/ui/common.rs` (path rows, probe slot, encoder widgets, output naming/overwrite checks).
3. Register it in `all_modes()` in `src/ui/modes/mod.rs`. Optionally add a case to `tests/integration_ffmpeg.rs`.
