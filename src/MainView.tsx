import PatchProgress, { type PatchStatus } from "./PatchProgress";
import VerifyProgress from "./VerifyProgress";
import NewsFeed from "./NewsFeed";


interface MainViewProps {
  installPath: string | null;
  validationResult: { valid: boolean; version: string | null; rdb_count: number; message: string } | null;
  patchStatus: PatchStatus | null;
  patching: boolean;
  patchPhase: string | null;
  checkingUpdates: boolean;
  launching: boolean;
  verifying: boolean;
  repairing: boolean;
  verifyResult: number | null;
  error: string | null;
  installerDownloading: boolean;
  installerProgress: { bytes_downloaded: number; total_bytes: number } | null;
  installerPhase: string | null;
  freshInstallDir: string;
  onSelectDirectory: () => void;
  onDownloadInstaller: () => void;
  onChooseFreshInstallDir: () => void;
  onStartPatching: () => void;
  onCheckForUpdates: () => void;
  onStartVerification: () => void;
  onCancelVerification: () => void;
  onVerifyComplete: (corruptedCount: number) => void;
  onRepair: () => void;
  onLaunch: () => void;
  onError: (err: string | null) => void;
}

/** Simple byte formatter for the status line */
function formatBytesSimple(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const k = 1024;
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), units.length - 1);
  const val = bytes / Math.pow(k, i);
  return `${val.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

export default function MainView({
  installPath,
  validationResult,
  patchStatus,
  patching,
  patchPhase,
  checkingUpdates,
  launching,
  verifying,
  repairing,
  verifyResult,
  error,
  installerDownloading,
  installerProgress,
  installerPhase,
  freshInstallDir,
  onSelectDirectory,
  onDownloadInstaller,
  onChooseFreshInstallDir,
  onStartPatching,
  onCheckForUpdates,
  onStartVerification,
  onCancelVerification,
  onVerifyComplete,
  onRepair,
  onLaunch,
  onError,
}: MainViewProps) {
  const isValid = validationResult?.valid === true;
  const needsUpdate = patchStatus !== null && !patchStatus.up_to_date;
  const isUpToDate = patchStatus !== null && patchStatus.up_to_date;

  return (
    <>

      {/* Fresh Install Section — shown when no valid install detected */}
      {!isValid && (
        <section className="section fresh-install-section">
          <label className="section-label">Get Started</label>

          {installerPhase === "complete" ? (
            <div className="installer-complete">
              <span className="validation-icon">✓</span>
              <span>Installation complete! {installPath ? "Install directory detected automatically." : "Click Browse above to select your install directory."}</span>
            </div>
          ) : installerPhase === "installing" ? (
            <div className="installer-progress">
              <div className="patch-phase">Installing The Secret World…</div>
              <div className="progress-stats">
                <span className="progress-pct">Please wait — this may take a few minutes</span>
              </div>
            </div>
          ) : installerPhase === "error" ? (
            <div className="installer-error">
              <span className="validation-icon">✗</span>
              <span>Download failed. Please try again.</span>
            </div>
          ) : installerDownloading ? (
            <div className="installer-progress">
              <div className="patch-phase">Downloading installer…</div>
              <div className="progress-bar-container">
                <div
                  className="progress-bar-fill"
                  style={{
                    width: installerProgress && installerProgress.total_bytes > 0
                      ? `${Math.min(100, (installerProgress.bytes_downloaded / installerProgress.total_bytes) * 100)}%`
                      : "0%",
                  }}
                />
              </div>
              {installerProgress && installerProgress.total_bytes > 0 && (
                <div className="progress-stats">
                  <span className="progress-pct">
                    {Math.round((installerProgress.bytes_downloaded / installerProgress.total_bytes) * 100)}%
                  </span>
                  <span className="progress-bytes">
                    {formatBytesSimple(installerProgress.bytes_downloaded)} / {formatBytesSimple(installerProgress.total_bytes)}
                  </span>
                </div>
              )}
            </div>
          ) : patching || patchPhase === "complete" || patchPhase === "error" ? (
            <div className="install-progress-section">
              <PatchProgress />
            </div>
          ) : (
            <div className="fresh-install-controls">
              <div className="fresh-install-dir">
                <span className="fresh-install-dir-label">Install to:</span>
                <span className="fresh-install-dir-path">{freshInstallDir}</span>
                <button className="btn btn-secondary btn-small" onClick={onChooseFreshInstallDir}>
                  Change
                </button>
              </div>
              <button className="btn btn-install" onClick={onDownloadInstaller}>
                Download &amp; Install TSW
              </button>
              <div className="fresh-install-divider">
                <span>or</span>
              </div>
              <button className="btn btn-secondary" onClick={onSelectDirectory}>
                I already have the game installed
              </button>
            </div>
          )}
        </section>
      )}

      {/* Patch Status Section */}
      {isValid && (
        <section className="section">
          <label className="section-label">Patch Status</label>

          {checkingUpdates && (
            <div className="patch-status patch-status-checking">
              Checking for updates…
            </div>
          )}

          {!checkingUpdates && isUpToDate && !patching && patchPhase !== "complete" && (
            <div className="patch-status patch-status-ok">
              <span className="validation-icon">✓</span>
              <span>Game is up to date</span>
            </div>
          )}

          {!checkingUpdates && needsUpdate && !patching && (
            <div className="patch-status patch-status-update">
              <span>
                {patchStatus.files_to_download > 0
                  ? `${patchStatus.files_to_download} files need updating (${formatBytesSimple(patchStatus.total_bytes)})`
                  : "Game files need to be downloaded"}
              </span>
              <button className="btn btn-update" onClick={onStartPatching}>
                {patchStatus.files_to_download > 0 ? "Update" : "Download Game Files"}
              </button>
            </div>
          )}

          {/* Progress component — visible during and after patching */}
          {(patching || patchPhase === "complete" || patchPhase === "error") && (
            <PatchProgress />
          )}

          {!checkingUpdates && !patching && patchStatus === null && (
            <button className="btn btn-secondary" onClick={onCheckForUpdates}>
              Check for Updates
            </button>
          )}
        </section>
      )}

      {/* Community News Section */}
      {isValid && (
        <section className="section">
          <label className="section-label">Community News</label>
          <NewsFeed />
        </section>
      )}

      {/* Maintenance Section — Verify & Bundle Mode */}
      {isValid && (
        <section className="section verify-section">
          <label className="section-label">Maintenance</label>

          {/* Verify / Cancel Buttons */}
          <div className="verify-actions">
            {!verifying && (
              <button
                className="btn btn-verify"
                onClick={onStartVerification}
                disabled={repairing}
              >
                Verify Game Files
              </button>
            )}
            {verifying && (
              <button className="btn btn-cancel" onClick={onCancelVerification}>
                Cancel Verification
              </button>
            )}
          </div>

          {/* Verification progress — visible during and after verification */}
          {(verifying || verifyResult !== null) && (
            <VerifyProgress
              onComplete={onVerifyComplete}
              onRepairClick={onRepair}
              showRepair={!repairing}
            />
          )}

          {/* Repair in progress message */}
          {repairing && (
            <div className="verify-phase">Repairing corrupted files…</div>
          )}
        </section>
      )}

      {/* Launch Button */}
      <section className="section">
        <button
          className="btn btn-launch"
          disabled={!isValid || launching || patching || verifying || repairing || needsUpdate}
          onClick={onLaunch}
        >
          {launching ? "Starting…" : patching ? "Patching…" : verifying ? "Verifying…" : repairing ? "Repairing…" : needsUpdate ? "Update Required" : "Start Game"}
        </button>
      </section>

      {/* Error Display */}
      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button className="error-dismiss" onClick={() => onError(null)}>
            ✕
          </button>
        </div>
      )}
    </>
  );
}
