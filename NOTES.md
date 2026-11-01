# Notes / changes vs PowerShell script v22.0

## 0.3.0
- **Chapter creator** (Re-encode, Custom pipeline incl. its folder batch, Batch re-encode):
  - ffprobe now reads chapters (`-show_entries chapter=start_time,end_time:chapter_tags=title`); each form shows
    "N chapter(s) found" or "No chapters found in this file" with a **Create chapters…** button.
  - **Chapter creator…** window: mode (off / auto / manual / auto + manual), interval, manual table (add, edit, delete,
    move, sort by time, clear), **set from timestamp** (`750.5`, `12:30`, `1:02:03.250`, `1h2m3s` → normalised
    `hh:mm:ss.mmm`), **import from text** ("00:12:30 Title", "00:12:30 - Title", YouTube-style "0:00 Intro";
    append/replace; open a .txt or an existing FFMETADATA file), **load the file's own chapters** (shifted by trim start),
    copy list as text, live **preview** (start/end/length/title) and the exact FFMETADATA1 text.
  - Option **"If the input has no chapters, create one every N min"** (chapters off): only files without chapters get
    them; files with chapters are untouched. Available per preset in Re-encode / Custom and in Batch re-encode.
  - New built-in preset **Add chapters (no re-encode)**: Custom pipeline with stream copy, auto chapters every 5 min,
    container "same as input" (mp4/mkv/mov/m4v, else mkv), suffix `_chapters`. Existing presets files get it appended
    once (files remember `known_builtins`, so deleting it later sticks).
  - Custom pipeline container gained **same** (as input).
