# Notes / changes vs PowerShell script v22.0

## 0.5.4-codecs.2 — stable 0.5.4 changes (File › New, job history, top-aligned rows, hover help, own FFmpeg never modified) on the Codecs edition
- **File › New (Ctrl+N)** (`PresetUi::new_job`): replaces the selected preset's form with a fresh one built
  from the stored preset (as on first open): input/output files cleared, unsaved option edits dropped,
  folders/defaults from Settings. Asks first only when the form has unsaved option edits. Presets, other
  forms, Settings, job history, the queue (running/queued jobs) and FFmpeg are not touched.
- **Job history** (`src/job_history.rs`, File › Job history…): persistent `logs/job-history.log`; the form's
  values are attached to the job ids when queued (`Tracker::register`), and an entry is appended only when a
  job ends `Done` (failed/cancelled are dropped; batch = one entry per successful file). Clear removes just
  that file. Replaces the idea of a transient "Last run" preset.
- **Top-aligned option rows**: `ui::common::row_label` draws a grid row's label at the top of the row
  (egui's Grid centres cells vertically and can't be configured); the label keeps its normal space, so row
  heights don't change (unit test `row_labels_are_top_aligned_and_stable`, incl. a grid's last row). Used for
  every option label in the first column of the form, settings, updater, path-row and dialog grids.
- **Hover help** (`row_label_tip`) for options without their own help text: passthrough, x264 preset/tune,
  gain, compressor and its parameters, override duration, extra input/output options, frameserver frame
  rate, MP4 subtitles, commentary name, fallback sample rate, keep ffmpeg_error logs.
- **Your own FFmpeg is never modified**: the updater installs only into the app-managed copy (the
  "Update in place" option is gone; the app switches to the managed copy instead) and Roll back is offered
  only for the app-managed copy.

## 0.5.4-codecs.1 — Codecs edition (branch `codecs`, experimental, side by side with stable)
- Identity: `FFmpegEncoder-Codecs.exe`, title “(Codecs)”, `DATA_DIR_NAME`/`FILE_STEM` `FFmpegEncoder-Codecs`
  (`src/lib.rs`); path helpers `roaming_dir_named` / `local_app_dir_named`; other editions' FFmpeg folders are
  protected from managed-FFmpeg moves. First start copies the stable settings/presets in
  (`src/import_stable.rs`, from the experimental branch; stable files only read).
- `src/cmd/codecs.rs`: `VideoCodec` (H264 default / Hevc / Av1), `AudioCodec` (Aac default / Ac3 / Copy),
  `video_args` / `audio_args` (H.264 and AAC go through the unchanged `x264_args` / `aac_args`),
  `check_video` / `check_audio` (container rules, warnings), `Encoders::from_list` (`ffmpeg -encoders`,
  stored in `FfmpegCheck`, so cached per FFmpeg path).
- New `VideoEncode` fields (`codec`, `ten_bit`, `svt_preset`, `film_grain`, `film_grain_denoise`) and
  `AudioEncode` fields (`codec`, `ac3_bitrate_k`) are `serde(default)`: old presets load as H.264/AAC.
- UI `src/ui/codecs_ui.rs`, used by Re-encode, Batch, Join (no audio copy) and Custom pipeline (video).
- Built-ins `builtin-hevc-game`, `builtin-hevc-camera`, `builtin-av1-game`, `builtin-av1-camera`
  (category “Experimental codecs”, sorted below the regular presets); appended via `known_builtins`.
- Tests: `tests/h264_unchanged.rs` + `tests/h264_golden.txt` (golden 0.5.3 commands, written before any codec
  change), `tests/codecs_ffmpeg.rs` (real ffmpeg/ffprobe: hevc+hvc1+yuv420p10le, av1 MP4/MKV + grain, AC3 5.1,
  copy), unit tests in `cmd::codecs` and `presets`.
- `scripts/build-release.sh` builds the portable zip into `dist-codecs/` (installer only with `INSTALLER=1`).

