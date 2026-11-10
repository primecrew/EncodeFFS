# Changelog

User-facing history for the **Slideshow** edition (`slideshow`, `FFmpegEncoder-Slideshow.exe`). Developer notes are in [NOTES.md](NOTES.md).

This edition is experimental and is not merged into the stable build. It includes the stable 0.5.4 toolset plus Create Slideshow.

## 0.5.4-slideshow.3

- Larger slideshow thumbnails (Small / Medium / Large at 96 / 160 / 256 points).
- Input and output folders are remembered at the app level (File › Settings), not in presets. On first run they default to the Windows Videos folder. Switching presets or File › New does not change them. Exporting a preset omits folder paths. Existing saved folders are kept.
- Audio bitrate shows a distinct **Will encode: N kb/s** readout and a highlighted selected rate, plus a custom value.

## 0.5.4-slideshow.2

- Vertical image list with thumbnails, drag-and-drop reorder, sort, and a pop-out window. List display options are remembered.
- Transitions are additive: each image is fully visible for its display time D, then each transition adds T (`total = N×D + (N−1)×T`). Without FFmpeg's `xfade` filter, hard cuts keep display time and do not add transition time.

## 0.5.4-slideshow.1

- First Slideshow edition: Create Slideshow (images → H.264/AAC MP4 with optional audio and crossfades), portable zip only, separate data folder. First start can import stable settings (read only).

## 0.5.4 (included from stable)

- File › New (Ctrl+N), optional output frame rate, job history, top-aligned option rows, hover help.
- The updater never modifies an FFmpeg folder you installed yourself — only the app-managed copy.

## 0.5.3

- Encoding Finished popup when the queue is idle, with Go to file.
- Windows installer (NSIS) alongside the portable zip (stable edition).
- The Bold text option was removed; older settings files that still have it load fine.

## 0.5.2

- Remembers window layout (position, size, panels, open sections, dialog sizes) with Remember / Reset.
- Configurable folder for the app-managed copy of FFmpeg, with move, verify and rollback.

## 0.5.1

- **Turn off Update Check for FFMPEG** in Settings. Automatic checks run only for the app-managed copy. Help › Check for FFmpeg updates… still works.

## 0.5.0

- Frameserver preset (AviSynth / DebugMode FrameServer script).
- Built-in presets are read-only; changes are saved as a new preset. Existing customised built-ins are migrated.

## 0.4.0

- Audio & subtitles tool: add, replace or extract audio, and add, remove or extract subtitles, with a length-tolerance warning.

## 0.3.3

- Overwrite protection: keep both, overwrite, or skip, including clashes in the same queue and with inputs. Partial outputs are published atomically.

## 0.3.2

- Themes (System, Dark, Light, High contrast), text size 75–300%, monospace size, keyboard focus ring, and screen-reader names (AccessKit).

## 0.3.1

- Drag-and-drop targets throughout the UI.
- Join / stitch files preset.
- Application icon.

## 0.3.0

- Chapter creator (FFMETADATA1, detect / auto-create, import, validation).
- HandBrake-style preset panel (search, favorites, folders, run).
- Source info panel from ffprobe.

## 0.2.0

- Modes became named, editable presets (built-ins plus a Custom pipeline type, import and export).
- First-run dialog to locate FFmpeg, or download a build, or skip for now.

## 0.1.0

- First public Windows GUI (Rust + egui) for these FFmpeg jobs, with an FFmpeg updater.
