# Troubleshooting and FAQ

[Back to the guide](README.md)

## "FFmpeg not found"

The top right of the window says **FFmpeg not found** when there is no working FFmpeg. Use **Help › Locate FFmpeg folder…** to pick the folder with `ffmpeg.exe` and `ffprobe.exe`, or download a copy from the same dialog. See [Getting started](getting-started.md#first-start-finding-ffmpeg).

## The Source panel says ffprobe isn't found

`ffprobe.exe` must be in the same folder as `ffmpeg.exe`. Most FFmpeg downloads include both.

## A codec says "(not in your FFmpeg)"

Your FFmpeg doesn't include that encoder. Pick it anyway and press **Use the app's own FFmpeg for …**: the app downloads its own copy (or uses it if it's already there) for those jobs. Your own FFmpeg folder is left alone. **Help › Check for FFmpeg updates…** works too.

## A job failed

- The **Log** at the bottom says what went wrong.
- FFmpeg's own log is kept as `ffmpeg_error_<name>.log` in the output folder.
- Nothing half-written is left under the real file name. The unfinished temporary file is deleted.
- **Copy** in the run confirmation gives you the exact command, so you can run it in PowerShell or Command Prompt to look at it more closely.

## The output file already exists

The run confirmation asks: **Keep both** (adds ` (2)` to the new name), **Overwrite** or **Skip this file**. Set a fixed answer in **Settings › When the output file exists**.

## Frameserving doesn't work

- Keep the editor **serving** until the encode has finished.
- Install **64-bit AviSynth+**. The checks at the top of the Frameserver form say what is missing.
- Use **Test the script** for a plain explanation of what went wrong.
- Audio problems: try **Keep it, converted to 16-bit**.

See [Frameserver](frameserver.md).

## Dark scenes look grey or washed out

The levels were probably squeezed from full range to TV range twice. In Re-encode, leave **Convert full-range to limited-range** off for normal captures (OBS and similar). For frameserving from Vegas, use **PC.709** for 8-bit projects and **Rec709** for 32-bit full-range projects.

## Which codec should I use?

- **H.264**: plays on almost anything and encodes quickly. The default.
- **HEVC**: often 30–50 % smaller at a similar look; most recent TVs and phones play it.
- **AV1**: often smaller still, but slower to encode and needs newer hardware to play smoothly.

A short test on the screen you'll actually watch is the best check. Hover the codec or the CRF **(?)** for more.

## GPU encoding is greyed out or missing

Open **File › Settings › Encoding hardware** and press **Re-check**. The GPU encoders need a recent graphics driver and an FFmpeg built with them (the app's download has them).

## The text is too small or too big

Use **View › Text size**, Ctrl + plus / minus, or the slider in Settings › Appearance. Ctrl+0 goes back to 100 %.

## Where are my settings and presets?

See [Settings › Where things are stored](settings.md#where-things-are-stored). **Help › Open settings file** opens the settings file.

## Does uninstalling remove my settings?

No, unless you tick **Remove my settings and presets** in the uninstaller. Your own FFmpeg folder is never removed.
