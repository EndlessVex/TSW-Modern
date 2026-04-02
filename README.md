# TSW Modern Launcher

A modern replacement launcher for **The Secret World** (the original MMO, not Secret World Legends).

Replaces Funcom's broken `ClientPatcher.exe` with a fast, reliable alternative built with [Tauri v2](https://v2.tauri.app/) (Rust backend + web frontend).

## Why?

The original launcher has two fatal flaws:
- **Downloads degrade to 0 bytes/sec** over time, forcing restarts
- **Progress bar exceeds 100%**, making it impossible to know actual status

This launcher fixes both — downloads never degrade, progress never lies.

## Features

- **Reliable Patching** — Connection pooling, parallel downloads (8 concurrent), exponential backoff retry, resume on restart
- **Honest Progress** — Total size computed upfront, progress bar capped at 100%, real-time speed and ETA
- **File Integrity** — Verify all game files against server manifest, repair corrupted resources
- **Bundle Management** — Choose minimum client (~15GB) or full client (~43GB)
- **Account Login** — Log in from the launcher with Remember Me (username only, password never stored)
- **Game Launch** — DX9 or DX11 with user preference
- **Settings Panel** — Resolution, display mode, DX version, language, audio
- **Community News** — Reddit feed from r/TheSecretWorld
- **Fresh Install** — Downloads the TSW installer for new players
- **Auto-Update** — Checks GitHub Releases on startup

## Tech Stack

- **Backend:** Rust (Tauri v2)
- **Frontend:** React + TypeScript + Vite
- **Patching:** Custom binary parsers for le.idx (IBDR) and RDBHashIndex.bin (RDHI) formats
- **Downloads:** reqwest with connection pooling, tokio::sync::Semaphore for concurrency
- **Persistence:** tauri-plugin-store (LazyStore)
- **Auto-Update:** tauri-plugin-updater (GitHub Releases)

## Building

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) (v18+)
- Linux: `libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev` and related dev packages

### Development

```bash
npm install
npm run tauri dev
```

### Production Build

```bash
npm run tauri build
```

## Project Structure

```
src/                    # React frontend
  App.tsx               # Layout shell + state management
  MainView.tsx          # Primary launcher view
  LoginForm.tsx         # Account authentication
  SettingsPanel.tsx      # Tabbed settings (General/Graphics/Audio)
  PatchProgress.tsx      # Download progress display
  VerifyProgress.tsx     # Integrity check progress
  NewsFeed.tsx          # Reddit community feed
  store.ts              # Shared LazyStore singleton

src-tauri/src/          # Rust backend
  lib.rs                # Tauri commands + plugin registration
  rdb.rs                # le.idx + RDBHashIndex.bin binary parsers
  config.rs             # LocalConfig.xml parser
  download.rs           # Download engine (parallel, retry, resume)
  verify.rs             # Integrity verification + repair + bundle management
```

## License

MIT
