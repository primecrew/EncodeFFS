# FFmpeg Encoder

> **Experimental edition (branch `ui-experimental`, version 0.6.0-dev.x).** This is a test version, separate from
> the stable 0.5.x line on `master`, and safe to install next to it:
> - It is `FFmpegEncoder-Experimental.exe`, and the window title ends in “(Experimental)”.
> - It keeps its own settings, presets, logs and downloaded FFmpeg (`%APPDATA%` / `%LOCALAPPDATA%\FFmpegEncoder-Experimental`;
>   portable: `FFmpegEncoder-Experimental.settings.json` / `.presets.json` next to the exe).
> - It never reads or writes the stable version's files, except when you choose **File › Import settings from
>   stable version…** (also offered on the first start). That copies them; the stable files are only read.
> - The installer has its own Add/Remove Programs entry (`FFmpegEncoderExperimental`).
> See “Experimental edition” below.

Portable Windows GUI (Rust + egui) that replaces the PowerShell script v22.0.
One `FFmpegEncoder.exe`, no installer, no runtime. Needs `ffmpeg.exe` + `ffprobe.exe`
(the app can download them for you).

## Run
1. Copy `FFmpegEncoder.exe` anywhere (e.g. `C:\Apps\FFmpegEncoder\`) and run it.
2. First start (and whenever no working ffmpeg/ffprobe is found) asks for the folder holding `ffmpeg.exe` and
   `ffprobe.exe`, pre-filled with `C:\ffmpeg\bin`, with Browse… and live validation (the parent of a
   `bin` folder is accepted too). Secondary buttons: **Download newest FFmpeg** and **Skip for now**.
   Later: Help › Locate FFmpeg folder… or File › Settings.
3. Pick a preset in the **preset panel** on the left, choose a file (Browse… or drag-and-drop – see below) and
   press **Preview & run…** — or double-click a preset / press **▶ Run** / F5. The confirmation shows every job's full
   ffmpeg command (full `ffmpeg.exe` path, temp chapter file contents) and any existing output files (see below); **Run** queues them.
   Each command has a **Copy** button (copy icon; "Copy all" for batches) that copies it for **PowerShell** or
   **Command Prompt** (pick under "Command syntax"), with paths and special characters quoted for that shell. The **Source** panel on the right shows what ffprobe
   reports about the input.
4. The queue shows progress per job and can cancel one or all. Cancel sends `q` to ffmpeg, kills it after 4 s,
   and deletes the partial output.

FFmpeg lookup order: Settings folder (default `C:\ffmpeg\bin`) → `ffmpeg\` or `ffmpeg\bin\` next to
the exe → app-managed copy `%LOCALAPPDATA%\FFmpegEncoder\ffmpeg\bin` → `PATH`.
On start the app checks the build has `libx264` and `aac`.

## Appearance, text size & keyboard
- **Theme** (View menu, or File › Settings… › Appearance): **System** (default — follows Windows' light/dark app mode,
  dark if unknown), **Dark**, **Light**, **High contrast** (black background, white text, yellow accent, thick outlines).
  Applies immediately and is remembered. All status colours (ok / warning / error / info) meet WCAG AA 4.5:1 in every theme.
- **Text size** 75–300 % (steps 100, 110, 125, 150, 175, 200, 250, 300 %): **Ctrl + plus / Ctrl + minus**, **Ctrl + 0** = 100 %,
  **Ctrl + mouse wheel**, View › Text size, or the slider in Settings › Appearance. Scales all text, buttons, lists,
  dialogs and icons; remembered. Separate **command/log (monospace) size** (80–200 % on top of the text size) and a
  **Bold text** option (uses Segoe UI Bold). At large sizes panels wrap and scroll; the Source panel hides itself when the
  window is too narrow and comes back when there is room (View › Source info panel).
- **Keyboard:** Tab / Shift+Tab move between controls (the focused one has a thick outline), Enter / Space press it,
  arrow keys change sliders and lists, Esc closes dialogs. Shortcuts: **F5** run selected preset (the confirmation opens
  with **Run** focused), **Ctrl+O** open file(s)/folder for the selected preset, **Ctrl+S** save preset, **Ctrl+F** search
  presets, **F1** list of shortcuts.
- **Screen readers:** buttons, labels, check boxes and text fields are exposed through UI Automation (AccessKit), and
  icon-only buttons have names (e.g. "Move chapter 2 up", "Copy command for job 1"). See NOTES for known gaps.

## Existing output files (overwrite protection)
Before anything is queued, every job's output is checked (all presets, single files, batches, Join, still image,
WAV extract …; case-insensitive like Windows):
- a file with that name already exists,
- two jobs of the same run would write the same file,
- the output is one of the job's own input files (never allowed – ffmpeg can't write over its input).

The run confirmation then lists each such file with the existing file's size and date and three choices:
**Keep both** (default, pre-selected and focused: the new file gets the first free Windows-style name
`name (2).ext`, `name (3).ext` …, checking the folder and the other outputs of the run), **Overwrite** (replace the
existing file; not offered for inputs / same-run clashes) and **Skip this file**. With several files, **Apply to all:
Keep both / Overwrite / Skip** sets them all; **Cancel** (or Esc) runs nothing. Every choice is a labelled radio
button reachable with Tab and Space.

File › Settings › **When the output file exists**: *Ask* (default), *Always keep both (auto-number)*,
*Always overwrite*, *Always skip* – the confirmation still shows the conflicts, pre-set to that choice.

How a job writes its file: ffmpeg writes `name.ffenc-partial-<id>.ext` in the output folder (with `-n`; `-y` only
for a confirmed overwrite) and the app renames it to the final name only after ffmpeg succeeded. A failed or
cancelled job deletes just that partial file – an existing file is never touched. Without a confirmed overwrite the
rename never replaces a file; if one appeared meanwhile, the result is saved as `name (2).ext` and the log says so.
The confirmation and **Copy** show exactly this: the ffmpeg command with the partial name plus the rename step
(`[IO.File]::Move` in PowerShell, `move` in cmd). Partial files left by a crash are removed at the next start.
Remux (MKV → MP4) keeps its own rule: existing MP4s are skipped.

## Drag and drop
Drop files or folders anywhere on the window; they go to the selected preset. While you drag, the window gets an accent
frame, a banner names the preset, and every field that accepts a drop (input file / folder fields, the Join list) is
highlighted. Empty input fields read "Drop a file here, or Browse…"; Browse… always works too. After the drop a short
note says "Added to …" or that nothing in the drop fits that preset.
- **Single-file presets** take the first matching file: a dropped folder is expanded to the files inside it (natural
  name order), so dropping a folder on Re-encode picks its first video. Presets with a second input (commentary audio,
  replacement WAV, subtitle file, still image + audio) fill those from the same drop.
- **Folder presets** (Batch re-encode, Remux, Custom pipeline in batch mode) use the dropped folder (or the folder of a
  dropped file).
- **Join / stitch files** takes every video in the drop (folders expanded), sorted naturally, duplicates ignored.

## Join / stitch files
Built-in preset (type "Join / stitch files", editable like any other) that joins several videos into one, top to bottom.
- **File list:** add with drag-and-drop (files or folders), **Add files…** or **Add folder…**. New files are sorted in
  natural order (`file2` before `file10`, case-insensitive). Reorder by dragging the dotted grip or with ⬆ / ⬇; ✖
  removes; **Sort by name** (natural), **Sort by modified date** (oldest first), **Reverse**, **Clear**. Each file is
  probed: duration, codec, size, fps, audio; the total duration is shown.
- **Fast, lossless** (default): concat demuxer, `-f concat -safe 0 -i <list> -c copy`. The list is a temp file
  (`%TEMP%\ffenc_concat_<id>.txt`, UTF-8) in FFmpeg's syntax: `ffconcat version 1.0`, then `file '<full path>'` per input;
  an apostrophe in a path is written `'\''`, backslashes stay as they are (literal inside single quotes). Deleted after
  the job (success, failure or cancel). Maps all video and audio (plus subtitles for MKV).
- **Compatibility check:** compares every file with the first (video codec, resolution, frame rate, pixel format, audio
  codec, sample rate, channels, audio track count). Any difference shows a warning with the details and a **Switch to
  re-encode join** button.
- **Re-encode:** concat filter. Every file is scaled (keeping aspect) and padded to the first file's size, `setsar=1`,
  converted to its frame rate and yuv420p; audio is resampled to the first file's rate, mono or stereo. Files without
  audio get silence and files without video get black frames. Encoded with the preset's x264 (CRF / preset / tune) and AAC
  (bitrate / gain / compressor) settings. Subtitles are not carried over.
- **One chapter per source file** (on by default): titled with the file name (without extension), start/end from
  the probed durations, accumulated in milliseconds; written with the same FFMETADATA1 writer as the chapter creator.
- Output: folder, name (auto `<first file><suffix>.<container>`, suffix `_joined`), container mp4 / mkv / mov,
  faststart.
- The confirmation shows the full command plus the temp list and chapter file contents. The copied PowerShell command
  writes both files first, so it runs on its own.

## Audio & subtitles
Built-in preset (type "Audio & subtitles") for everything about a video's sound and subtitle tracks. The picture is
never re-encoded. Pick a video and its tracks are listed; then use the sections below. Everything goes into one new file
in one pass, and the confirmation shows a plain summary ("Output will have: 1 video, 3 audio (…), 2 subtitles (…)")
plus the full command with a Copy button.
- **Tracks in the new file:** one box per audio or subtitle track. Untick a track to remove it, and edit the title and
  language (players show these). **Default** marks the track a player picks first (one per type), **Forced** marks
  subtitles that should always show. Move tracks with the ⬆ / ⬇ buttons.
- **Add audio track:** a commentary or second-language track from an audio file or from another video (pick which
  track). Optional title and language, volume (dB), compressor and delay in milliseconds ("+" = later; use it if the
  track is out of sync). **Also add a track that mixes this into the main audio** adds one more track that mixes the main audio and the new track
  ("Game + Commentary Track").
- **Replace audio:** swaps one audio track, or all of them, for a file. The new track takes the old one's place, title,
  language and default flag. To drop all the original audio, untick those tracks.
- **Add subtitles:** .srt, .ass, .ssa, .vtt, .sub, .sup files or another video's subtitles. Language, title,
  default/forced flags, delay and text encoding. **Auto** reads UTF-8 and otherwise assumes Windows Western (CP1252);
  pick another encoding if accents show up wrong.
- **Extract audio to separate files:** saves one track or every track as WAV, FLAC, MP3, AAC (.m4a), Opus or the original format
  (no re-encode). Files are named `Movie - audio 2 (eng, Commentary).ext`. **Extract subtitles to separate files** saves text subtitles
  as .srt or .ass (or the original format).
- **Lengths:** an added track rarely matches the video exactly, and that is fine. Each added track shows how it compares
  ("1.4 s longer – close enough"). Above the **Length tolerance** (Settings, default 1.5 s) you get a warning, but the
  job still runs. Longer audio: **Trim audio to the video length** (default) or **Keep the full audio**. Shorter audio:
  **Leave as is** (default; the track just ends early) or **Pad with silence**. The video is never cut short and no black frames are added.
- **MKV or MP4:** MKV holds any audio and subtitle type and is the safest choice. MP4 plays everywhere but only takes
  AAC/MP3/AC-3 audio (others are converted to AAC) and only text subtitles (converted to MP4's own format; styling from
  .ass is lost). Picture-based subtitles (.sup, .sub, DVD/Blu-ray) are left out of MP4, with a warning.
- Audio that fits the container is copied without re-encoding. Audio that needs changing (volume, compressor, delay
  filter, trim, pad, mix) or doesn't fit becomes AAC (MP4) or FLAC/AAC (MKV, FLAC if the source was lossless).
  Chapters, metadata and attachments (fonts, MKV) are kept. Overwrite protection and partial files work as in every
  other preset.

## Frameserving (DebugMode FrameServer)
Built-in preset **Frameserver (AviSynth script)**. Your editor (Premiere, Vegas, After Effects…) serves the finished
timeline through DebugMode FrameServer, which writes a small "signpost" .avi. You pick that file. The app writes an
AviSynth script (.avs) that opens it and encodes the script with FFmpeg (`-i script.avs`), using the same choices as
Re-encode.
- **You need:**
  - DebugMode FrameServer, with the project open and **serving until the encode has finished**. The signpost only
    works while the editor serves frames.
  - AviSynth+, in the **same bitness as FFmpeg**. The app's FFmpeg is 64-bit, so it needs 64-bit AviSynth+; the
    AviSynth+ installer puts both versions in Windows.
  - An FFmpeg with the avisynth demuxer. Standard BtbN and gyan.dev Windows builds have it (`--enable-avisynth`; they
    load `avisynth.dll` when needed).
- **Checks:** the preset shows plain-language checks at the top:
  - whether the FFmpeg has the avisynth demuxer;
  - whether `avisynth.dll` is found in `Windows\System32`, `SysWOW64` or next to FFmpeg, and whether it is 32- or
    64-bit.
  - Missing pieces are explained, with what to install.
  - **How frameserving works…** opens a short help text.
- **Script options** (all optional; the defaults suit most HD projects):
  - **Colour format** (default **YUV 4:2:0, `ConvertToYUV420()`**). **Which one? (help)** explains each choice:
    - **YUV420:** the normal choice for H.264/H.265 in MP4/MKV; plays everywhere.
    - **YV12 (`ConvertToYV12()`):** the older 8-bit name for the same layout; for old filters or plugins.
    - **YUV422 / YUV444 (`ConvertToYUV422()` / `ConvertToYUV444()`):** more colour detail for editing or archiving,
      but bigger files and fewer players.
    - **RGB32 (`ConvertToRGB32()`):** only for filters that need RGB.
    - **None:** keeps what the signpost gives.
  - **Colour matrix:** `matrix="Rec709"` by default (Rec601, PC.709, PC.601 and Rec2020 are also offered), plus
    **Interlaced source** (`interlaced=true`).
  - **Bit depth:** 8-bit, or 10-bit with `ConvertBits(10)` and a 10-bit encoder format. 10-bit isn't offered with
    YV12 or RGB32.
  - **Signpost format:** AviSource `pixel_type`; leave on Automatic.
  - **Audio:** keep the audio, keep it converted to 16-bit (`ConvertAudioTo16bit()`), or no audio (`audio=false`).
  - **Force a frame rate:** `AssumeFPS(num, den)`, e.g. 30000/1001.
  - **Only part of the timeline:** `Trim(first, last)` in frames; last = 0 means to the end.
- **Script file:** the path is written as-is in quotes; AviSynth has no escapes, and Windows paths can't contain `"`.
  Paths with non-English letters get `utf8=true`. The file is UTF-8 without BOM, with CRLF line endings.
  Example script:
  ```
  AviSource("C:\Users\you\Documents\project.avi", audio=true)
  ConvertToYUV420(matrix="Rec709")
  ```
- **Preview and edit:** the script is shown with a **Copy script** button. **Edit the script by hand** lets you change
  it, for example to use DirectShowSource or FFVideoSource (FFMS2) instead of AviSource if AviSource can't open the
  signpost. That is rarely needed. **Reset to the generated script** goes back.
- **Test the script:** opens the script with ffprobe and reports size, frame rate, frames, length and audio. If it
  fails, you get a plain explanation: the editor isn't serving, AviSynth+ is missing or the wrong bitness, or a
  function needs AviSynth+.
- **Where the .avs goes:**
  - By default it's a temporary file, `%TEMP%\ffenc_frameserve_<id>.avs`, deleted after the job (kept when **Keep
    temporary files for debugging** is on; leftovers are cleaned at start-up).
  - You can also keep it **next to the signpost** or **in a folder you choose**, as `<signpost name>_ffenc.avs`.
  - **Save the .avs now** writes it without encoding.
