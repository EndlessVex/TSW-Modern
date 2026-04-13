//! Progress reporting abstraction.
//!
//! `ProgressReporter` is the sink for progress events emitted by tsw-core
//! operations (download, verify, client_files). Implementors decide how to
//! surface them:
//!
//! - The Windows Tauri launcher implements it in `src-tauri/src/tauri_reporter.rs`,
//!   wrapping `AppHandle::emit` to forward events to the React frontend.
//! - The Linux CLI implements it in `tsw-cli/src/reporter.rs`, drawing
//!   indicatif progress bars.
//! - Tests use `NullReporter` or a custom capturing implementation.
//!
//! The trait has `Send + Sync` bounds because download tasks are spawned
//! across multiple tokio workers and share the reporter via `Arc`.

use crate::download::DownloadProgress;
use crate::verify::VerifyProgress;

/// Sink for progress events emitted by tsw-core operations.
pub trait ProgressReporter: Send + Sync {
    /// Called for every download/client_files progress event.
    ///
    /// Default implementation does nothing. Implementors override only
    /// the methods they care about.
    fn on_download(&self, _progress: &DownloadProgress) {}

    /// Called for every verify progress event.
    fn on_verify(&self, _progress: &VerifyProgress) {}
}

/// A reporter that discards all events. Useful for tests and headless
/// library calls that don't need progress output.
pub struct NullReporter;

impl ProgressReporter for NullReporter {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::DownloadProgress;

    #[test]
    fn null_reporter_accepts_download_events() {
        let reporter = NullReporter;
        let progress = DownloadProgress {
            bytes_downloaded: 100,
            total_bytes: 200,
            files_completed: 1,
            files_total: 5,
            speed_bps: 1_000_000,
            current_file: "test".into(),
            phase: "downloading".into(),
            failed_files: 0,
        };
        reporter.on_download(&progress);
        // No panic means success.
    }

    #[test]
    fn null_reporter_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NullReporter>();
    }
}
