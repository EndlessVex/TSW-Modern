import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { store } from "./store";
import Header from "./Header";
import MainView from "./MainView";
import SettingsPanel from "./SettingsPanel";
import type { PatchStatus, DownloadProgress } from "./PatchProgress";
import "./App.css";

/** Matches the Rust InstallValidation struct from lib.rs */
interface InstallValidation {
  valid: boolean;
  version: string | null;
  rdb_count: number;
  message: string;
}

type DxVersion = "dx9" | "dx11";

function App() {
  // View switching
  const [currentView, setCurrentView] = useState<"main" | "settings">("main");

  // Shared state passed down to MainView
  const [installPath, setInstallPath] = useState<string | null>(null);
  const [validationResult, setValidationResult] = useState<InstallValidation | null>(null);
  const [dxVersion, setDxVersion] = useState<DxVersion>("dx11");
  const [launching, setLaunching] = useState(false);
  const [patching, setPatching] = useState(false);
  const [patchStatus, setPatchStatus] = useState<PatchStatus | null>(null);
  const [patchPhase, setPatchPhase] = useState<string | null>(null);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [verifying, setVerifying] = useState(false);
  const [verifyResult, setVerifyResult] = useState<number | null>(null);
  const [repairing, setRepairing] = useState(false);
  const [bundleMode, setBundleMode] = useState<"full" | "minimum">("full");
  const [installerDownloading, setInstallerDownloading] = useState(false);
  const [installerProgress, setInstallerProgress] = useState<{ bytes_downloaded: number; total_bytes: number } | null>(null);
  const [installerPhase, setInstallerPhase] = useState<string | null>(null);
  const [updateAvailable, setUpdateAvailable] = useState<{ version: string; date: string; update: import("@tauri-apps/plugin-updater").Update } | null>(null);
  const [updating, setUpdating] = useState(false);

  // Listen for patch:progress events to track patching and repair state
  useEffect(() => {
    const unlisten = listen<DownloadProgress>("patch:progress", (event) => {
      const { phase } = event.payload;
      setPatchPhase(phase);

      if (phase === "repairing") return;

      if (phase === "complete" || phase === "error") {
        setPatching(false);
        setRepairing(false);
        if (phase === "complete") {
          setPatchStatus({ up_to_date: true, files_to_download: 0, total_bytes: 0 });
        }
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Listen for installer:progress events
  useEffect(() => {
    const unlisten = listen<{ bytes_downloaded: number; total_bytes: number; phase: string }>("installer:progress", (event) => {
      const { bytes_downloaded, total_bytes, phase } = event.payload;
      setInstallerProgress({ bytes_downloaded, total_bytes });
      setInstallerPhase(phase);
      if (phase === "complete" || phase === "error") {
        setInstallerDownloading(false);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const validatePath = useCallback(async (path: string) => {
    try {
      const result = await invoke<InstallValidation>("validate_install_dir", { path });
      setValidationResult(result);
      setError(null);
      return result;
    } catch (err) {
      setError(String(err));
      setValidationResult(null);
      return null;
    }
  }, []);

  const checkForUpdates = useCallback(async (path: string) => {
    setCheckingUpdates(true);
    try {
      const status = await invoke<PatchStatus>("check_for_updates_cmd", { installPath: path });
      setPatchStatus(status);
      setPatchPhase(null);
      setError(null);
    } catch (err) {
      setError(`Update check failed: ${err}`);
    } finally {
      setCheckingUpdates(false);
    }
  }, []);

  // Load saved settings on startup
  useEffect(() => {
    async function loadSettings() {
      try {
        const savedPath = await store.get<string>("install_path");
        const savedDx = await store.get<DxVersion>("dx_version");

        if (savedDx === "dx9" || savedDx === "dx11") {
          setDxVersion(savedDx);
        }

        if (savedPath) {
          setInstallPath(savedPath);
          const result = await validatePath(savedPath);
          if (result?.valid) {
            await checkForUpdates(savedPath);
          }
        }

        // Load bundle mode from Tauri backend
        try {
          const mode = await invoke<string>("get_bundle_mode");
          if (mode === "full" || mode === "minimum") {
            setBundleMode(mode);
          }
        } catch {
          // Ignore — defaults to "full"
        }
      } catch (err) {
        console.error("Failed to load settings:", err);
      } finally {
        setLoading(false);
      }
    }

    loadSettings();
  }, [validatePath, checkForUpdates]);

  // Check for launcher updates after loading completes
  useEffect(() => {
    if (loading) return;
    async function checkLauncherUpdate() {
      try {
        const update = await check();
        if (update) {
          setUpdateAvailable({
            version: update.version,
            date: update.date ?? "",
            update,
          });
        }
      } catch {
        // Silently ignore — no releases published yet or dev environment
      }
    }
    checkLauncherUpdate();
  }, [loading]);

  // --- Action handlers passed to MainView ---

  async function handleDownloadInstaller() {
    if (installerDownloading) return;
    setInstallerDownloading(true);
    setInstallerPhase("downloading");
    setInstallerProgress(null);
    setError(null);
    try {
      await invoke("download_installer");
    } catch (err) {
      setError(`Installer download failed: ${err}`);
      setInstallerDownloading(false);
      setInstallerPhase("error");
    }
  }

  async function handleUpdateLauncher() {
    if (!updateAvailable || updating) return;
    setUpdating(true);
    try {
      await updateAvailable.update.downloadAndInstall();
      await relaunch();
    } catch (err) {
      setError(`Launcher update failed: ${err}`);
      setUpdating(false);
    }
  }

  async function handleSelectDirectory() {
    try {
      const selected = await open({
        directory: true,
        title: "Select TSW Install Directory",
      });

      if (selected) {
        setInstallPath(selected);
        setError(null);
        setPatchStatus(null);
        setPatchPhase(null);
        const result = await validatePath(selected);
        await store.set("install_path", selected);
        await store.save();
        if (result?.valid) {
          await checkForUpdates(selected);
        }
      }
    } catch (err) {
      setError(`Failed to open directory picker: ${err}`);
    }
  }

  async function handleDxChange(version: DxVersion) {
    setDxVersion(version);
    await store.set("dx_version", version);
    await store.save();
  }

  async function handleStartPatching() {
    if (!installPath || patching) return;
    setPatching(true);
    setPatchPhase("checking");
    setError(null);
    try {
      await invoke("start_patching", { installPath });
    } catch (err) {
      setError(`Patching failed: ${err}`);
      setPatching(false);
      setPatchPhase("error");
    }
  }

  async function handleStartVerification() {
    if (!installPath || verifying || repairing) return;
    setVerifying(true);
    setVerifyResult(null);
    setError(null);
    try {
      await invoke("start_verification", { installPath });
    } catch (err) {
      setError(`Verification failed: ${err}`);
      setVerifying(false);
    }
  }

  async function handleCancelVerification() {
    try {
      await invoke("cancel_verification");
    } catch (err) {
      setError(`Cancel failed: ${err}`);
    }
  }

  function handleVerifyComplete(corruptedCount: number) {
    setVerifying(false);
    setVerifyResult(corruptedCount);
  }

  async function handleRepair() {
    if (!installPath || repairing) return;
    setRepairing(true);
    setError(null);
    try {
      await invoke("repair_corrupted", { installPath });
    } catch (err) {
      setError(`Repair failed: ${err}`);
      setRepairing(false);
    }
  }

  async function handleBundleModeChange(mode: "full" | "minimum") {
    setBundleMode(mode);
    try {
      await invoke("set_bundle_mode", { mode });
    } catch (err) {
      setError(`Failed to set bundle mode: ${err}`);
    }
  }

  async function handleLaunch() {
    if (!installPath || !validationResult?.valid || launching || patching || verifying || repairing) return;
    setLaunching(true);
    setError(null);
    try {
      await invoke("launch_game", { installPath, dxVersion });
    } catch (err) {
      setError(`Launch failed: ${err}`);
    } finally {
      setTimeout(() => setLaunching(false), 2000);
    }
  }

  // Loading screen
  if (loading) {
    return (
      <div className="container">
        <p className="loading">Loading settings…</p>
      </div>
    );
  }

  return (
    <div className="app-layout">
      <Header
        currentView={currentView}
        onToggleSettings={() =>
          setCurrentView((v) => (v === "main" ? "settings" : "main"))
        }
      />
      <div className="app-content">
        {updateAvailable && (
          <div className="update-banner">
            <span>Launcher update v{updateAvailable.version} available</span>
            <button
              className="btn btn-update-launcher"
              onClick={handleUpdateLauncher}
              disabled={updating}
            >
              {updating ? "Updating…" : "Update Now"}
            </button>
          </div>
        )}
        {currentView === "main" ? (
          <MainView
            installPath={installPath}
            validationResult={validationResult}
            dxVersion={dxVersion}
            patchStatus={patchStatus}
            patching={patching}
            patchPhase={patchPhase}
            checkingUpdates={checkingUpdates}
            launching={launching}
            verifying={verifying}
            repairing={repairing}
            verifyResult={verifyResult}
            error={error}
            installerDownloading={installerDownloading}
            installerProgress={installerProgress}
            installerPhase={installerPhase}
            onSelectDirectory={handleSelectDirectory}
            onDownloadInstaller={handleDownloadInstaller}
            onStartPatching={handleStartPatching}
            onCheckForUpdates={() => installPath && checkForUpdates(installPath)}
            onStartVerification={handleStartVerification}
            onCancelVerification={handleCancelVerification}
            onVerifyComplete={handleVerifyComplete}
            onRepair={handleRepair}
            onLaunch={handleLaunch}
            onLaunchStart={() => setLaunching(true)}
            onLaunchEnd={() => setLaunching(false)}
            onError={setError}
          />
        ) : (
          <SettingsPanel
            installPath={installPath}
            dxVersion={dxVersion}
            bundleMode={bundleMode}
            verifying={verifying}
            repairing={repairing}
            onBack={() => setCurrentView("main")}
            onSelectDirectory={handleSelectDirectory}
            onDxChange={handleDxChange}
            onBundleModeChange={handleBundleModeChange}
            onStartVerification={handleStartVerification}
          />
        )}
      </div>
    </div>
  );
}

export default App;