- **Encoding:** x264 (CRF / preset / tune), optional resize, AAC (bitrate / gain / compressor), chapters, MP4 or MKV,
  faststart. Overwrite protection and the partial output file work as in every other preset. The confirmation shows
  the script and the full command; the copied PowerShell command writes the script first.

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
Everything in the preset panel is a **preset**: a named, saved set of options for an operation **type**.

- Ships with 14 **built-in** presets: the 9 script modes, **Audio & subtitles**, **Join / stitch files**,
  **Frameserver (AviSynth script)**, **Custom pipeline** and **Add chapters (no re-encode)**.
- **Built-in presets are read-only.** They show a padlock and “Built-in, read-only” (screen readers hear
  “built-in, read-only” in the preset list), and they always have the values that ship with the app, so an update
  refreshes them. You can still change the form to try something:
  - The preset is then marked `*` and the notice says “Built-in preset: changes can't overwrite it.”
  - **Save** (button or Ctrl+S) opens **Save as new preset**, with a name like “Re-encode video (copy)” filled in.
  - Clicking another preset while a built-in has unsaved changes asks: **Save as new preset**, **Discard changes**,
    **Keep for later** (the changes stay as unsaved `*`) or **Cancel**.
  - Built-ins can't be renamed or deleted. **Delete** becomes **Hide**. Pin to favorites, folders and reordering
    still work.
  - **Presets › Restore built-in presets…** shows hidden built-ins again and throws away unsaved changes in them.
    Your own presets are not touched.