## 0.5.3 — Encoding Finished popup; installer; Bold text option removed
- **Encoding Finished popup** (`show_finished_popup`, on by default; Settings › When encoding finishes):
  - `ffenc::finished`: `Tracker` collects finished jobs and hands out one popup when the queue is idle again
    and at least one job succeeded (single job, batch or queue); failures/cancellations only counted.
  - Jobs record `final_output` (the published name after keep-both numbering, the remux destination, or the
    output path); never the temporary partial name.
  - Go to file: `explorer.exe /select,"<path>"` (from %SystemRoot%), passed raw; paths containing `"` or
    control characters (invalid in Windows names) or a missing file open the folder instead.
  - UI `ui::finished` (foreground window, Esc closes, default button focused one frame later and not while a
    text field has focus; taskbar flash via RequestUserAttention when unfocused).
  - Tests: `finished::tests` (tracker, quoting, fallback, formatting), `settings::tests::finished_popup_setting_and_old_bold_field`,
    `tests/overwrite_guard.rs` checks `final_output` is the keep-both name.
- **Installer** (ported from the experimental edition, stable names): `installer/installer.nsi` +
  `scripts/build-release.sh` → `dist/FFmpegEncoder-<v>-setup.exe` and the portable zip. App id / uninstall key
  `FFmpegEncoder` (experimental: `FFmpegEncoderExperimental`), folder `FFmpeg Encoder`, per-user default, optional
  per-machine, Start Menu + optional desktop shortcut, keeps settings unless ticked, no FFmpeg bundled.
  `settings::INSTALLED_MARKER` (`FFmpegEncoder.installed`) → settings in `%APPDATA%\FFmpegEncoder`;
  test `settings::tests::portable_vs_installed_settings_location`.
- **Bold text removed** (Settings › Appearance and View menu): `appearance.bold` is gone; old settings files
  that still have it load fine (ignored, not written back). Fonts: Segoe UI regular only.

## 0.5.2 — window layout, app-managed FFmpeg location
Functional changes ported from the experimental branch (0.6.0-dev.2/dev.3); no UI restyle, splash or installer.
- **Window layout** (`remember_layout`, on by default; Settings › Window layout › Remember / Reset):
  - window position (physical px), inner size (logical px), maximized state and screen work area; preset list /
    Source panel widths and queue panel height; open sections (stable keys); resizable dialog sizes/positions.
  - `FFmpegEncoder.layout.json` next to the portable settings or `layout.json` in `%APPDATA%\FFmpegEncoder`;
    separate from settings, lenient parsing, unusable values dropped; written ~1.5 s after a change and on exit
    (temp file + rename).
  - Placement decided before the window is created (`ffenc::layout::placement`): off-screen → centered default;
    clamped to the screen; maximized restores maximized with its normal size; mixed-DPI re-check.
  - Panels are re-seeded during a short restore phase so a first frame at another size can't overwrite them.
  - Not resizable, so not remembered: form columns (egui grids), text boxes, the queue list.
- **App-managed FFmpeg location** (`managed_ffmpeg_dir`, "" = `%LOCALAPPDATA%\FFmpegEncoder\ffmpeg`):
  - `ffmpeg::managed`: location override followed by `locate::managed_root()` (updater, `bin.previous` backups,
    automatic check and managed-vs-own detection follow), Windows path validation, folder checks (write probe,
    `GetDiskFreeSpaceExW`), `move_copy` (staging folder, size-checked copy, rename, verify by running ffmpeg,
    delete old; rollback on failure). UI in `ui::managed_loc`.
- Tests: `layout::tests`, `ffmpeg::managed::tests`, `settings::tests::{remember_layout_default_on_and_saved,
  managed_ffmpeg_dir_setting}`.
- New Windows-only dependency `windows-sys` 0.59 (already in the build via eframe/rfd).

## 0.5.1 — "Turn off Update Check for FFMPEG"
- **New option:** File › Settings › FFmpeg updates & logs › **Turn off Update Check for FFMPEG** (`never_check_updates`).
  - **Unchecked by default** for new users and on first run, so the once-a-day check at startup stays on.
  - When checked: no automatic check and no network call at startup, and no update notice (the notice is also
    suppressed if the option is switched on while a check is running).
  - Help › Check for FFmpeg updates… still works and notes that automatic checks are off.
  - The checkbox's visible text is its screen-reader label, and plain help text sits under it.
