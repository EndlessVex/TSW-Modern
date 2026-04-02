import { LazyStore } from "@tauri-apps/plugin-store";

/** Shared settings store — used by App.tsx (install_path, dx_version) and LoginForm (saved_username, remember_me) */
export const store = new LazyStore("settings.json");