- **Your own presets** (New…, Duplicate, Save as new…) are fully editable: **Save preset** (Ctrl+S) stores the
  changes, **Revert** drops them, and you can rename and delete them.
- **Upgrading from 0.4 or older:** if a built-in had been edited or renamed, your version is kept as a new preset
  (“Re-encode video (your edits)”, or the name you gave it) and the built-in goes back to factory. Built-ins you had
  deleted come back hidden. User presets that used a built-in's name are renamed (“… (2)”). The log lists every change.
- **New…** starts from a copy of any preset or a blank preset of any type. Right-click a preset for the same actions.
- Files chosen for a run (inputs, typed output names) are *not* part of a preset; they're remembered as a draft and carried
  over when you switch to another preset that takes a file.
- **Favorites** (`"pinned": true`) and **folders** (`"category"`) are stored with the preset and survive export/import.
- **Import / export** (panel buttons or Presets menu): JSON files with one or many presets. Imports get new ids and unique names;
  unknown types are skipped. Imported copies of built-ins become your own presets.
- Stored in `FFmpegEncoder.presets.json` next to the settings file (or `%APPDATA%\FFmpegEncoder\presets.json`).
  For built-ins only the order, pin, folder and hidden flag are stored (`"values": null`); their values come from the app.
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
| **Join / stitch files** | Concatenate several videos: lossless concat demuxer or re-encode with the concat filter, chapter per file (see above). |
| **Audio & subtitles** | Keep/remove/rename/reorder tracks, add commentary or second-language audio, replace audio, add/remove subtitles, extract audio or subtitles (see above). |
| **Frameserver (AviSynth script)** | DebugMode FrameServer signpost .avi -> generated AviSynth script -> x264/AAC MP4/MKV (see above). |
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
**Length tolerance** (seconds, default 1.5): how far an added audio track's length may differ from the video's before
the Audio & subtitles tool warns. It never blocks a job.
**Keep temporary files for debugging** (Settings, off by default) keeps the temporary chapter files, Join list files
and unfinished partial outputs, writes their paths to the log, and skips the startup clean-up.

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
- **Automatic check:** when the app starts it looks for a newer FFmpeg at most once a day and shows a notice if
  there is one. Nothing installs without a click.
  - It only runs for the **app-managed copy** (the FFmpeg the app downloaded). With **your own FFmpeg folder**
    (a folder you chose, an `ffmpeg` folder next to the app, or PATH) the app never checks by itself.
  - **File › Settings › “Turn off Update Check for FFMPEG”** stops automatic checks and the notice completely. It is
    **unchecked by default**, so checks stay on.
  - With your own folder the option is greyed out (screen readers still read it), with a note that it doesn't apply.
  - Upgrading from an older version keeps what you had: if automatic checks were on they stay on; if you had turned
    them off, the new option is checked.
  - **Help › Check for FFmpeg updates…** always works when you choose it, even with the option checked or your own
    folder, and can download an app-managed copy. The dialog then says automatic checks are off.
