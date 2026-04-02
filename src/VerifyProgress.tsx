import { useState, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

/** Matches the Rust VerifyProgress struct from verify.rs */
export interface VerifyProgressData {
  entries_checked: number;
  entries_total: number;
  corrupted_count: number;
  bytes_scanned: number;
  current_file: string;
  phase: string;
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const k = 1024;
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), units.length - 1);
  const val = bytes / Math.pow(k, i);
  return `${val.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

/** Parse the phase string — Rust sends "scanning", "complete", "cancelled", or "error: <msg>" */
function phaseLabel(phase: string): string {
  if (phase === "scanning") return "Scanning game files…";
  if (phase === "complete") return "✓ Verification complete";
  if (phase === "cancelled") return "Verification cancelled";
  if (phase.startsWith("error")) return "✗ Verification failed";
  return phase;
}

function phaseClass(phase: string): string {
  if (phase === "complete") return "verify-phase-complete";
  if (phase === "cancelled") return "verify-phase-cancelled";
  if (phase.startsWith("error")) return "verify-phase-error";
  return "";
}

interface VerifyProgressProps {
  onComplete?: (corruptedCount: number) => void;
  onRepairClick?: () => void;
  showRepair?: boolean;
}

export default function VerifyProgress({ onComplete, onRepairClick, showRepair }: VerifyProgressProps) {
  const [progress, setProgress] = useState<VerifyProgressData | null>(null);

  useEffect(() => {
    const unlisten = listen<VerifyProgressData>("verify:progress", (event) => {
      setProgress(event.payload);

      // Notify parent when verification completes
      if (event.payload.phase === "complete" && onComplete) {
        onComplete(event.payload.corrupted_count);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [onComplete]);

  if (!progress) {
    return null;
  }

  const { phase, entries_checked, entries_total, corrupted_count, bytes_scanned, current_file } = progress;

  const pct = entries_total > 0
    ? Math.min(100, Math.max(0, (entries_checked / entries_total) * 100))
    : 0;

  const isTerminal = phase === "complete" || phase === "cancelled" || phase.startsWith("error");

  return (
    <div className="verify-progress">
      <div className={`verify-phase ${phaseClass(phase)}`}>
        {phaseLabel(phase)}
      </div>

      <div className="progress-bar-container">
        <div
          className={`progress-bar-fill ${phase === "complete" ? "progress-bar-complete" : ""} ${phase.startsWith("error") ? "progress-bar-error" : ""}`}
          style={{ width: `${pct}%` }}
        />
      </div>

      <div className="progress-stats">
        <span className="progress-pct">{pct.toFixed(1)}%</span>
        <span className="progress-bytes">
          {entries_checked.toLocaleString()} / {entries_total.toLocaleString()} entries
        </span>
      </div>

      <div className="progress-details">
        <span className="progress-bytes">{formatBytes(bytes_scanned)} scanned</span>
        {corrupted_count > 0 && (
          <span className="corrupted-count">
            {corrupted_count} corrupted
          </span>
        )}
      </div>

      {!isTerminal && current_file && (
        <div className="verify-current-file" title={current_file}>
          {current_file}
        </div>
      )}

      {/* Repair button shown when verification completes with corruption */}
      {showRepair && phase === "complete" && corrupted_count > 0 && (
        <button className="btn btn-repair" onClick={onRepairClick}>
          Repair {corrupted_count} corrupted file{corrupted_count !== 1 ? "s" : ""}
        </button>
      )}
    </div>
  );
}
