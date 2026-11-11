# Changelog

User-facing history for the **Codecs** edition (`codecs`, `FFmpegEncoder-Codecs.exe`). Developer notes are in [NOTES.md](NOTES.md).

This edition is experimental and is not merged into the stable build. It includes the stable toolset plus optional HEVC/AV1 video and AC3 audio.

## Unreleased

- Conflict warning text wraps to the window width, and the fix button (plus “Keep my setting” or the pre-run “Run as set” footer) sits on the next line at a natural width, so the button is not squeezed into a narrow column at small windows or large text.
- Advanced compressor controls are easier to find: the Compressor drop-down uses **advanced…**, and the ratio, threshold, knee, attack, release and make-up gain fields appear in that order (after the Gain / pre-gain row) when Advanced is selected. Audio & subtitles now shows those controls under the track list. File › Settings lists them as **Advanced compressor defaults**.
- Frameserver colour matrix defaults to **PC.709** (keep full range, recommended for Vegas 8-bit). Rec709 is labelled as the squeeze-to-TV choice for 32-bit full-range Vegas projects. Saved presets keep the matrix they stored; older files that omitted the field still load as Rec709.
- Re-encode, Batch, Custom pipeline and similar forms warn (but still run) when the source is tagged as TV (limited) range and **Convert full-range to limited-range** is on. A one-click fix turns the conversion off; **Run as set** keeps it. Missing or unknown tags are not warned about (a short hover notes that the range is untagged). README has a **Colour range and Vegas frameserving** section.
- Faststart (`-movflags +faststart` on MP4/MOV) is off by default for new jobs. Saved presets keep the value they stored; older files that omitted the field still load as on. The Faststart hover text was updated.
- README has a **Choosing a codec** chapter: when to use H.264, HEVC or AV1, rough starting points, a short test-clip method, and a note on frameserved audio.

## 0.5.5-codecs.3

- Unusual option combinations (double range conversion, 10-bit with an 8-bit-only colour format, a 10-bit encoder that does not match the script, AV1 in .mov, copied audio in MP4, and similar) now offer a one-click fix next to the warning and in a pre-run popup. “Fix it” tells you exactly what it will change; “Keep my setting” / “Run as set” leaves your choice alone. Settings are never changed unless you click. Warnings with no single obvious fix still only warn.
- Frameserver 10-bit is one control: the HEVC/AV1 10-bit box also adds ConvertBits(10) to the script. The separate script Bit depth row is gone. H.264 stays 8-bit. Old presets that had script bits set to 10-bit turn the codec 10-bit box on when the codec supports it.
- The preset sidebar lists **Default Presets** (built-ins, including the Experimental codecs folder) and **User Presets** (saved, duplicated and imported). User Presets is empty until you add one; presets from older files appear there automatically.
- Audio bitrate is a drop-down of typical rates (AAC: 96, 128, 160, 192, 256, 320; AC3: 192, 256, 384, 448, 640) plus Custom… to type another value. The **Will encode: N kb/s** readout is unchanged.
- Hover help was added on technical option labels that did not have it (codec, CRF, 10-bit, film grain, colour range, container, trim, resize, deinterlace, faststart, and the Frameserver colour/signpost/audio rows, plus chapters and interval).
- Leftover FFmpeg update folders (`.ffmpeg-update-…`, left behind if the app was stopped mid-download) are removed in the background when the app starts. Only the app-managed FFmpeg folders are scanned; a download still in progress is left alone.
- HEVC and AV1 encoder speed is labelled **Speed** (H.264 stays **Preset**). AV1 is a drop-down of useful speeds (2, 4, 5, 6, 8, 10, 12, 13) with short labels, plus Custom… to type any value from 0–13. Film grain and **Denoise first** have plain-words help; Denoise first is disabled when film grain is off. The codec and CRF notes are one line each; the full comparison is in the hover (the expanding ⓘ text is gone).

## 0.5.5-codecs.2

- Frameserver (AviSynth script) now has the same video and audio codec pickers as Re-encode: H.264 + AAC stays the default; HEVC, AV1, AC3 and copy are optional. Old saved presets without the new fields still load and behave as before.
- Colour range in Frameserver: Rec709 / Rec601 / Rec2020 already convert to TV range in the script. The default is not to apply a second full-to-TV filter (that would look washed out). You can still turn it on; a clear warning is shown and the encode still runs. PC.709 / PC.601 keep full range and are tagged as full range, with an optional “Convert full range to TV range” choice. Unusual bit-depth or colour combinations also warn instead of blocking or silently changing your settings.
- 10-bit HEVC/AV1 in Re-encode, Batch, Join and Custom pipeline: a full-range → TV conversion and the 10-bit pixel format happen in one step, so the squeeze is not done at 8-bit. H.264 and 8-bit commands are unchanged.

## 0.5.5-codecs.1

- Input and output folders are remembered at the app level (File › Settings), not in presets. On first run they default to the Windows Videos folder. Switching presets or File › New does not change them. Exporting a preset omits folder paths. Existing saved folders are kept.
- Audio bitrate shows a distinct **Will encode: N kb/s** readout and a highlighted selected rate, plus a custom value.

## 0.5.4-codecs.2

- File › New (Ctrl+N), optional output frame rate, job history, top-aligned option rows, and hover help, brought over from stable 0.5.4.
- The updater never modifies an FFmpeg folder you installed yourself — only the app-managed copy.

## 0.5.4-codecs.1

- First Codecs edition: optional HEVC (libx265) and AV1 (SVT-AV1) video, and AC3 or stream-copy audio, with H.264/AAC remaining the default. Separate exe and data folder; first start can import stable settings (read only).

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