- **Check for app updates…** is a stub for now.

## Experimental edition: look, splash screen, installer
- **Look:**
  - Tidier spacing and rounded corners.
  - A header card for each preset (name, type and “Built-in, read-only” tags, description, Save / Revert buttons).
  - Section headings with a thin rule.
  - The main action in each place is emphasised: **Preview & run…** sits in an action bar pinned under the form
    (at the end of the form on very short screens), and Run / Save / Import are emphasised in dialogs.
  - A **status bar** at the bottom shows the FFmpeg in use, the queue state and the version.
  - The top bar shows an “Experimental build” tag.
  - Settings › Encoding defaults has a collapsed **Advanced compressor settings** section.
  - Short dialogs fit their content (no empty block under the buttons).
- **Accessibility:** all themes (including High contrast), 75–300 % text size, keyboard use and screen-reader names
  work as before, and every colour pair is contrast-tested.
- **Splash screen** (File › Settings › Startup › **Show splash screen at startup**, on by default):
  - Shows for about 2 seconds. Any key or click closes it, and nothing moves.
  - **To change it** without rebuilding, put files named `splash.txt` and/or `splash.png` next to the exe (or in the
    settings folder):
    - `splash.txt`: a `title:` line, a `tagline:` line, then any other lines as body text. Lines starting with `#`
      are ignored.
    - `splash.png`: any PNG, shown at most 360 × 200.
  - The built-in defaults are `assets/splash.txt` and the app icon.
