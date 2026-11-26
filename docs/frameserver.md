# Frameserver (DebugMode FrameServer and AviSynth+)

[Back to the guide](README.md)

The **Frameserver (AviSynth script)** preset encodes straight from your video editor (Vegas, Premiere, After Effects and others), with no big intermediate file.

How it works: the editor serves the finished timeline through **DebugMode FrameServer**, which writes a small "signpost" `.avi` file. You pick that file. The app writes a short **AviSynth** script that opens the signpost, and FFmpeg encodes the script.

![The Frameserver form with C:\Videos\My Project.avi selected. Both checks at the top are OK: FFmpeg can read AviSynth scripts, and 64-bit AviSynth was found in Windows\System32. The signpost line reads 1280x720, 29.970 fps, 28042 frames, 00:15:35.67, audio 48000 Hz, 2 ch. Below are the colour format, colour matrix, signpost format, audio and trim options.](images/frameserver.png)

## What you need

- **DebugMode FrameServer** in your editor (see [Installing the frameserving tools](#installing-the-frameserving-tools)). Keep the project open and **serving until the encode has finished**; the signpost only works while the editor is serving.
- **AviSynth+ (64-bit)**, because the app's FFmpeg is 64-bit. The AviSynth+ installer can put both versions on your PC.
- An FFmpeg that can read AviSynth scripts. The usual Windows builds (BtbN and gyan.dev, including the one the app downloads) can.

The checks at the top of the form tell you, in plain words, whether each piece is there. When everything is ready, both lines start with **OK:**, as in the picture above: FFmpeg can read AviSynth scripts, and AviSynth+ was found (64-bit, matching FFmpeg). If something is missing, the line starts with **Problem:** and says what to install. **How frameserving works…** shows a short help text.

## Installing the frameserving tools

The app doesn't include AviSynth+ or DebugMode FrameServer. Both are free. Install them once, from their official pages:

| Tool | Official download |
| --- | --- |
| AviSynth+ | [github.com/AviSynth/AviSynthPlus/releases](https://github.com/AviSynth/AviSynthPlus/releases) |
| DebugMode FrameServer | [debugmode.com/frameserver.html](https://www.debugmode.com/frameserver.html) |

Close your video editor before you start.

### 1. AviSynth+ (64-bit)

1. Open the [AviSynth+ releases page](https://github.com/AviSynth/AviSynthPlus/releases). The version tested with Encode FFS is **3.7.5** ([release page](https://github.com/AviSynth/AviSynthPlus/releases/tag/v3.7.5)).
2. Under **Assets**, download **`AviSynthPlus_3.7.5_20250420.exe`**. That is the normal installer for 64-bit Windows.
   - If Windows later says a Visual C++ runtime DLL is missing, run **`AviSynthPlus_3.7.5_20250420_vcredist.exe`** from the same list instead. It is the same installer with the Microsoft runtime included.
   - Don't use the `_xp`, `_arm64`, macOS or `filesonly` downloads.
3. Run the installer. Pick the language, accept the licence and press **Next**.
4. **Select Destination Location**: the suggested folder is fine. You can choose another folder or drive if you like; the app finds AviSynth+ either way.
5. **Select Components**: keep **Recommended installation**. Make sure **AviSynth+ (x64)** is ticked. That is the part that matters, because the app's FFmpeg is 64-bit and needs the 64-bit AviSynth+. **AviSynth+ (x86)** (32-bit) can stay ticked; it does no harm.
6. Press **Next**, then **Install**, then **Finish**.

### 2. DebugMode FrameServer v7 (64-bit)

1. Open the [DebugMode FrameServer page](https://www.debugmode.com/frameserver.html) and click **Download the latest version (v7.0, 64-bit)**. This downloads `fssetup.exe`. Don't use the old 32-bit FrameServer 2.14 on the same page; it is for old 32-bit editors.
2. Run `fssetup.exe` and press **I Agree**.
3. In the list of components, **FrameServer Core** and **Dokan Library** are always installed. Tick the plug-in for your editor:
   - **Magix Vegas Plugin** for your Vegas version: **Vegas 23+**, **Vegas 22**, **Vegas 18 – 21** or **upto Vegas 17**.
   - **Adobe Premiere Pro/Elements Plugin** for Premiere.
4. **Select where to install DebugMode FrameServer Core Files**: the suggested folder is fine.
5. **Select where to install Magix Vegas Pro Plugin**: this must be **your Vegas program folder**, the one with `vegas*.exe` in it (for example `C:\Program Files\VEGAS\VEGAS Pro 22.0`). If the box shows a different folder, press **Browse…** and pick the Vegas folder. Otherwise Vegas won't show the plug-in.
6. Finish the installer, then start Vegas.

### 3. Make the signpost in Vegas

1. Use 8-bit or 16-bit audio in the project (**File › Properties › Audio**). FrameServer can't serve 24-bit or 32-bit audio.
2. Choose **File › Render As…**. Pick **DebugMode FrameServer** as the format, type a file name (this is the "signpost" `.avi`) and press **Render**.
3. In the FrameServer window that opens:
   - Leave audio on, so the signpost carries the sound. The app takes the audio from the signpost; you don't need a separate audio file.
   - For the video output, pick **RGB24**, **RGB32** or **YUY2**. RGB24 is a safe choice. The app's **Signpost format: Automatic** takes whichever of these the signpost serves and converts it in the script.
   - Leave network frameserving off.
4. Press **Next** to begin serving. Keep that window open until the encode in Encode FFS has finished.

In Premiere, export from the timeline (**File › Export**) and pick **DebugMode FrameServer** as the format.

### 4. Check that it works

Select **Frameserver (AviSynth script)** in Encode FFS. The two lines at the top of the form should both start with **OK:**:

- **OK: FFmpeg can read AviSynth scripts.**
- **OK: AviSynth found: …\System32\avisynth.dll (64-bit, matches FFmpeg).** The folder can be different if you installed elsewhere.

If a line starts with **Problem:**, it says what's wrong. **How to install…** next to it opens this section.

- **"This FFmpeg can't read AviSynth scripts"**: switch to the app's own FFmpeg (the FFmpeg menu at the top right, or **Settings › FFmpeg to use**), or install a standard Windows build with **Help › Check for FFmpeg updates…**.
- **"AviSynth doesn't seem to be installed"**: install AviSynth+ as in step 1, then restart Encode FFS.
- **"Only a 32-bit AviSynth was found"**: run the AviSynth+ installer again and make sure **AviSynth+ (x64)** is ticked.

Then pick the signpost and press **Test the script**. It reports the size, frame rate, length and audio. If it fails, check that Vegas is still serving.

## Step by step

1. In the editor, start DebugMode FrameServer and save the signpost `.avi`.
2. In Encode FFS, select **Frameserver (AviSynth script)** and pick the signpost (drop it or **Browse…**). The line under it shows what the editor is serving: size, frame rate, number of frames, length and audio.
3. Check the script options. The defaults suit most HD projects.
4. Optional: **Test the script** opens it with ffprobe and reports size, frame rate, length and audio. If it fails, it tells you why (the editor isn't serving, AviSynth+ is missing, the wrong bitness, and so on).
5. Choose the video and audio codecs and the output, then **Preview & run…** and **Run**.

## Script options

- **Colour format**: YUV 4:2:0 (`ConvertToYUV420()`) is the normal choice and plays everywhere. **Which one? (help)** explains the others.
- **Colour matrix**: **PC.709** by default, which keeps full range and is recommended for Vegas 8-bit projects. Use **Rec709** for 32-bit full-range Vegas projects. If dark scenes look grey and milky, the levels were probably squeezed twice.
- **Signpost format**: leave it on **Automatic** unless the picture looks skewed or the colours are odd.
- **Trim**: **Only part of the timeline** takes a first and last frame (0 = to the end). This trim is in frames, not seconds.
- **Keep the .avs file**: by default the script is a temporary file that is deleted after the job. You can keep it next to the signpost or in a folder you choose, or save it without encoding.
- You can edit the script by hand if you need to, and go back to the generated one at any time.

## Audio

The signpost carries the editor's audio as well as the picture, so you don't need a separate audio file.

- **Keep the audio** (default) encodes it with the codec you choose (AAC by default).
- **Keep it, converted to 16-bit** is worth trying if the audio fails.
- **No audio (video only)**.

The audio arrives uncompressed (PCM), so there is nothing to pass through: you get one normal audio encode. If the result is too quiet, raise the levels in the editor, or use the pre-gain and compressor in the audio options.

## Frame rate

The video keeps the signpost's frame rate. To change it, type a rate in **Output frame rate** in the Video section (for example `30` or `29.97`). FFmpeg drops or repeats frames, so the length and the audio stay in sync. This is the only frame rate control for frameserving; the script doesn't change the speed.

## Video

H.264 (default), HEVC or AV1, with the same settings as [Re-encode](single-file-presets.md#re-encode-video): CRF, speed, optional resize and crop, chapters, MP4 or MKV. For HEVC and AV1 the 10-bit option also makes the script hand over 10-bit video.

Next: [Create Slideshow](slideshow.md)
