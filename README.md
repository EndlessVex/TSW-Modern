# TSW Downloader

A downloader for **The Secret World**.

## Download

**[Latest Release](https://github.com/EndlessVex/TSW-Modern/releases/latest)** — grab the `.exe` and run it. No install needed.

## What it does

- Downloads The Secret World (~42GB) using parallel connections
- Decompresses and writes game resources to match the official install
- Shows live progress with accurate percentage and speed
- Picks up where it left off if interrupted
- Runs at low priority so your system stays usable

## Requirements

- Windows 10+ (x64)
- ~45GB free disk space
- Internet connection

## Known issues

- **ClientPatcher's "Repair Broken Data" will re-download textures unnecessarily.** Our downloader generates texture mip levels during install. The result is visually identical but not byte-for-byte the same as the original, so ClientPatcher's hash check sees them as corrupted. It'll re-download ~4.6GB the first time, then ~129MB on every subsequent repair. The textures work fine in-game — this is just a hash mismatch, not actual corruption.
- **UI modding source files not included.** Funcom shipped Flash/ActionScript source for building custom UIs, but these aren't part of the core game files so we skip them during install. If you want them, [download the zip](https://github.com/EndlessVex/TSW-Modern/releases/download/game-data/Customized.zip) and extract it to `Data/Gui/` in your install folder.

## Not affiliated with Funcom

This is a community project. It downloads the same game files from Funcom's CDN — we just handle the install process differently.

## License

[MIT](LICENSE)
