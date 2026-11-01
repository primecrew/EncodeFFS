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
3. Pick a preset in the **preset panel** on the left, choose a file (Browse… or drag-and-drop onto the window) and
   press **Preview & run…** — or double-click a preset / press **▶ Run** / F5. The preview shows the exact ffmpeg
   command(s) and any overwrite warnings; **Run** queues them. The **Source** panel on the right shows what ffprobe
   reports about the input.
4. The queue shows progress per job and can cancel one or all. Cancel sends `q` to ffmpeg, kills it after 4 s,
   and deletes the partial output.

FFmpeg lookup order: Settings folder (default `C:\ffmpeg\bin`) → `ffmpeg\` or `ffmpeg\bin\` next to
the exe → app-managed copy `%LOCALAPPDATA%\FFmpegEncoder\ffmpeg\bin` → `PATH`.
On start the app checks the build has `libx264` and `aac`.

## Main window (HandBrake-style)
- **Left – presets:** every preset, built-in and yours, in your order. One click selects; **double-click** (or **▶ Run**,
  F5) loads the current input into that preset and opens the command preview, from where it goes to the queue.
  `*` = unsaved changes. Quick buttons **New… / Duplicate / Import… / Export**, a **search** box (name, type, folder,
  description), **Pin** to put favourites in a "Favorites" group at the top, and optional **folders**
  (right-click › Set folder…; groups only appear once you use them). ⬆/⬇ reorder within a group; right-click has
  everything else. The **Presets** menu has the full set of actions too.
- **Middle – the selected preset's form**, with Save preset / Revert / Save as new….
- **Right – Source** (View › Source info panel): ffprobe's summary of the input — container, duration, size, bitrate,
  video codec/profile, resolution, frame rate, pixel format, interlaced/progressive (field order), every audio track
  (codec, channels, sample rate, bitrate, language, title, default), every subtitle track (language, forced), chapters
  and attachments. ffprobe must sit in the same folder as ffmpeg; the panel shows which one is used.
- **Bottom – queue** with per-job progress, cancel and logs.

## Presets
Everything in the preset panel is a **preset**: a named, saved set of options for an operation **type**. Nothing is locked.

- Ships with 11 built-ins: the 9 script modes, **Custom pipeline** and **Add chapters (no re-encode)**. Edit, rename, reorder, duplicate or delete any of them;
  built-ins can be **reset to factory defaults** and deleted ones restored (Presets › Restore missing built-in presets).
- Editing a form marks the preset `*` (unsaved). **Save preset** (Ctrl+S) stores it, **Revert** drops the edits,
  **Save as new…** turns the current form into a new preset (the original keeps its saved values).
- **New…** starts from a copy of any preset or a blank preset of any type. Right-click a preset for the same actions.
- Files chosen for a run (inputs, typed output names) are *not* part of a preset; they're remembered as a draft and carried
  over when you switch to another preset that takes a file.
- **Favorites** (`"pinned": true`) and **folders** (`"category"`) are stored with the preset and survive export/import.
- **Import / export** (panel buttons or Presets menu): JSON files with one or many presets. Imports get new ids and unique names;
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
| **Add chapters (no re-encode)** | Custom pipeline preset: copies every stream, adds chapters every 5 min (or your list), same container as the input, suffix `_chapters`. |

## Chapters & the chapter creator
Re-encode, Custom pipeline (single file and folder batch) and Batch re-encode can write chapters.
- The form shows whether ffprobe found chapters in the input ("No chapters found in this file" → **Create chapters…**).
- **Chapter creator…** opens a window with: mode (off / every N min / manual / both), the manual list (add, edit,
  delete, move, sort, clear), **set from timestamp** (paste `750.5`, `12:30`, `1:02:03.250` or `1h2m3s`),
  **import from text** — one chapter per line:
  ```
  00:00:00 Intro
  00:12:30 Boss fight
  1:02:03.250 - Credits
  ```
  (also opens a .txt or an existing FFMETADATA file), **load the file's own chapters** to edit them, and a live
  **preview** with start/end/length and the exact FFMETADATA1 file. Errors (duplicate or out-of-range times, invalid
  times, trim longer than the input) are shown before you run.
- **"If the input has no chapters, create one every N min"** (chapters off): files with chapters keep theirs; the
  others get automatic chapters. Batch re-encode has the same switch.
- Times are relative to the trimmed output. Untitled chapters are named "Chapter N".
- The chapter list is written to `%TEMP%\ffenc_chapters_<id>.ffmeta` (UTF-8, no BOM) in FFmpeg's FFMETADATA1 format:
  ```
  ;FFMETADATA1
  [CHAPTER]
  TIMEBASE=1/1000
  START=0
  END=750000
  title=Intro
  ```
  with `=`, `;`, `#`, `\` and newlines in titles escaped by `\`. It's passed as `-f ffmetadata -i <file>` and mapped
  with `-map_chapters <n>` (other metadata is untouched) and deleted after the job, also on failure/cancel.

### Custom pipeline options
- **Input:** single file (video or audio) or every video in a folder (with per-file include).
- **Video:** copy / re-encode x264 (CRF, preset, tune, optional resize, range conversion, deinterlace) / drop.
- **Audio:** copy all / AAC all / drop / replace with a file / mix all tracks into one / copy + add a file as an extra
  track — with bitrate, gain and compressor, and an optional track title. Folder batches pick the audio file with the
  same name next to each video (`game.mkv` + `game.wav`).
- **Subtitles:** copy / convert text subs (MP4: mov_text, others: SRT) / drop / import from a file (optionally keeping
  the originals; batches use the same-name `.srt/.ass/.vtt`).
- **Trim**, **chapters** (chapter creator, auto-create when missing, or remove the source's), **container** (same as input, mp4, mkv, mov, m4v, m4a, mka),
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
- `src/cmd/chapters.rs` – chapter list building, validation, FFMETADATA1 writer/parser, timestamp/list parsing, temp files.
- `src/ui/` – egui app, dialogs, updater window; `src/ui/presets.rs` – preset panel (search, favorites, folders), editors,
  change tracking; `src/ui/chapter_creator.rs` – chapter creator; `src/ui/source.rs` – Source panel;
  `src/ui/modes/` – one form (`Mode`) per preset type.

## Adding a preset type
Usually not needed: a Custom pipeline preset covers most new workflows. For a new built-in type:
1. Add `src/cmd/<name>.rs` with a params struct and `build(&Params) -> Result<BuiltCommand, String>`; unit-test the args.
2. Add `src/ui/modes/<name>.rs` with a struct implementing `Mode`
   (`key`, `title`, `description`, `run_fields` (per-run keys not stored in presets), `ui`, `build -> Plan`,
   `on_drop`, `save`/`load`). Use helpers in `src/ui/common.rs`.
3. Register it in `all_modes()` in `src/ui/modes/mod.rs`; optionally add a factory preset to `BUILTINS` in `src/presets.rs`.
4. Optionally add a case to `tests/integration_ffmpeg.rs`.
