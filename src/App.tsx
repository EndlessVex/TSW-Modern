import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { store } from "./store";
import Header from "./Header";
import MainView from "./MainView";
import type { PatchStatus, DownloadProgress } from "./PatchProgress";
import "./App.css";

/** Matches the Rust InstallValidation struct from lib.rs */
interface InstallValidation {
  valid: boolean;
  version: string | null;
  rdb_count: number;
  message: string;
}


function App() {
  // Shared state
  const [installPath, setInstallPath] = useState<string | null>(null);
  const installPathRef = useRef<string | null>(null);
  const [validationResult, setValidationResult] = useState<InstallValidation | null>(null);
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
  const [installerDownloading, setInstallerDownloading] = useState(false);
  const [installerProgress, setInstallerProgress] = useState<{ bytes_downloaded: number; total_bytes: number } | null>(null);
  const [installerPhase, setInstallerPhase] = useState<string | null>(null);
  const [freshInstallDir, setFreshInstallDir] = useState<string>(
    "C:\\Program Files (x86)\\Funcom\\The Secret World"
  );

  // Keep ref in sync for use in event listeners (avoids stale closure)
  useEffect(() => { installPathRef.current = installPath; }, [installPath]);

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
          // Re-validate install path after download completes
          const currentPath = installPathRef.current;
          if (currentPath) {
            validatePath(currentPath);
          }
        }
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

  // Listen for installer:progress events
  useEffect(() => {
    const unlisten = listen<{ bytes_downloaded: number; total_bytes: number; phase: string }>("installer:progress", async (event) => {
      const { bytes_downloaded, total_bytes, phase } = event.payload;
      setInstallerProgress({ bytes_downloaded, total_bytes });

      if (phase === "installing") {
        setInstallerPhase("installing");
      } else if (phase.startsWith("complete:")) {
        // Installer finished and auto-detected install path
        const detectedPath = phase.substring("complete:".length);
        setInstallerDownloading(false);
        setInstallPath(detectedPath);
        await store.set("install_path", detectedPath);
        await store.save();
        const result = await validatePath(detectedPath);
        if (result?.valid) {
          // Clear installer phase so the fresh install section hides
          // and the normal install directory section shows
          setInstallerPhase(null);
          await checkForUpdates(detectedPath);
        } else {
          setInstallerPhase("complete");
        }
      } else if (phase === "complete") {
        setInstallerDownloading(false);
        // No path detected — try auto-detect ourselves
        try {
          const detected = await invoke<string | null>("auto_detect_install_dir");
          if (detected) {
            setInstallPath(detected);
            await store.set("install_path", detected);
            await store.save();
            const result = await validatePath(detected);
            if (result?.valid) {
              setInstallerPhase(null);
              await checkForUpdates(detected);
            } else {
              setInstallerPhase("complete");
            }
          } else {
            setInstallerPhase("complete");
          }
        } catch {
          setInstallerPhase("complete");
        }
      } else if (phase === "error") {
        setInstallerPhase("error");
        setInstallerDownloading(false);
      } else {
        setInstallerPhase(phase);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [validatePath, checkForUpdates]);

  // Load saved settings on startup
  useEffect(() => {
    async function loadSettings() {
      try {
        const savedPath = await store.get<string>("install_path");

        let pathToUse = savedPath;

        // If no saved path, try auto-detection
        if (!pathToUse) {
          try {
            const detected = await invoke<string | null>("auto_detect_install_dir");
            if (detected) {
              pathToUse = detected;
              await store.set("install_path", detected);
              await store.save();
            }
          } catch {
            // Auto-detect failed silently — user can still browse manually
          }
        }

        if (pathToUse) {
          setInstallPath(pathToUse);
          const result = await validatePath(pathToUse);
          if (result?.valid) {
            await checkForUpdates(pathToUse);
          }
        }

      } catch (err) {
        console.error("Failed to load settings:", err);
      } finally {
        setLoading(false);
      }
    }

    loadSettings();
  }, [validatePath, checkForUpdates]);


  // --- Action handlers passed to MainView ---

  async function handleDownloadInstaller() {
    if (patching) return;

    // Set the install path and save it
    const targetDir = freshInstallDir;
    setInstallPath(targetDir);
    await store.set("install_path", targetDir);
    await store.save();

    // Validate — write_static_files will create LocalConfig.xml
    // so we'll validate after the install starts

    setPatching(true);
    setPatchPhase("checking");
    setInstallerPhase(null);
    setError(null);
    try {
      await invoke("start_full_install", { installPath: targetDir });
    } catch (err) {
      setError(`Install failed: ${err}`);
      setPatching(false);
      setPatchPhase("error");
    }
  }

  async function handleChooseFreshInstallDir() {
    try {
      const selected = await open({
        directory: true,
        title: "Choose where to install The Secret World",
        defaultPath: freshInstallDir,
      });
      if (selected) {
        setFreshInstallDir(selected as string);
      }
    } catch {
      // User cancelled
    }
  }




  async function handleStartPatching() {
    if (!installPath || patching) return;
    setPatching(true);
    setPatchPhase("checking");
    setError(null);
    try {
      await invoke("start_full_install", { installPath });
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


  async function handleLaunch() {
    if (!installPath || !validationResult?.valid || launching || patching || verifying || repairing) return;
    setLaunching(true);
    setError(null);
    try {
      await invoke("launch_patcher", { installPath });
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
      <Header />
      <div className="app-content">
        <MainView
          installPath={installPath}
          validationResult={validationResult}
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
          freshInstallDir={freshInstallDir}
          onDownloadInstaller={handleDownloadInstaller}
          onChooseFreshInstallDir={handleChooseFreshInstallDir}
          onStartPatching={handleStartPatching}
          onCheckForUpdates={() => installPath && checkForUpdates(installPath)}
          onStartVerification={handleStartVerification}
          onCancelVerification={handleCancelVerification}
          onVerifyComplete={handleVerifyComplete}
          onRepair={handleRepair}
          onLaunch={handleLaunch}
          onError={setError}
        />
      </div>
    </div>
  );
}

export default App;
