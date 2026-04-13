//! Tauri adapter for `tsw_core::progress::ProgressReporter`.
//!
//! Forwards events from tsw-core to the React frontend via
//! `AppHandle::emit`. Event names and payload shapes are preserved
//! exactly — the frontend reads `event.payload.<field>` and does not
//! know tsw-core exists.

// Dead-code-allow is temporary: this adapter is wired up incrementally across
// Tasks 13–17. Task 13 creates it. Task 15 uses it for verify. Task 17 uses it
// for the install pipeline. Once all emit sites are migrated, the allow goes.
#![allow(dead_code)]

use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tsw_core::download::DownloadProgress;
use tsw_core::progress::ProgressReporter;
use tsw_core::verify::VerifyProgress;

/// Wraps a Tauri `AppHandle` and emits progress events to the frontend.
pub(crate) struct TauriReporter {
    app: AppHandle,
}

impl TauriReporter {
    /// Construct a shared reporter ready to hand to tsw-core functions.
    pub(crate) fn new(app: AppHandle) -> Arc<dyn ProgressReporter> {
        Arc::new(Self { app })
    }
}

impl ProgressReporter for TauriReporter {
    fn on_download(&self, progress: &DownloadProgress) {
        // Event name `patch:progress` matches the existing frontend listener
        // in src/App.tsx:56 and src/PatchProgress.tsx:57. Payload is the
        // DownloadProgress struct, which serializes to the exact JSON shape
        // the frontend reads.
        let _ = self.app.emit("patch:progress", progress);
    }

    fn on_verify(&self, progress: &VerifyProgress) {
        // Event name `verify:progress` matches the existing frontend listener
        // in src/App.tsx:85 and src/VerifyProgress.tsx:49.
        let _ = self.app.emit("verify:progress", progress);
    }
}
