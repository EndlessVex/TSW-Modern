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

**ClientPatcher's "Repair Broken Data" re-downloads textures every time.** If you run the repair tool inside ClientPatcher.exe after installing with this downloader, it will flag around 18,000 textures as corrupted and re-download ~4.6GB of data. Those textures are not actually broken — the game renders them correctly. The repair tool checks MD5 hashes against the original distribution, and our generated mip levels produce valid but non-identical hashes. Running repair again after that will download another ~129MB for the same reason. This loop never resolves because every re-encode produces slightly different compression output.

**The UI modding SDK is not included.** The original installer ships 747 Flash/ActionScript source files in `Data/Gui/Customized/` (~109MB) for players who want to build custom UI mods. Our downloader doesn't include these yet. If you need them, running ClientPatcher's repair will fetch them along with the texture re-downloads mentioned above.

## Not affiliated with Funcom

Community tool. Same files, same CDN, just faster.

## License

[MIT](LICENSE)
