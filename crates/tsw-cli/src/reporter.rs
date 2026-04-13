//! Terminal progress reporter using indicatif.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Mutex;
use tsw_core::download::DownloadProgress;
use tsw_core::progress::ProgressReporter;
use tsw_core::verify::VerifyProgress;

pub struct CliReporter {
    multi: MultiProgress,
    download_bar: Mutex<Option<ProgressBar>>,
    verify_bar: Mutex<Option<ProgressBar>>,
}

impl CliReporter {
    pub fn new() -> Self {
        Self {
            multi: MultiProgress::new(),
            download_bar: Mutex::new(None),
            verify_bar: Mutex::new(None),
        }
    }
}

impl Default for CliReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressReporter for CliReporter {
    fn on_download(&self, p: &DownloadProgress) {
        match p.phase.as_str() {
            "checking" | "bootstrapping" => {
                if !p.current_file.is_empty() {
                    eprintln!("[{}] {}", p.phase, p.current_file);
                }
            }
            "downloading" | "patching" | "installing" => {
                let mut slot = self.download_bar.lock().unwrap();
                let bar = slot.get_or_insert_with(|| {
                    let b = self.multi.add(ProgressBar::new(p.total_bytes.max(1)));
                    b.set_style(
                        ProgressStyle::with_template(
                            "{msg:>12.cyan.bold} [{bar:40}] {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})"
                        )
                        .unwrap()
                        .progress_chars("=> "),
                    );
                    b.set_message("Downloading");
                    b
                });
                bar.set_length(p.total_bytes.max(1));
                bar.set_position(p.bytes_downloaded);
                if p.failed_files > 0 {
                    bar.set_message(format!("Downloading ({} failed)", p.failed_files));
                }
            }
            "complete" => {
                if let Some(bar) = self.download_bar.lock().unwrap().take() {
                    bar.finish_with_message(format!(
                        "Done — {} files, {} failed",
                        p.files_completed, p.failed_files
                    ));
                }
            }
            phase if phase.starts_with("error") => {
                if let Some(bar) = self.download_bar.lock().unwrap().take() {
                    bar.abandon_with_message(format!("Error: {}", p.current_file));
                }
            }
            _ => {}
        }
    }

    fn on_verify(&self, p: &VerifyProgress) {
        match p.phase.as_str() {
            "scanning" => {
                let mut slot = self.verify_bar.lock().unwrap();
                let bar = slot.get_or_insert_with(|| {
                    let b = self.multi.add(ProgressBar::new(p.entries_total.max(1)));
                    b.set_style(
                        ProgressStyle::with_template(
                            "{msg:>12.yellow.bold} [{bar:40}] {pos}/{len} entries (eta {eta})"
                        )
                        .unwrap()
                        .progress_chars("=> "),
                    );
                    b.set_message("Verifying");
                    b
                });
                bar.set_length(p.entries_total.max(1));
                bar.set_position(p.entries_checked);
                if p.corrupted_count > 0 {
                    bar.set_message(format!("Verifying ({} corrupted)", p.corrupted_count));
                }
            }
            "complete" => {
                if let Some(bar) = self.verify_bar.lock().unwrap().take() {
                    bar.finish_with_message(format!(
                        "Verified — {} corrupted, {} OK",
                        p.corrupted_count,
                        p.entries_checked.saturating_sub(p.corrupted_count)
                    ));
                }
            }
            phase if phase.starts_with("error") => {
                if let Some(bar) = self.verify_bar.lock().unwrap().take() {
                    bar.abandon_with_message(phase.to_string());
                }
            }
            _ => {}
        }
    }
}
