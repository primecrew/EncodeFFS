# Changelog

User-facing history for **Encode FFS** (stable edition, `master`, `Encode FFS.exe`). Developer notes are in [NOTES.md](NOTES.md).

Create Slideshow is part of this stable build (the old **Slideshow** branch is archived as `archive/slideshow`). The **Codecs** edition (optional HEVC / AV1 video and AC3 audio, H.264 + AAC remaining the default) is now also merged into this stable app. The `codecs` branch is kept for history and is tagged `archive/codecs`. The retired **Experimental UI** branch (`ui-experimental`) remains separate.

## Unreleased

- **User guide** with screenshots, in Markdown in the `docs` folder (start at `docs/README.md`): getting started, the single-file presets, Batch, Remux, Frameserver, Slideshow, Custom pipeline and custom batches, Trim and time formats, Presets, Settings, Job history and a short troubleshooting page.
- **Help › User Guide** opens it. The installer and the portable zip now include an offline copy (HTML pages and images in `docs` next to the program), which opens first; without it the app opens the online guide on GitHub. Uninstall removes the guide's files.

## 1.1.0-rc.3

- **Fast start (web streaming)** is only offered for MP4, MOV and M4V outputs. For MKV it is greyed out with a note, because it only applies to MP4-style files. Create Still Image Video follows the same rule.
- Menus stay on one line at every text size (for example View › Choose size no longer wraps after changing the text size).
- **Video codec picker:** codecs that the FFmpeg in use does not have stay selectable, marked "(not in your FFmpeg)". Picking one shows a short warning and a **Use the app's own FFmpeg for <codec>** button, which downloads the app's own copy (your own FFmpeg folder is never changed). A (?) next to the "Using:" line explains which FFmpeg is used and why.
- **GPU encoding** (File › Settings… › Encoding hardware), off by default. The app checks your FFmpeg for NVIDIA (NVENC), Intel (Quick Sync) and AMD (AMF) encoders with a short test encode, in the background on first start and with **Re-check**, and shows what it found, for example "NVIDIA GeForce RTX 4070 found: H.264, HEVC and AV1 can use your GPU (much faster, slightly bigger files)". Choose **Use my GPU** or **CPU only (best quality per size)**; after the first successful check the app offers once to turn it on. When on, every preset that re-encodes H.264, HEVC or AV1 uses the GPU encoder that passed its test. The CRF value is converted to the GPU's quality setting, 10-bit stays 10-bit where the GPU can do it, and the run preview and the "GPU: …" line under "Using:" show the GPU encoder. If a GPU encode fails, that file is encoded again on the CPU and the batch carries on. Copy/remux and Custom are never changed. With it off, commands are exactly as before.
- **Frameserver: audio stays in sync when the frame rate changes.** The script's old "Force a frame rate" option relabelled the frames (AviSynth AssumeFPS), so the video got faster or slower while the PCM audio from the signpost kept its length (for example 29.97 → 23.976 fps made 10 s of video last 12.5 s). That option is removed: the **Output frame rate** box is the only frame rate control. It repeats or drops frames, so the length and the audio stay the same. Presets saved with the old option still load; their rate moves into Output frame rate when that box is empty.

## 1.1.0-rc.2

- Crop: one large preview instead of four small pictures. **◀ Previous** and **Next ▶** step through the frames at 20, 40, 60 and 80 % into the video, with a label such as "2 of 4 (40 %)". The crop outline and the file drop-down (several files) are unchanged. The Top / Right / Bottom / Left boxes, with the source and result size, are now under the preview.
- Watermark: the "Keep the image's shape" box is now called **Maintain Aspect Ratio** (it works the same way).

## 1.1.0-rc.1

