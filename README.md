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

- **ClientPatcher's "Repair Broken Data" re-downloads files unnecessarily.** If you run a repair through the official ClientPatcher, it will report downloading several GB of files. It isn't actually fixing anything. We verified that every file it "repairs" is already identical to what it re-downloads. The patcher's repair mode just does a full re-download pass regardless of whether anything is wrong, writing the data to new files and rebuilding the index. This is just how the patcher works unfortunately.
- **UI modding source files not included.** Funcom shipped Flash/ActionScript source for building custom UIs, but these aren't part of the core game files so we skip them during install. If you want them, [download the zip](https://github.com/EndlessVex/TSW-Modern/releases/download/game-data/Customized.zip) and extract it to `Data/Gui/` in your install folder.

## Not affiliated with Funcom

This is a community project. It downloads the same game files from Funcom's CDN — we just handle the install process differently.

## License

[MIT](LICENSE)
