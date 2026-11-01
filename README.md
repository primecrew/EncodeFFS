# FFmpeg Encoder

Portable Windows GUI (Rust + egui) that replaces the PowerShell script v22.0.
One `FFmpegEncoder.exe`, no installer, no runtime. Needs `ffmpeg.exe` + `ffprobe.exe`
(the app can download them for you).

## Run
1. Copy `FFmpegEncoder.exe` anywhere (e.g. `C:\Apps\FFmpegEncoder\`) and run it.
2. First start (and whenever no working ffmpeg/ffprobe is found) asks for the folder holding `ffmpeg.exe` and
   `ffprobe.exe`, pre-filled with `C:\ffmpeg\bin`, with Browse… and live validation (the parent of a
   `bin` folder is accepted too). Secondary buttons: **Download newest FFmpeg** and **Skip for now**.
   Later: Help › Locate FFmpeg folder… or File › Settings.
3. Pick a preset on the left, fill the form (Browse… or drag-and-drop files onto the window), press **Preview & run…**.
   The preview shows the exact ffmpeg command(s) and any overwrite warnings; **Run** queues them.
4. The queue shows progress per job and can cancel one or all. Cancel sends `q` to ffmpeg, kills it after 4 s,
   and deletes the partial output.

FFmpeg lookup order: Settings folder (default `C:\ffmpeg\bin`) → `ffmpeg\` or `ffmpeg\bin\` next to
the exe → app-managed copy `%LOCALAPPDATA%\FFmpegEncoder\ffmpeg\bin` → `PATH`.
On start the app checks the build has `libx264` and `aac`.

## Presets
Everything on the left is a **preset**: a named, saved set of options for an operation **type**. Nothing is locked.

- Ships with 10 built-ins: the 9 script modes plus **Custom pipeline**. Edit, rename, reorder, duplicate or delete any of them;
  built-ins can be **reset to factory defaults** and deleted ones restored (Presets › Restore missing built-in presets).
- Editing a form marks the preset `*` (unsaved). **Save preset** (Ctrl+S) stores it, **Revert** drops the edits,
  **Save as new…** turns the current form into a new preset (the original keeps its saved values).
- **New…** starts from a copy of any preset or a blank preset of any type. Right-click a preset for the same actions.
- Files chosen for a run (inputs, typed output names) are *not* part of a preset; they're remembered as a draft and carried
  over when you switch to another preset that takes a file.
- **Import / export** (Presets menu): JSON files with one or many presets. Imports get new ids and unique names;
  unknown types are skipped.
- Stored in `FFmpegEncoder.presets.json` next to the settings file (or `%APPDATA%\FFmpegEncoder\presets.json`).
  Built-ins that you never customised store `"values": null` = “type defaults from File › Settings”.
- Every run shows the exact ffmpeg command(s) first.

### Preset types
| Type | What it does |
|------|--------------|
| Re-encode video | libx264 + AAC. Trim start/end, duration override, resize (keeps aspect, even dims), full→limited range + BT.709 tags, bwdif deinterlace (auto when interlaced), gain + compressor (light/medium/strong/custom), chapters (off / every N min / manual list / both), passthrough video or audio, container (auto/mp4/mkv/mov), name suffix, faststart. Keeps all audio tracks with titles/languages, subtitles, attachments where the container allows. |
| MP4 from still image | Image + audio → H.264 MP4 (tune stillimage, RGB→BT.709 limited). Length = audio (`-shortest`) or fixed seconds. |
| Add commentary track | Copies video + all original audio, adds an audio file as a new AAC track named “Commentary Track” (configurable). |
| Extract audio to WAV | Picks a track; PCM s16le, original sample rate (fallback 48000), stereo. |
| Replace primary audio | Video copied, audio replaced by a WAV encoded to AAC, optional gain/compressor. |
| Mix audio tracks (MKV) | amix of ≥2 tracks into one AAC track; video copied; chapters kept. |
| Batch re-encode folder | Scans a folder, probes every file, include/exclude per file, same options as re-encode, per-file progress + summary. |
| Copy subtitle tracks | Re-mux or re-encode with chosen subtitle tracks from a second file; MP4 converts text subs to mov_text (or try copy). |
| Remux MKV → MP4 | Stream copy, never overwrites, optional delete of source only after the MP4 verifies; summary in the script's wording. |
| **Custom pipeline** | Build your own from independent choices (below). |

### Custom pipeline options
- **Input:** single file (video or audio) or every video in a folder (with per-file include).
- **Video:** copy / re-encode x264 (CRF, preset, tune, optional resize, range conversion, deinterlace) / drop.
- **Audio:** copy all / AAC all / drop / replace with a file / mix all tracks into one / copy + add a file as an extra
  track — with bitrate, gain and compressor, and an optional track title. Folder batches pick the audio file with the
  same name next to each video (`game.mkv` + `game.wav`).
- **Subtitles:** copy / convert text subs (MP4: mov_text, others: SRT) / drop / import from a file (optionally keeping
  the originals; batches use the same-name `.srt/.ass/.vtt`).
- **Trim**, **chapters** (auto/manual/both, or remove the source's), **container** (mp4, mkv, mov, m4v, m4a, mka),
  **name suffix**, **faststart**, and free-form **extra input options** (before `-i`) and **extra output options**
  (before the output file; quotes group values with spaces).

## Settings
Stored as JSON. Portable first: `FFmpegEncoder.settings.json` next to the exe if that folder is writable,
otherwise `%APPDATA%\FFmpegEncoder\settings.json`. Logs go to `logs\` next to the settings file
(app log rotates at 5 MB). **Help › Open settings file / Open logs folder** jump there.
File › Settings… edits the defaults (script values: CRF 22, preset slow, AAC 192k, chapters 5 min, compressor
-18 dB / 2:1 / knee 4 / +2 dB / 20 ms / 250 ms, commentary name, fallback 48000 Hz, input `D:\Videos\Inbox`,
output `D:\Videos\Out`). Built-in presets you haven't customised follow these defaults.

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
- `src/presets.rs` – preset file format, built-in list, import/export rules.
- `src/ui/` – egui app, dialogs, updater window; `src/ui/presets.rs` – preset list, editors, change tracking;
  `src/ui/modes/` – one form (`Mode`) per preset type.

## Adding a preset type
Usually not needed: a Custom pipeline preset covers most new workflows. For a new built-in type:
1. Add `src/cmd/<name>.rs` with a params struct and `build(&Params) -> Result<BuiltCommand, String>`; unit-test the args.
2. Add `src/ui/modes/<name>.rs` with a struct implementing `Mode`
   (`key`, `title`, `description`, `run_fields` (per-run keys not stored in presets), `ui`, `build -> Plan`,
   `on_drop`, `save`/`load`). Use helpers in `src/ui/common.rs`.
3. Register it in `all_modes()` in `src/ui/modes/mod.rs`; optionally add a factory preset to `BUILTINS` in `src/presets.rs`.
4. Optionally add a case to `tests/integration_ffmpeg.rs`.