- **Crop** on every preset that re-encodes video (Re-encode and the archive presets, Batch re-encode, Join with the Re-encode method, Custom pipeline when video is encoded, Frameserving, Add Watermark). Off by default. Tick **Crop** and enter Top / Right / Bottom / Left in pixels; four preview pictures (20, 40, 60 and 80 % into the video) show the crop outline, with the source size and the result size. With several files, a drop-down picks which file to preview, and every file is checked. An odd value or odd result shows a red warning with a **Round to even** button; a crop that removes the whole picture stops the run; a result that is not a multiple of 8 is only a note. The crop is the first step, before deinterlace and resize.
- **Add Watermark** preset: add videos (files, a folder or drag and drop), pick an image (PNG, JPG, WebP, BMP, GIF, TIFF — PNG transparency is kept) and place it on the picture from the middle of the video. Drag it to move, drag a corner to resize (shape locked by default, untick "Keep the image's shape" to stretch), set opacity 0–100 %, type X / Y / W / H, or use the corner and center buttons with a margin. The position and size are stored as a share of the picture, so they suit every file. The output is as long as the video; audio is copied by default and all the usual video and audio options are there. Optional crop runs first.
- **Add watermark** check box (off by default) on the same re-encode presets as Crop, with **Set up watermark…** to open the same placement window. Ticking it the first time opens the window. With Crop on, the watermark is placed on the cropped picture. Join puts the watermark on the joined video.
- About text is now "A dedicated front end for FFmpeg by Tolwyn" (the old PowerShell-script line is gone). The heading and the GPL v3 line are unchanged.

## 1.0.0

- First stable release.

- LICENSE is the official GPL v3 text again (so GitHub can detect it). A short COPYRIGHT file next to it has the PrimeCrew Software copyright line; the portable zip and installer include that file.

## 0.6.0-rc.7

- A first-time install goes in `%LOCALAPPDATA%\Programs\Encode FFS` (or Program Files\Encode FFS when installing for everyone). An upgrade of an existing copy stays in its current folder and still uses the Add/Remove Programs key `FFmpegEncoder`. The portable zip and installer are now named `EncodeFFS-v<version>-win64.zip` and `EncodeFFS-v<version>-setup.exe`. The setup footer is "PrimeCrew Software". The About box shows "Encode FFS v<version> by PrimeCrew Software, (C) 2026", and the same copyright line is at the top of LICENSE (above the official GPL v3 text) and in the exe's file properties.

- The installer welcome and Ready to Install pages show when they are updating an existing copy: a banner "Updating existing installation: Encode FFS v<old> → v<new>" and a read-only "Installed in:" path (the folder cannot be changed). A first-time setup is titled "New installation" and keeps the usual "Install to:" folder page with Browse. The last page says "Updated to v<new> in <path>" or "Installed v<new> to <path>". If the previous version is not recorded, the banner shows v(unknown).

## 0.6.0-rc.6

- Folder-based presets (Remux, Batch re-encode, Custom in folder mode) start with empty folder fields, including after File › New. A Recent drop-down lists the Settings default folder first, then folders you used recently. File › Settings › Remember recent folders is 4 by default (0 = none, maximum 20). Picking a folder or running a job remembers it; the Settings default folders are not changed. Other presets still use the Settings output folder on New.

## 0.6.0-rc.5

- The Encoding Finished window is wide enough for a typical file name such as `2025-04-25 17-01-45.mp4` to stay on one line, with the size and time underneath and the Go to file buttons aligned. Scroll, Open folder and Close are unchanged.

- **Remux MKV → MP4** is now **Remux**. Add files, a folder, or drag and drop, and pick the output container: MKV, MP4, MOV, WebM or TS (no AVI). Tracks are always stream-copied, never re-encoded. Each container warns about tracks (video, audio, subtitles, attachments) that FFmpeg cannot copy into it. Files already in the chosen container are skipped. MPEG-TS notes that chapters and track names are mostly lost. Older “Remux MKV → MP4” presets still load as Remux with MP4. A remux from MKV to MP4 uses the same command as before.

- If your FFmpeg does not have the encoder for a job (for example AV1) and the app-managed FFmpeg does, that encode runs with the app-managed copy. The **Using:** line shows this. Your own FFmpeg is not changed. If the app-managed copy is not installed yet, you are asked: “Your FFmpeg doesn't support AV1. Download the current FFmpeg build?” with Download / Cancel. A download goes only into the app-managed folder.

- A **(?)** next to CRF opens a short explanation (lower number = better quality, bigger file) and a suggested range for the current video's size. Suggestions follow HandBrake's official quality tables (the same x264/x265 ranges, and a separate SVT-AV1 table). Your CRF value is never changed automatically.

## 0.6.0-rc.4

