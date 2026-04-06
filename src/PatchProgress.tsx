import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Matches the Rust DownloadProgress struct from download.rs */
export interface DownloadProgress {
  bytes_downloaded: number;
  total_bytes: number;
  files_completed: number;
  files_total: number;
  speed_bps: number;
  current_file: string;
  phase: "checking" | "downloading" | "patching" | "complete" | "error" | "repairing" | "bootstrapping";
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

function formatTime(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  return `${h.toString().padStart(2, "0")}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

interface PatchProgressProps {
  /** "bar" renders in the status bar zone (like original ClientPatcher). */
  layout?: "bar" | "panel";
}

export default function PatchProgress({ layout = "panel" }: PatchProgressProps) {
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [paused, setPaused] = useState(false);
  const [smoothedSpeed, setSmoothedSpeed] = useState(0);
  const [elapsedSec, setElapsedSec] = useState(0);

  useEffect(() => {
    const unlisten = listen<DownloadProgress>("patch:progress", (event) => {
      setProgress(event.payload);
      const alpha = 0.15;
      setSmoothedSpeed(prev => {
        const raw = event.payload.speed_bps;
        if (prev === 0) return raw;
        return Math.round(alpha * raw + (1 - alpha) * prev);
      });
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Elapsed time counter
  useEffect(() => {
    if (!progress || (progress.phase !== "downloading" && progress.phase !== "patching")) return;
    if (paused) return;
    const interval = setInterval(() => setElapsedSec(s => s + 1), 1000);
    return () => clearInterval(interval);
  }, [progress?.phase, paused]);

  // Keyboard controls: ESC to pause/cancel, Space to resume
  useEffect(() => {
    if (!progress || (progress.phase !== "downloading" && progress.phase !== "patching" && progress.phase !== "checking" && progress.phase !== "bootstrapping")) return;

    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        if (paused) {
          // Second ESC while paused → cancel
          invoke("cancel_patching");
          setPaused(false);
        } else {
          // First ESC → pause
          invoke("pause_patching");
          setPaused(true);
        }
      } else if (e.key === " " && paused) {
        e.preventDefault();
        invoke("resume_patching");
        setPaused(false);
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [progress?.phase, paused]);

  if (!progress) {
    return null;
  }

  const { phase, bytes_downloaded, total_bytes, files_completed, files_total } = progress;

  // During download: progress = bytes downloaded / total bytes
  // During patching tail: progress = files completed / files total
  const pct = phase === "patching"
    ? (files_total > 0 ? Math.min(100, Math.max(0, (files_completed / files_total) * 100)) : 100)
    : (total_bytes > 0 ? Math.min(100, Math.max(0, (bytes_downloaded / total_bytes) * 100)) : 0);

  // ─── Bar layout: status bar zone between content and tabs ───
  if (layout === "bar") {
    const phaseLabel =
      phase === "checking" ? "Checking for updates..." :
      phase === "bootstrapping" ? "Downloading patch index..." :
      phase === "downloading" ? "Downloading game files" :
      phase === "patching" ? "Processing remaining files..." :
      phase === "complete" ? "Patching complete" :
      phase === "error" ? "Patching failed" :
      phase === "repairing" ? "Repairing files..." : "";

    return (
      <div className="status-bar-progress">
        <div className="status-bar-info">
          <span className="status-bar-stats">
            {phase === "downloading" && (
              <>
                {formatBytes(bytes_downloaded)}/{formatBytes(total_bytes)}
                {" - "}
                {files_completed}/{files_total}
                {" - "}
                {formatSpeed(smoothedSpeed)}
                {" - "}
                {formatTime(elapsedSec)}
              </>
            )}
            {phase === "patching" && (
              <>
                {files_completed}/{files_total} files
                {" - "}
                {formatTime(elapsedSec)}
              </>
            )}
            {phase === "complete" && "Download complete"}
            {phase === "error" && (progress.current_file || "Download failed")}
          </span>
          <span className="status-bar-phase">
            {phaseLabel}
            {(phase === "downloading" || phase === "patching") && !paused && (
              <span className="status-bar-hint">  Press ESC to pause.</span>
            )}
            {(phase === "downloading" || phase === "patching") && paused && (
              <span className="status-bar-hint">  Paused — Space to resume, ESC to cancel.</span>
            )}
          </span>
        </div>
        {(phase === "downloading" || phase === "patching" || phase === "complete" || phase === "error") && (
          <div className="status-bar-track">
            <div
              className={`status-bar-fill ${phase === "complete" ? "status-bar-fill-complete" : ""} ${phase === "error" ? "status-bar-fill-error" : ""} ${paused ? "status-bar-fill-paused" : ""}`}
              style={{ width: `${pct}%` }}
            />
          </div>
        )}
      </div>
    );
  }

  // ─── Panel layout (fallback, not currently used) ───
  return null;
}
