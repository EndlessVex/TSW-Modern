import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { store } from "./store";

type DxVersion = "dx9" | "dx11";
type TabId = "general" | "graphics" | "audio";

interface SettingsPanelProps {
  installPath: string | null;
  dxVersion: DxVersion;
  bundleMode: "full" | "minimum";
  verifying: boolean;
  repairing: boolean;
  onBack: () => void;
  onSelectDirectory: () => void;
  onDxChange: (version: DxVersion) => void;
  onBundleModeChange: (mode: "full" | "minimum") => void;
  onStartVerification: () => void;
}

const FALLBACK_RESOLUTIONS = [
  "800x600", "1024x768", "1280x720", "1280x800", "1280x1024",
  "1366x768", "1440x900", "1600x900", "1680x1050", "1920x1080",
  "2560x1440", "3840x2160",
];

const LANGUAGES = ["English", "French", "German"];
const DISPLAY_MODES = ["Fullscreen", "Windowed", "Borderless"];

export default function SettingsPanel({
  installPath,
  dxVersion,
  bundleMode,
  verifying,
  repairing,
  onBack,
  onSelectDirectory,
  onDxChange,
  onBundleModeChange,
  onStartVerification,
}: SettingsPanelProps) {
  const [activeTab, setActiveTab] = useState<TabId>("general");

  // Local state for settings loaded from / persisted to LazyStore
  const [textLanguage, setTextLanguage] = useState("English");
  const [audioLanguage, setAudioLanguage] = useState("English");
  const [resolution, setResolution] = useState("1920x1080");
  const [displayMode, setDisplayMode] = useState("Fullscreen");
  const [enableAudio, setEnableAudio] = useState(true);
  const [enableMusic, setEnableMusic] = useState(true);
  const [availableResolutions, setAvailableResolutions] = useState<string[]>(FALLBACK_RESOLUTIONS);

  // Query system display modes on mount
  useEffect(() => {
    async function loadDisplayModes() {
      try {
        const modes = await invoke<string[]>("get_display_modes");
        if (modes && modes.length > 0) {
          setAvailableResolutions(modes);
        }
      } catch {
        // Fall back to static list
      }
    }
    loadDisplayModes();
  }, []);

  // Load persisted settings on mount
  useEffect(() => {
    async function load() {
      try {
        const tl = await store.get<string>("text_language");
        if (tl) setTextLanguage(tl);

        const al = await store.get<string>("audio_language");
        if (al) setAudioLanguage(al);

        const res = await store.get<string>("resolution");
        if (res) {
          setResolution(res);
        } else {
          // Default to current screen resolution
          const screenRes = `${window.screen.width}x${window.screen.height}`;
          setResolution(screenRes);
        }

        const dm = await store.get<string>("display_mode");
        if (dm) setDisplayMode(dm);

        const ea = await store.get<boolean>("enable_audio");
        if (ea !== null && ea !== undefined) setEnableAudio(ea);

        const em = await store.get<boolean>("enable_music");
        if (em !== null && em !== undefined) setEnableMusic(em);
      } catch (err) {
        console.error("Failed to load settings:", err);
      }
    }
    load();
  }, []);

  // --- Change handlers ---

  async function handleTextLanguageChange(val: string) {
    setTextLanguage(val);
    await store.set("text_language", val);
    await store.save();
  }

  async function handleAudioLanguageChange(val: string) {
    setAudioLanguage(val);
    await store.set("audio_language", val);
    await store.save();
  }

  async function handleResolutionChange(val: string) {
    setResolution(val);
    await store.set("resolution", val);
    await store.save();
  }

  async function handleDisplayModeChange(val: string) {
    setDisplayMode(val);
    await store.set("display_mode", val);
    await store.save();
  }

  async function handleEnableAudioChange(checked: boolean) {
    setEnableAudio(checked);
    await store.set("enable_audio", checked);
    await store.save();
  }

  async function handleEnableMusicChange(checked: boolean) {
    setEnableMusic(checked);
    await store.set("enable_music", checked);
    await store.save();
  }

  function handleDxChange(version: DxVersion) {
    onDxChange(version);
  }

  const tabs: { id: TabId; label: string }[] = [
    { id: "general", label: "General" },
    { id: "graphics", label: "Graphics" },
    { id: "audio", label: "Audio" },
  ];

  return (
    <div className="settings-panel">
      {/* Tab bar */}
      <div className="settings-tab-bar">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            className={`settings-tab${activeTab === tab.id ? " settings-tab-active" : ""}`}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="settings-content">
        {activeTab === "general" && (
          <div className="settings-tab-pane">
            {/* Install path */}
            <div className="settings-field">
              <label className="settings-label">Install Directory</label>
              <div className="path-row">
                <span className="path-display">
                  {installPath ?? "No directory selected"}
                </span>
                <button className="btn btn-secondary" onClick={onSelectDirectory}>
                  Browse…
                </button>
              </div>
            </div>

            <div className="settings-divider" />

            {/* Text language */}
            <div className="settings-field">
              <label className="settings-label" htmlFor="text-lang">
                Text Language
              </label>
              <select
                id="text-lang"
                className="settings-select"
                value={textLanguage}
                onChange={(e) => handleTextLanguageChange(e.target.value)}
              >
                {LANGUAGES.map((l) => (
                  <option key={l} value={l}>{l}</option>
                ))}
              </select>
            </div>

            {/* Audio language */}
            <div className="settings-field">
              <label className="settings-label" htmlFor="audio-lang">
                Audio Language
              </label>
              <select
                id="audio-lang"
                className="settings-select"
                value={audioLanguage}
                onChange={(e) => handleAudioLanguageChange(e.target.value)}
              >
                {LANGUAGES.map((l) => (
                  <option key={l} value={l}>{l}</option>
                ))}
              </select>
            </div>

            <div className="settings-divider" />

            {/* Bundle mode */}
            <div className="settings-field">
              <label className="settings-label">Download Mode</label>
              <div className="dx-selector">
                <label className={`dx-option${bundleMode === "full" ? " dx-active" : ""}`}>
                  <input
                    type="radio"
                    name="bundle-settings"
                    value="full"
                    checked={bundleMode === "full"}
                    onChange={() => onBundleModeChange("full")}
                    disabled={verifying || repairing}
                  />
                  Full Client
                </label>
                <label className={`dx-option${bundleMode === "minimum" ? " dx-active" : ""}`}>
                  <input
                    type="radio"
                    name="bundle-settings"
                    value="minimum"
                    checked={bundleMode === "minimum"}
                    onChange={() => onBundleModeChange("minimum")}
                    disabled={verifying || repairing}
                  />
                  Minimum Client
                </label>
              </div>
            </div>

            <div className="settings-divider" />

            {/* Verify game files */}
            <div className="settings-field">
              <label className="settings-label">Maintenance</label>
              <button
                className="btn btn-verify"
                onClick={onStartVerification}
                disabled={verifying || repairing}
              >
                {verifying ? "Verifying…" : "Verify Game Files"}
              </button>
            </div>
          </div>
        )}

        {activeTab === "graphics" && (
          <div className="settings-tab-pane">
            {/* DX version */}
            <div className="settings-field">
              <label className="settings-label">DirectX Version</label>
              <div className="dx-selector">
                <label className={`dx-option${dxVersion === "dx11" ? " dx-active" : ""}`}>
                  <input
                    type="radio"
                    name="dx-settings"
                    value="dx11"
                    checked={dxVersion === "dx11"}
                    onChange={() => handleDxChange("dx11")}
                  />
                  DX11
                </label>
                <label className={`dx-option${dxVersion === "dx9" ? " dx-active" : ""}`}>
                  <input
                    type="radio"
                    name="dx-settings"
                    value="dx9"
                    checked={dxVersion === "dx9"}
                    onChange={() => handleDxChange("dx9")}
                  />
                  DX9
                </label>
              </div>
            </div>

            <div className="settings-divider" />

            {/* Resolution */}
            <div className="settings-field">
              <label className="settings-label" htmlFor="resolution">
                Resolution
              </label>
              <select
                id="resolution"
                className="settings-select"
                value={resolution}
                onChange={(e) => handleResolutionChange(e.target.value)}
              >
                {availableResolutions.map((r) => (
                  <option key={r} value={r}>{r}</option>
                ))}
                {/* Include current selection even if not in system list */}
                {!availableResolutions.includes(resolution) && (
                  <option key={resolution} value={resolution}>{resolution} (current)</option>
                )}
              </select>
              <span className="settings-hint">
                Advisory — the game validates on launch.
              </span>
            </div>

            <div className="settings-divider" />

            {/* Display mode */}
            <div className="settings-field">
              <label className="settings-label" htmlFor="display-mode">
                Display Mode
              </label>
              <select
                id="display-mode"
                className="settings-select"
                value={displayMode}
                onChange={(e) => handleDisplayModeChange(e.target.value)}
              >
                {DISPLAY_MODES.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </select>
            </div>
          </div>
        )}

        {activeTab === "audio" && (
          <div className="settings-tab-pane">
            {/* Enable audio */}
            <div className="settings-field">
              <label className="settings-toggle">
                <input
                  type="checkbox"
                  checked={enableAudio}
                  onChange={(e) => handleEnableAudioChange(e.target.checked)}
                />
                <span>Enable Audio</span>
              </label>
            </div>

            {/* Enable music */}
            <div className="settings-field">
              <label className="settings-toggle">
                <input
                  type="checkbox"
                  checked={enableMusic}
                  onChange={(e) => handleEnableMusicChange(e.target.checked)}
                />
                <span>Enable Music</span>
              </label>
            </div>

            <div className="settings-divider" />

            <p className="settings-note">
              These are launcher preferences. The game has its own audio settings in-game.
            </p>
          </div>
        )}
      </div>

      {/* Back button */}
      <button className="btn btn-secondary settings-back" onClick={onBack}>
        ← Back to Launcher
      </button>
    </div>
  );
}