- **Window layout** (File › Settings › Startup › **Remember window layout**, on by default; 0.6.0-dev.2):
  - Saved and restored at the next start:
    - the window's position, size and maximized state, and which screen it is on;
    - the widths of the preset list and the Source panel, and the height of the queue/log panel;
    - which collapsible sections are open (preset groups, Advanced compressor settings, Audio & subtitles sections,
      Show differences, Import from text, Files lists).
  - The last selected preset was already remembered (settings file).
  - Saved about 1.5 seconds after a change and on exit, so a crash loses at most the last moment.
  - A window that would open off-screen (for example on a screen that is no longer connected) opens centered at
    the default size. A window partly off the screen, or bigger than it, is moved/shrunk onto it, and never opens
    smaller than the minimum size (820 × 600).
  - Sizes are in Windows' logical pixels, so the window keeps its size when you change the text size or theme,
    and on screens with a different scaling. Panel sizes follow the text size.
  - **Reset window layout** (same place) puts the window back to the default size centered on its screen and the
    panels and sections back to their defaults, immediately.
  - Turning the option off stops saving and deletes the saved layout; the window then opens at the default size.
  - Stored in its own file, next to the settings: `FFmpegEncoder-Experimental.layout.json` (portable) or
    `%APPDATA%\FFmpegEncoder-Experimental\layout.json` (installed). If that file is missing, old or damaged, the app
    starts with the defaults; settings are not affected.