- **Own FFmpeg folder:**
  - Automatic checks only run for the **app-managed copy** (`FfPaths::is_app_managed`: found as the managed copy,
    or the configured folder is the managed bin, compared case- and slash-insensitively).
  - With the user's own folder (configured elsewhere, next to the app, or PATH) automatic checks never run.
  - The option is shown greyed out (still in the accessibility tree, hover text) with the note "Doesn't apply:
    you're using your own FFmpeg folder…".
  - The manual check still works and can install an app-managed copy.
- **Upgrade path:** files without `never_check_updates` get `!auto_check_updates`; a missing key counts as the old
  default, on. So nobody's behaviour changes. `auto_check_updates` is still written (mirror) so 0.5.0 behaves the
  same after a downgrade.
- **Scheduler:** a single pure function, `Settings::may_check_updates(trigger, now, FfmpegInUse)`. Manual checks are
  always allowed. Automatic checks need app-managed FFmpeg, the option unchecked, and the last check more than a day ago.
- **Tests:**
  - **Unit:** the default is unchecked and checks are enabled (also with no settings file); migration of
    old on/off/missing values and the new key winning; save writes a consistent mirror; the scheduler (day limit,
    no FFmpeg, own folder, option checked, manual always allowed); which FFmpeg counts as app-managed.
  - **Wine smoke test:**
    - The Settings dialog with the managed copy (enabled, unchecked) and with an own folder (greyed out with the note,
      HC 1.5×).
    - With a check due, startup checked for the managed copy (`last_update_check` updated) but not with the option
      checked or with an own folder.
    - The manual Help check still worked with the option checked, showing the note.
- **Not tested:** real Windows and a real screen reader. A disabled egui control isn't reachable with Tab; the note
  under it is plain text.

## 0.5.0 — frameserving and read-only built-in presets
- **Frameserver (AviSynth script)**, a new built-in preset type (`src/cmd/frameserve.rs`,
  `src/ffmpeg/avisynth.rs`, `src/ui/modes/frameserve.rs`).
  - **What it does:** you pick the DebugMode FrameServer signpost .avi (drag/drop or Browse; it is probed with
    ffprobe), and the app generates `%TEMP%\ffenc_frameserve_<id>.avs`. FFmpeg then encodes it with x264/AAC (Re-encode
    choices: CRF/preset/tune, resize, gain/compressor, chapters, MP4/MKV, faststart, overwrite protection,
    partial output).
  - **Function names** were checked against the AviSynth+ docs (avisynth.nl Convert / AviSource):
    - `ConvertToYUV420()` (**default**), `ConvertToYV12()`, `ConvertToYUV422()`, `ConvertToYUV444()` and
      `ConvertToRGB32()`, each with `matrix=` (default `"Rec709"`) and `interlaced=true`.
    - Also: `ConvertBits(10)`, `AssumeFPS(n, d)`, `Trim(a, b)` and `ConvertAudioTo16bit()`.
    - `AviSource(path, audio=true|false[, pixel_type="…"][, utf8=true])`.
  - **Colour help:** inline text for the selected option, plus a **Which one? (help)** button that explains every
    choice in plain words.
  - **Quoting:** AviSynth strings have no escapes, so the path is written verbatim (triple quotes if it contains `"`;
    `"""`, a trailing `"` or line breaks are rejected). Non-ASCII paths get `utf8=true` (AviSynth+ r2768+). The file is UTF-8 without
    BOM, CRLF. ffmpeg's avisynth demuxer imports scripts with `utf8=true` on AviSynth+ (interface 7+), so a script
    path under a non-English user name works.
  - **Keep the .avs:** temporary (deleted after the job unless Keep temporary files; leftovers older than 6 h cleaned at
    start-up), next to the signpost, or in a chosen folder as `<name>_ffenc.avs` (new `JobSpec.keep_side_files`, so
    the runner never deletes it). **Save the .avs now** writes it without encoding.
  - **Edit the script by hand:** the typed text is used as-is. It must still contain a `…Source(` line.
    **Reset to the generated script** goes back.
  - **Test the script:** runs ffprobe on a temporary copy and shows resolution, fps, frames, length and audio. Errors
    are explained in plain words (`explain_error`: not serving / decompressor, AviSynth missing or wrong bitness,
    no avisynth demuxer, a function that needs AviSynth+). The tested length is used for chapters and the progress bar.
  - **Environment check (background):**
    - `ffmpeg -demuxers` (avisynth listed) and `-version` (`--enable-avisynth`);
    - `avisynth.dll` in `%SystemRoot%\System32`, `SysWOW64` and the FFmpeg folder, with its PE machine type, compared
      with ffmpeg.exe's (64-bit FFmpeg needs 64-bit AviSynth+).
    - A missing demuxer blocks the run with a plain message; a missing or wrong-bitness DLL is a warning.
