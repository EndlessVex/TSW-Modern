import { useState, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

/** Matches the Rust DownloadProgress struct from download.rs */
export interface DownloadProgress {
  bytes_downloaded: number;
  total_bytes: number;
  files_completed: number;
  files_total: number;
  speed_bps: number;
  current_file: string;
  phase: "checking" | "downloading" | "complete" | "error" | "repairing" | "bootstrapping";
  failed_files: number;
}

/** Matches the Rust PatchStatus struct from download.rs */
export interface PatchStatus {
  up_to_date: boolean;
  files_to_download: number;
  total_bytes: number;
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const k = 1024;
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), units.length - 1);
  const val = bytes / Math.pow(k, i);
  return `${val.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

function formatSpeed(bps: number): string {
  if (bps === 0) return "—";
  return `${formatBytes(bps)}/s`;
}

function formatEta(bytesRemaining: number, speedBps: number): string {
  if (speedBps <= 0 || bytesRemaining <= 0) return "—";
  const seconds = Math.ceil(bytesRemaining / speedBps);
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}m ${s}s`;
  }
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return `${h}h ${m}m`;
}

export default function PatchProgress() {
  const [progress, setProgress] = useState<DownloadProgress | null>(null);

  useEffect(() => {
    const unlisten = listen<DownloadProgress>("patch:progress", (event) => {
      setProgress(event.payload);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  if (!progress) {
    return null;
  }

  const { phase, bytes_downloaded, total_bytes, files_completed, files_total, speed_bps, failed_files } = progress;

  // Defensive clamp: never exceed 100% even if bytes_downloaded > total_bytes
  const pct = total_bytes > 0
    ? Math.min(100, Math.max(0, (bytes_downloaded / total_bytes) * 100))
    : 0;

  const bytesRemaining = Math.max(0, total_bytes - bytes_downloaded);

  return (
    <div className="patch-progress">
      <div className="patch-phase">
        {phase === "checking" && "Checking for updates…"}
        {phase === "bootstrapping" && "Downloading patch index…"}
        {phase === "downloading" && "Downloading updates…"}
        {phase === "complete" && "✓ Update complete"}
        {phase === "error" && "✗ Update failed"}
      </div>

      {/* Show error detail from current_file when in error state */}
      {phase === "error" && progress.current_file && (
        <div className="progress-error-detail">{progress.current_file}</div>
      )}

      {(phase === "downloading" || phase === "complete" || phase === "error") && (
        <>
          <div className="progress-bar-container">
            <div
              className={`progress-bar-fill ${phase === "error" ? "progress-bar-error" : ""} ${phase === "complete" ? "progress-bar-complete" : ""}`}
              style={{ width: `${pct}%` }}
            />
          </div>

          <div className="progress-stats">
            <span className="progress-pct">{pct.toFixed(1)}%</span>
            <span className="progress-bytes">
              {formatBytes(bytes_downloaded)} / {formatBytes(total_bytes)}
            </span>
          </div>

          <div className="progress-details">
            <span className="progress-speed">{formatSpeed(speed_bps)}</span>
            <span className="progress-eta">ETA: {formatEta(bytesRemaining, speed_bps)}</span>
            <span className="progress-files">
              {files_completed} / {files_total} files
            </span>
          </div>

          {failed_files > 0 && (
            <div className="progress-failed">
              {failed_files} file{failed_files !== 1 ? "s" : ""} failed
            </div>
          )}
        </>
      )}
    </div>
  );
}