- **FFMETADATA1 output** (per FFmpeg's "Metadata" docs): first line `;FFMETADATA1`, per chapter `[CHAPTER]`,
  `TIMEBASE=1/1000`, integer `START`/`END` in ms (rounded), `title=`; no spaces around `=`; `=`, `;`, `#`, `\` and
  newlines escaped with `\` (CR/CRLF in titles become one escaped LF); UTF-8 without BOM; LF line endings (a CRLF
  writer exists and is tested against real ffmpeg). Only chapters are written: no global keys, so with
  `-map_chapters N` (never `-map_metadata`) the input's other metadata is untouched.
- **Validation:** starts strictly increasing, no zero-length or overlapping chapters, all inside `[0, output duration)`,
  last END = output duration, duplicate manual times are an error (were silently merged), invalid times name the row.
- **Behaviour changes:** untitled chapters are named "Chapter N" by their *final position* (auto + manual used to number
  auto chapters separately); no automatic chapter is created in the last second of the output (rounding produced 4 ms
  tail chapters).
- **Temp files:** `%TEMP%\ffenc_chapters_<id>.ffmeta` (was `.txt`), written before the job, deleted on success, failure
  and cancel by the runner; leftovers older than 6 h (crash) are removed at startup.
- **HandBrake-style preset panel** (left): all presets in your order; one click selects, **double-click or ▶ Run (F5)**
  loads the current input into that preset and opens the command preview (Add to queue / Start from there);
  `*` marks unsaved changes; quick buttons **New / Duplicate / Import / Export**; **search** (name, type, folder,
  description); **favorites** (pin → "Favorites" group at the top); optional **folders** (Set folder…; groups appear only
  when used; ⬆/⬇ move within a group). The full Presets menu stays (also has Pin, Set folder, Run).
- **Source panel** (right, View › Source info panel): ffprobe summary of the selected preset's input – container,
  duration, size, bitrate, video codec/profile, resolution, fps, pixel format, interlace (field order), audio tracks
  (codec, layout, rate, bitrate, language, title, default), subtitle tracks (language, forced), chapters (list),
  attachments. It names the ffprobe used ("same folder as ffmpeg"); the FFmpeg folder dialog and Settings say so too.
- ffprobe parsing also reads fps, profile, pix_fmt, bitrates, channel layout, dispositions; cover-art streams are no
  longer taken as the main video.

## 0.2.0
- Modes became **presets** (named, editable, savable configurations of a preset type). The 9 script modes ship as
  built-in presets; all can be edited, renamed, reordered, duplicated, deleted, reset to factory, exported and imported.
- New **Custom pipeline** type: composable video / audio / subtitle actions, trim, chapters, container, name suffix,
  extra input/output ffmpeg options, single file or folder batch.
- Preset values exclude per-run fields (input files, typed output names). Those are kept as drafts, so switching presets
  or restarting keeps the chosen file, and the file is carried to the next preset you open.
- 0.1.x remembered values (`modes` in the settings file) are migrated once into the built-in presets.
- Built-in types gained a **name suffix** field; Re-encode gained a **container** choice (auto = script behaviour).
- First run, or no working ffmpeg/ffprobe: a dialog asks for the FFmpeg folder (pre-filled `C:\ffmpeg\bin`,
  Browse, validation; the parent of `bin` also works), with Download newest FFmpeg / Skip for now. Help › Locate FFmpeg folder… reopens it.
- Custom pipeline details: video is mapped as `0:V?` (real video, not cover art); data streams are not mapped; attachments
  are kept only in Matroska; trim start is applied to the extra audio/subtitle inputs too (they're assumed aligned with the
  main file); a corrupt presets file is moved aside to `*.bad-<time>`, never overwritten.

## 0.1.0

Behaviour follows the script unless listed here. Each change fixes a bug or an inconsistency found while porting.

## Encoding
- **Range conversion is consistent.** Whenever video is re-encoded with “convert range” on, the app applies
  `scale=in_range=full:out_range=tv` **and** writes BT.709 tags (`-color_range tv -colorspace/-color_primaries/-color_trc bt709`).
  The script used only `-color_range tv` in the passthrough-audio path and tags without the filter in mode 8.
  Mode 2 (still image, RGB source) uses `scale=out_range=tv:out_color_matrix=bt709,format=yuv420p`.
- **Seek:** `-ss` stays before `-i` (fast, frame-accurate with re-encode); `-t` is an output option.
- **Chapters** are a separate ffmetadata input mapped with `-map_chapters 1`. The script used `-map_metadata 1`,
  which wiped global metadata, and dropped chapters in the passthrough-video multi-track path.
  If a manual chapter falls on the same time as an auto chapter, the manual title wins. Chapters are relative to the trimmed output.
- **Multi-audio gain/compressor** uses `-filter:a` (one simple graph per stream) instead of `-filter_complex`,
  so track order, titles and languages are kept.
- **Compressor** may be used even when gain ≤ 0 (the script hid it); a hint explains it.
- **Duration override** is an explicit checkbox; otherwise ffprobe's duration is used.
- **MP4 outputs:** text subtitles → `mov_text`; bitmap subtitles (PGS/DVB/VobSub) and attachments are excluded
  with a warning (MP4 can't hold them; the script's command failed). Mode 8 offers convert or try-copy.
- `-movflags +faststart` is an output option (the script put it before inputs in modes 2/3, where ffmpeg ignored it).
- **Mode 3:** the commentary track index follows the source's real audio track count (script hard-coded `a:1`,
  wrong for multi-track sources). In MP4 the name is also written as `handler_name` because MP4 has no per-track
  `title` (the script's MP4 output lost the name). Found by the integration test.
- **Mode 6** always writes MKV (amix output + chapters), `amix normalize=1`, needs ≥2 tracks.
- **Mode 7** refuses to overwrite its own input; deinterlace defaults on (auto only when ffprobe says interlaced).

## Files & safety
- Every ffmpeg call is an argument vector (no shell, no quoting bugs); windows hidden via `CREATE_NO_WINDOW`.
- Per-job log `ffmpeg_error_<name>.log` in the output folder; removed on success unless the setting is on.
- **Remux** never overwrites: it writes a temp file and publishes via hard link (Windows `rename` would replace),
  falling back to check-then-rename. Source is deleted only after the MP4 exists and is non-empty.
  “Delete sources” is not remembered between runs.
- Overwrite warnings are shown in the preview before anything runs.

## FFmpeg updater
- Default install target is an app-managed folder, not your `C:\ffmpeg\bin`; in-place update needs a tick.
- gyan.dev has no “release full” zip any more (404); Stable uses `ffmpeg-release-essentials.zip`, Latest uses
  `ffmpeg-git-essentials.7z` (7z support built in). Both verified against gyan's `.sha256`.
- BtbN “Stable” = newest `ffmpeg-nX.Y-latest-win64-gpl-X.Y.zip` in the `latest` release; verified with GitHub's asset digest.
- Same release branch (e.g. installed n9.0.2-39-… vs available n9.0) is compared by build date (±1 day).
- One backup generation; rollback swaps current and previous (so rollback twice = undo the rollback).
- Updates and rollback are disabled while jobs are running.
