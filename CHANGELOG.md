# Changelog

User-facing history for the retired **Experimental UI** edition (`ui-experimental`, `FFmpegEncoder-Experimental.exe`). This branch is not developed further. Developer notes are in [NOTES.md](NOTES.md).

Use the **stable** edition on `master` for the supported build.

## 0.6.0-dev.4

- Encoding Finished popup when the queue is idle, with Go to file.
- The Bold text option was removed; older settings files that still have it load fine.

## 0.6.0-dev.3

- Remembers dialog sizes; more reliable panel restore.
- Configurable folder for the app-managed copy of FFmpeg, with move, verify and rollback.

## 0.6.0-dev.2

- Remembers window layout (position, size, maximized state, screen, panel sizes, open sections), with validation when a screen is missing. Remember / Reset in Settings.

## 0.6.0-dev.1

- First Experimental UI edition: separate exe, data folder and installer id, so it can sit next to stable. Import from stable on first start (read only). UI cleanup, splash screen, NSIS installer and portable zip.

## 0.5.1 and earlier

This branch was taken from stable 0.5.1. For 0.5.1–0.1.0 see the changelog on `master`.