- **Read-only built-in presets** (`presets::enforce_builtins`, `ui/presets.rs`):
  - **Where values come from:** built-in name, type, description and values always come from the code. The presets
    file keeps only their order, pin, folder and the new `hidden` flag (`values` is written as null).
  - **Saving:** Save / Ctrl+S on a built-in opens Save as new preset ("<name> (copy)", made unique).
  - **Switching away** from an edited built-in prompts: Save as new / Discard / Keep for later / Cancel. A
    double-click Run continues after the choice.
  - **Rename and delete:** rename is refused; delete hides. You can't hide the last visible preset.
  - **Restore built-in presets…** unhides them and drops their unsaved changes. Pin, folders and reordering still work.
  - **Screen reader and visuals:** padlock plus "Built-in, read-only" in the header; the accessible name of a list
    entry is "<name>, built-in, read-only[, unsaved changes]"; the dialogs' name fields are labelled and focused.
  - User presets are unchanged: Save, Revert, Save as new, Rename, Delete.
- **Migration on load:**
  - A built-in that differs from the factory one is kept as a user preset right after it: "<name> (your edits)", or the
    custom name if it was renamed. The app compares resolved form values, so concrete-but-default values saved by
    0.4 don't count.
  - The built-in goes back to factory, keeping its pin, folder and position.
  - Built-ins deleted in older versions come back hidden; new ones are added.
  - Duplicate or unknown built-in links become user presets. User presets with a factory id get a new id.
  - User presets named like a built-in are renamed "… (2)". Every change is written to the log.
- **Tests:**
  - **Unit:** 5 for the script generator and command builder, 3 for the AviSynth checks, 1 for the form state,
    5 for presets/migration, 4 for the UI read-only rules.
  - **`tests/integration_frameserve.rs`:**
    - Part 1 always runs the built encode arguments with the real ffmpeg (the .avs input swapped for an equivalent
      AVI) for 6 script variants.
    - Part 2 (set `FFENC_WINE_FFMPEG`) runs each script with the Windows BtbN ffmpeg plus AviSynth+ 3.7.6 x64 under
      Wine and checks pixel format, audio and frame count. This passed here.
  - **Wine app run:** colour help, Test the script ("Test OK: 320x240 · 29.970 fps · 120 frames…"), F5 confirm with
    the script, then Run gave an H.264/AAC MP4 (bt709 tags, 4.004 s) and the temporary .avs was deleted.
  - **Read-only prompts in Wine:** the Save as new prompt, the leave prompt, and the migration of an edited and a
    renamed built-in.
- **Limits:**
  - Not tested with a real DebugMode FrameServer signpost or editor: the "signpost" here was an uncompressed AVI read
    by AviSource through real AviSynth+. Whether AviSource reads the signpost depends on the frameserver's VfW codec
    being installed and the editor serving.
  - 10-bit needs an x264 with 10-bit support (current BtbN/gyan builds have it).
  - 4:2:2 / 4:4:4 / 10-bit H.264 plays on fewer devices (the confirmation warns).
  - The DLL check only looks in System32, SysWOW64 and the FFmpeg folder, not PATH.
  - Not tested with a real screen reader.