- Extract audio to WAV warns when a file has more than one audio track, names the default-disposition track FFmpeg would pick, and offers a choice: the default track (same command as today), a specific track, or each track to its own WAV (`<name> - Track 2 - Mic.wav`). The status line and sample rate follow the track that will actually be extracted.

- Join / stitch files (re-encode method) warns when any file has more than one audio track that only the first track of each file is kept. The lossless copy method is unchanged.

- Custom pipeline’s Replace action shows the same “all N tracks are replaced” warning as Replace primary audio.

- The **Add commentary track** preset is now **Add Additional Audio Track** (sidebar, form, hover help). The default track title is **Additional audio** (File › Settings › Added audio track name). Saved presets and the `commentary_name` settings key are unchanged; existing saved titles are kept.

- Remux MKV → MP4 warns when a copied audio or subtitle track cannot go in an MP4 (for example TrueHD, MLP, some PCM, SRT, ASS, PGS, WebVTT) or when the file has attachments, naming the track. FLAC, Opus, Vorbis and the PCM types FFmpeg can store in MP4 get a note that many players will not play them. The remux command is unchanged (warn, don’t block); a failed job still keeps its MKV.

## 0.6.0-rc.3

- Installer and first-run wording offers to use your existing FFmpeg or download a copy in the app (Help › Check for FFmpeg updates…), instead of leading with “FFmpeg is not included”. Behaviour is unchanged: the installer does not download FFmpeg, and a download never overwrites your own FFmpeg folder.

- The visible app name is now **Encode FFS**, and the publisher is **PrimeCrew Software** (window title, About box, installer display name, Add/Remove Programs, and the exe version-info resource). The Start Menu shortcut is **PrimeCrew Software\Encode FFS**; an upgrade removes leftover Start Menu shortcuts from earlier names (the previous shortcut in the PrimeCrew Software folder, and old names at the Start Menu root and on the desktop) so they are not duplicated, and uninstall removes the shortcut and the PrimeCrew Software folder if it is empty. The program file is `Encode FFS.exe` (the install folder and Add/Remove Programs key stay `FFmpeg Encoder` / `FFmpegEncoder`). An upgrade from a copy that still has `FFmpegEncoder.exe` removes that leftover file and repoints the shortcuts. Portable settings stay in `FFmpegEncoder.settings.json` next to the exe, so an existing portable folder keeps working. Installed settings now live in `%APPDATA%\PrimeCrew Software\Encode FFS`, and the default app-managed FFmpeg in `%LOCALAPPDATA%\PrimeCrew Software\Encode FFS\ffmpeg`. On first run, if the new folder is empty and the previous `%APPDATA%` / `%LOCALAPPDATA%\FFmpegEncoder` folders exist, those files (and a default-location FFmpeg copy) are copied across; the old folders are left as a backup. A custom FFmpeg location is left where it is. Portable mode is unchanged (settings stay next to the exe). If you install over a portable copy and have no installed settings yet, the app offers to import the portable files without deleting them. “Remove my settings and presets” on uninstall clears both the new folders and any leftover previous folders, never a custom FFmpeg location you chose.

- Conflict warning text wraps to the window width, and the fix button (plus “Keep my setting” or the pre-run “Run as set” footer) sits on the next line at a natural width, so the button is not squeezed into a narrow column at small windows or large text. The warning no longer panics when it is drawn inside a two-column form grid. Unusual settings again show a pre-run “Fix it / Run as set” popup before the command preview. Create Slideshow no longer crashes when its Duration / Transition / Size headings are drawn.

- Create Slideshow and MP4 from still image have the same video and audio codec pickers as Re-encode (H.264 + AAC remain the default, so existing default commands are unchanged). File › Settings › Encoding defaults is codec-aware as well. The sidebar folder for the HEVC / AV1 starting points is now **Archive codecs** (the old “Experimental codecs” name is renamed on load). Built-in padlocks stay in a fixed slot on the right of the name. Settings has a visible scrollbar, an **Apply** button (save and keep the window open; **Save** applies and closes; **Cancel** discards unapplied changes), and the automatic FFmpeg-update checkbox is labelled **Don't automatically check for FFmpeg updates**. Hover help is capped so large text does not bury the panel. The unused Help › Check for app updates stub is gone. The exe’s File Version / Product Version now follow the crate version (0.6.0-rc.3 → 0.6.0.3), and release builds remap compiler paths so panic messages do not include the build machine.