- **Portable or installed:**
  - **Portable zip:** unzip anywhere and run; settings are stored next to the exe.
  - **Installer** (`…-setup.exe`, NSIS, 64-bit):
    - **Just for me** by default (no administrator rights, `%LOCALAPPDATA%\Programs\FFmpeg Encoder Experimental`), or
      **For everyone** (Program Files; asks for permission).
    - A Start Menu shortcut, with a desktop shortcut optional.
    - An Add/Remove Programs entry and an uninstaller.
    - An installed copy keeps its settings in `%APPDATA%\FFmpegEncoder-Experimental`. A marker file
      `FFmpegEncoder-Experimental.installed` next to the exe tells the app it was installed.
    - Uninstalling keeps your settings unless you tick **Remove my settings and presets**. That also deletes the
      FFmpeg this edition downloaded, never the stable version's.
    - FFmpeg is not bundled.
  - **Silent install:** `setup.exe /S [/ALLUSERS] [/DESKTOP] [/D=C:\path]`.
  - **Silent uninstall:** `uninstall.exe /CURRENTUSER|/ALLUSERS /S [/REMOVESETTINGS]`.

### Building a release (portable zip + installer)
```
sudo apt install gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 nsis
rustup target add x86_64-pc-windows-gnu
scripts/build-release.sh        # -> dist-experimental/: …-win64.zip, …-setup.exe, SHA256SUMS
```
- The script builds the exe, zips the exe with README and NOTES, and runs `makensis installer/installer.nsi`.
- All names, IDs and folders are `-D` values set at the top of the script, so the stable line can get an installer
  later just by using its own values.

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
- `src/cmd/tracks.rs` – Audio & subtitles: track mapping, codecs per container, delays/trim/pad/mix, extract commands.
- `src/layout.rs` – window layout file and startup placement (off-screen checks, clamping, monitor list); `src/ui/layout.rs` – recording/saving the live layout, remembered sections, Reset.
- `src/import_stable.rs` – read-only import of the stable version's settings/presets; `src/splash.rs` + `src/ui/splash.rs` – splash text/picture (overridable) and overlay; `installer/installer.nsi` + `scripts/build-release.sh` – portable zip + NSIS installer.
- `src/cmd/frameserve.rs` – AviSynth script generator (quoting, colour/matrix/bit depth/trim/fps/audio) and the encode command; `src/ffmpeg/avisynth.rs` – avisynth demuxer and avisynth.dll (32/64-bit) checks.
- `src/cmd/join.rs` – join/stitch: concat list writer, compatibility check, chapter-per-file math, both command builders.
- `src/cmdline.rs` – Windows cmd / PowerShell quoting for the copied commands.
- `assets/` – `icon-source.png` (app artwork), `make_icon.py` → `app.ico` (16–256 px), `app.rc`, manifest.
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