## 0.4.0 — audio & subtitles
- **Before:** separate single-purpose presets (Add commentary, Extract audio to WAV, Replace primary audio, Copy
  subtitle tracks), one track at a time, no subtitle removal or file import, no length handling. They all still work.
- **Now:** a built-in "Audio & subtitles" preset (`src/cmd/tracks.rs` builder, `src/ui/modes/tracks.rs` form):
  a track list (keep/remove, title, language, default, forced, reorder) plus Add audio, Replace audio, Add subtitles,
  Extract audio (WAV/FLAC/MP3/M4A/Opus/original, one or all tracks) and Extract subtitles (srt/ass/original).
  One output, one pass, `-c:v copy`; chapters, metadata and MKV attachments kept.
- Design:
  - Copy audio when the container accepts it (MP4: aac/mp3/ac3/eac3/alac); otherwise, or when a filter is needed,
    encode AAC (MKV: FLAC for lossless sources).
  - MP4 track names are written as both `title` and `handler_name`.
  - Only the first ticked Default per type gets `default`.
  - Delays: `-itsoffset` for copied tracks, `adelay=…:all=1` for filtered ones, input `-ss` for negative delays.
  - Trim: input `-t (video − delay)`. Pad: `apad=whole_dur=<video>`.
  - Mix: `amix=inputs=2:duration=first:normalize=0`.
  - Subtitle files get `-sub_charenc` (auto: UTF-8, otherwise CP1252). MKV: srt/ass are copied, others converted.
    MP4: text becomes mov_text, picture subtitles are skipped with a warning.
- Length check: a note per added track; a warning (never a block) above the `duration_tolerance_s` setting
  (default 1.5 s). Auto: trim longer audio, leave shorter audio. Pad with silence and Keep full are optional.
  The video is never shortened or extended.
- Tests:
  - 7 builder unit tests.
  - `tests/integration_tracks.rs` with real ffmpeg on synthetic media: MKV commentary + subs + charset + attachment;
    MP4 replace + pad + mov_text + handler_name; shorter audio doesn't cut the video; m4a delay; mix; every extract
    format; subtitle extract.
  - Wine run end to end: 4 audio tracks incl. the mix, and a CP1252 subtitle decoded to "Grüße".
- Limits:
  - MP3/Opus extract needs an FFmpeg with libmp3lame/libopus (standard Windows builds have them).
  - No picture subtitles in MP4; ASS styling is lost in MP4.
  - The mix uses `normalize=0`, so loud tracks can clip (lower the volume).
  - Charset auto-detect only tells UTF-8 from CP1252.
  - Delay/volume only apply to added tracks, not the video's own tracks.
  - Not tested with a real screen reader.

## 0.3.3 — overwrite protection
- **Before:** every builder passed `-y`, so an existing output was replaced silently (the confirmation only showed a
  warning line); a failed/cancelled encode deleted or truncated the existing file; two jobs could write the same file;
  output = input was only blocked in Batch, Mix and Join.
- **Now** (`src/output_guard.rs`): conflicts = exists / same output as an earlier job of the run / equals one of the
  job's inputs (`-i` arguments), compared case-insensitively on Windows. Choices Keep both (default) / Overwrite /
  Skip per file + Apply to all; Overwrite is replaced by Keep both where it's not allowed. Keep-both names: last dot
  splits the extension (`my.clip.v2 (2).mkv`, `noext (2)`, `.hidden (2)`), an existing ` (N)` continues at N+1
  (`x (2).mp4` → `x (3).mp4`), first number free on disk, among the run's outputs and inputs.
- Setting `when_exists` (Ask / KeepBoth / Overwrite / Skip, default Ask; an unknown value falls back to Ask).
- Every non-remux job is rewritten by `prepare()`: `-n`/`-y`, output → `name.ffenc-partial-<id>.ext` (same folder,
  same extension so the muxer is unchanged), `PostAction::Publish`. After success: `rename` for a confirmed overwrite
  (atomic replace), otherwise hard-link publish that never replaces (falls back to the next free `(n)` name if a file
  appeared). Partial deleted on failure/cancel. Running partials are listed in `logs/partial-outputs.txt`; at start-up
  (background thread) listed partials plus `*.ffenc-partial-*` in the default input/output folders that are older
  than 30 min are removed.
