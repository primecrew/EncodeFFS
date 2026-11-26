# Frameserver (DebugMode FrameServer and AviSynth+)

[Back to the guide](README.md)

The **Frameserver (AviSynth script)** preset encodes straight from your video editor (Vegas, Premiere, After Effects and others), with no big intermediate file.

How it works: the editor serves the finished timeline through **DebugMode FrameServer**, which writes a small "signpost" `.avi` file. You pick that file. The app writes a short **AviSynth** script that opens the signpost, and FFmpeg encodes the script.

![The Frameserver form with C:\Videos\My Project.avi selected. Both checks at the top are OK: FFmpeg can read AviSynth scripts, and 64-bit AviSynth was found in Windows\System32. The signpost line reads 1280x720, 29.970 fps, 28042 frames, 00:15:35.67, audio 48000 Hz, 2 ch. Below are the colour format, colour matrix, signpost format, audio and trim options.](images/frameserver.png)

## What you need

- **DebugMode FrameServer** in your editor. Keep the project open and **serving until the encode has finished**; the signpost only works while the editor is serving.
- **AviSynth+ (64-bit)**, because the app's FFmpeg is 64-bit. The AviSynth+ installer can put both versions on your PC.
- An FFmpeg that can read AviSynth scripts. The usual Windows builds (BtbN and gyan.dev, including the one the app downloads) can.

The checks at the top of the form tell you, in plain words, whether each piece is there. When everything is ready, both lines start with **OK:**, as in the picture above: FFmpeg can read AviSynth scripts, and AviSynth+ was found (64-bit, matching FFmpeg). If something is missing, the line starts with **Problem:** and says what to install. **How frameserving works…** shows a short help text.

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
