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

- **Don't use ClientPatcher's "Repair Broken Data."** It will re-download ~4.6GB of textures that aren't actually broken, then do it again next time (~129MB). The game works fine without this.
- **UI modding source files not included.** The `Data/Gui/Customized/` folder (~109MB of Flash source for UI modders) is not downloaded yet. ClientPatcher's repair will fetch it, but also triggers the texture loop above.

## Not affiliated with Funcom

Community tool. Same files, same CDN, just faster.

## License

[MIT](LICENSE)