- A newer installer finds an existing Add/Remove Programs entry (current-user first, then all-users), upgrades in that same folder and install mode, and says so on the welcome page (for example “Encode FFS 0.6.0-rc.2 is installed. This will upgrade it in place to 0.6.0-rc.3. Your settings and presets are kept.”). The who/where pages are skipped. Installing the same version again, or an older one, is allowed, with a plain reinstall or older-version note. If the app is running, setup asks you to close it (Retry / Cancel) before replacing the exe. Settings, presets and leftover portable files are not removed. A fresh install is unchanged. Passing `/D=` still uses that folder.

- Preset names in the sidebar are left-aligned again (the padlock stays in a fixed slot on the right, and the selection highlight still spans the row).

- The video codec picker shows a short **Using:** line for the FFmpeg that is in use (“your FFmpeg” with its folder, or “app-managed FFmpeg”). If a codec is greyed out because that copy lacks the encoder, but the app-managed copy has it, a **Switch to the app-managed FFmpeg** button points the FFmpeg folder setting at the managed copy (saved), re-probes encoders, and leaves your own FFmpeg files alone. If there is no managed copy, the existing hover still points to Help › Check for FFmpeg updates…. The missing-encoder note names which FFmpeg is in use.

- The program file is now `Encode FFS.exe` (the crate, repo, zip and setup names stay `FFmpegEncoder`). An installer upgrade deletes a leftover `FFmpegEncoder.exe` from the install folder and repoints shortcuts. A portable copy still reads `FFmpegEncoder.settings.json` next to the exe.

- The **Using:** line under the codec picker is normal body text (full-strength colour, not small or grey). The **Switch to the app-managed FFmpeg** button is shown once per form: under the video picker when a missing video encoder is in the managed copy, or under the audio picker if only AC3 is the reason.

- The MP4 from still image subtitle is now “One image + one audio file -> MP4 video.” Create Slideshow’s subtitle no longer hard-codes H.264/AAC either (the defaults are unchanged).


## 0.6.0-rc.2

- The Codecs edition is merged into the stable app: optional HEVC / AV1 video and AC3 or stream-copy audio (H.264 + AAC remains the default), hover help, one-click conflict fixes, colour-range warnings, and the Frameserver default colour matrix of PC.709. Create Slideshow from rc.1 is kept. The app identity stays FFmpegEncoder / %LOCALAPPDATA%\FFmpegEncoder; the standalone Codecs first-run import is not carried over.
- README now describes the merged app in one place: Frameserver default colour matrix is PC.709 (example script updated), codec pickers on Frameserver and Custom pipeline, Create Slideshow in the preset list (19 built-ins), Default Presets / User Presets sidebar, Faststart off for new jobs, and HEVC starting points in **Choosing a codec**.
- File › Settings › Encoding defaults: the Gain row uses the same pre-gain hover as the encode forms.
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

## 0.6.0-rc.1

- **Create Slideshow** is a built-in preset in the stable app (images → H.264/AAC MP4 with optional audio and crossfades). Vertical image list, pop-out, drag-and-drop reorder, additive transitions (`total = N×D + (N−1)×T`). List display options are stored in `slideshow-ui.json` next to the stable settings file. The separate Slideshow edition identity (own exe, data folder, and first-run import from stable) is not used.
- Input and output folders are remembered at the app level (File › Settings), not in presets. On first run they default to the Windows Videos folder. Switching presets or File › New does not change them. Exporting a preset omits folder paths. Existing saved folders are kept.
- Audio bitrate shows a distinct **Will encode: N kb/s** readout and a highlighted selected rate, plus a custom value.

## 0.5.5

- Input and output folders became app-level settings (Windows Videos on first run; remembered thereafter).
- Audio bitrate selector gained a clear **Will encode** readout and a highlighted selected rate.

## 0.5.4

- File › New (Ctrl+N) starts a fresh job for the selected preset without changing presets, Settings, or the queue.
- Optional output frame rate on re-encode tools (empty = same as the source). Uses an `fps` filter so duration and audio stay the same.
- Job history of successful jobs (File › Job history…).
- Option-row labels are top-aligned; more hover help.
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