- Confirmation shows/copies the real sequence: ffmpeg command with the partial name + the rename line.
- Setting `keep_temp_files` ("Keep temporary files for debugging", default off): side files (chapters, join list)
  and partials of failed/cancelled jobs are kept and their paths logged; startup clean-up is skipped.
- Accessibility: radios with full accessible names ("Keep both: save the new file as x (2).mp4"), the first
  "Keep both" gets focus in Ask mode (Run otherwise); Run's accessible name includes the kept/overwritten/skipped counts.
- Icon check: the exe's only RT_GROUP_ICON (ID 1) holds 16/24/32/48/64/128/256 px, pixel-identical to
  `assets/make_icon.py` renders of `assets/icon-source.png`.

## 0.3.2 — appearance & accessibility
- **Before:** no theme setting (followed eframe's default), egui's built-in Ctrl+/- zoom worked but was not saved,
  no screen-reader support (AccessKit off), ~60 hard-coded colours (some below 4.5:1 on light backgrounds), icon-only
  buttons without names.
- **Added:** Settings › Appearance + View menu: theme System / Dark / Light / High contrast; text size 75–300 % with
  Ctrl+plus/minus/0 and Ctrl+wheel; separate monospace size; bold text. Live, saved in `appearance` in the settings file
  (old files without it load with defaults; a broken value is ignored instead of discarding the file).
- One palette per theme (`src/ui/theme.rs`); unit test checks every text colour ≥ 4.5:1 against its background.
- Visible focus ring (accent, 3 px in high contrast); shortcuts Ctrl+O, Ctrl+F, F1 (help window) added to F5 / Ctrl+S.
  The confirmation opens with Run focused.
- AccessKit enabled; icon buttons (copy, ⬆ ⬇ ✖, pin, drag handle) carry names + tooltips.
- Layout at large sizes: wrapping header, scrolling central form and queue/log panel, panel size limits relative to the
  window, dialogs capped to the screen, Source panel auto-hides when narrow.
- **PowerShell:** the app does not run PowerShell. The only PowerShell-related feature is "Command syntax:
  PowerShell / Command Prompt" in the confirmation, which controls how the Copy buttons quote the ffmpeg command for
  pasting into a terminal.
- **Limits (not verified):** tested under Wine only — no real Narrator/NVDA run; Wine has no UI Automation, so screen
  reader output is untested. Known gaps: the drag-to-reorder list and the queue progress rows are custom-painted and
  only partly announced; some states are shown by colour plus text but the colour is not announced; egui menus and
  right-click menus have limited keyboard navigation (all preset actions are also in the Presets menu); Windows
  system-theme switching and Segoe UI Bold were not verified on real Windows.

## 0.3.1
- **Run confirmation shows the full commands with copy buttons.** Before: the popup already showed the commands next to
  Run / Cancel, but as one plain-text block with POSIX-style quoting (`\"`, no PowerShell call operator), and a
  text-only "Copy commands" button. Now:
  - every job gets its own block: label, warnings, the full read-only (selectable, monospace, wrapped, never truncated)
    command starting with the full path to `ffmpeg.exe` and the runner's `-hide_banner -nostats -progress pipe:1`,
    exactly as it runs; the temp chapter file is shown with its path and contents;
  - **Copy** buttons with the standard copy icon (two overlapping pages, drawn so it renders on any Windows) +
    label, tooltip "Copy command to clipboard"; per job, plus **Copy all** for batches (one command per line, CRLF);
    "Copied" shows for 1.5 s;
  - **Command syntax** selector, remembered: **PowerShell** (default) or **Command Prompt**. The copied text is
    exactly what's shown.
    - cmd: CRT quoting (`"…"`, `\"`, trailing backslashes doubled) plus `^` escapes for `& | < > ^ ( )` wherever cmd
      would see them unquoted (cmd's quote state is tracked across the whole line).
    - PowerShell: `& '…\ffmpeg.exe'`, single-quoted literals for anything not plainly safe, so `$ \` , ; @ {}` are never
      interpreted; dash tokens like `-c:v` / `-metadata:s:a:0` are quoted because PowerShell can split them. If a job uses
      a chapter file, the copied PowerShell text writes it first (`[IO.File]::WriteAllText`, UTF-8 without BOM, LF), so
      it runs on its own. With cmd, save the shown chapter text to the shown path first.
    - Limits: arguments containing `"` need PowerShell 7.3+ (5.1 mangles embedded quotes in native arguments); cmd
      expands `%NAME%` when such a variable exists; an argument that is exactly `--%` can't go through PowerShell
      (ffmpeg never needs one).
  - Run / Run all, Cancel and Esc work as before; Run is labelled "Run all" for several jobs.
- The queue log's "Running:" line uses the cmd syntax.
- **Drag and drop everywhere:**
  - While files are dragged over the window there's an accent frame, a banner ("Drop N items for “preset” –
    highlighted fields accept files"), and every droppable field is highlighted. The full-screen dark overlay is gone,
    because it hid the targets.
  - Input fields show "Drop a file here, or Browse…" or "Drop a folder here, or Browse…". Output folders and Settings
    are not drop targets.
  - After a drop, a 3 s note says "Added to …" or "Nothing in the drop fits …".
  - Folders dropped on single-file presets are expanded to their files in natural order, so the first video wins.
    Folder presets (Batch, Remux, Custom batch) still get the folder (`Mode::wants_folders`). Join takes every video.
  - Test hook: `FFENC_SIMULATE_DROP="path1|path2"` hovers those paths 3–7 s after start and then drops them. It's used
    for the Wine smoke test, where OS drags can't be scripted.
- **New built-in preset "Join / stitch files"** (`kind: "join"`, appended once to existing presets files via
  `known_builtins`):
  - File list with drag-and-drop, Add files/folder, natural sort, drag-grip reordering (egui DnD with an insertion
    line), ⬆/⬇/✖, Sort by name / modified date, Reverse, Clear. Per-file ffprobe in the background (duration, codec,
    size, fps, audio) and the total.
  - Lossless join with the concat demuxer: a temp list `%TEMP%\ffenc_concat_<id>.txt` with `ffconcat version 1.0` and
    `file '<path>'` lines, `'` → `'\''`. It's a runner side file, so it's deleted after the job; stale ones are cleaned
    at startup together with chapter files. Durations come from the container duration, because that's what the concat
    demuxer advances by.
  - Compatibility check (codec, size, fps, pix_fmt, audio codec/rate/channels/track count) with a "Switch to re-encode
    join" button.
  - Re-encode join with the concat filter: scale+pad to the first file, `setsar=1`, its fps (NTSC rates as exact
    fractions), yuv420p; `aresample` + `aformat` to its rate, mono or stereo; `anullsrc` silence or `color` black for
    missing streams. Uses the preset's x264/AAC settings, and gain/compressor after the concat.
  - One chapter per file, titled with the file stem. Cumulative times are rounded once on the running total, so no
    drift.
  - Output folder/name/suffix (`_joined`), container mp4/mkv/mov, faststart.
- `util::natural_cmp` / `natural_sort_paths` / `expand_dropped` / `same_path`.
- **App icon:** the user's film-strip artwork.
  - `assets/icon-source.png` is 128×128 RGBA. `assets/make_icon.py` builds `assets/app.ico` with 16, 24, 32, 48, 64,
    128 and 256 px PNG entries. Resampling is premultiplied-alpha Lanczos, with a light unsharp mask at ≤ 32 px, and
    transparency is kept.
  - The 256 px entry is a 2× upscale, so it's a little soft. At 16 px the sprocket holes merge into a brown tilted tile,
    while from 32 px up the frame and holes read clearly.
  - The same artwork (native 128 px) is the window/taskbar icon (`eframe::icon_data::from_png_bytes`), replacing the
    generated play-button icon.

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
