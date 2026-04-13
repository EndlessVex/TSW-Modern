//! Cross-platform system info helpers.
//!
//! Currently exposes only memory detection, used by the install pipeline
//! to tune download concurrency based on available RAM.

use sysinfo::System;

/// Return the currently-available (free) system memory in megabytes.
///
/// Matches the pre-refactor Windows behavior, which called
/// `GlobalMemoryStatusEx` and returned `ullAvailPhys / 1_048_576` —
/// i.e. free physical RAM, not total physical RAM. The concurrency
/// tuner in the install pipeline uses this to decide how many large
/// resources may sit in memory simultaneously, so free memory is the
/// right signal.
///
/// Falls back to 4096 (assumes 4 GB free) if the platform doesn't
/// report memory info. Same fallback the pre-refactor code used when
/// `GlobalMemoryStatusEx` returned zero.
pub fn available_ram_mb() -> u64 {
    let mut sys = System::new();
    sys.refresh_memory();
    let available_bytes = sys.available_memory();
    if available_bytes == 0 {
        4096
    } else {
        available_bytes / (1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_ram_is_nonzero() {
        let mb = available_ram_mb();
        // Any real machine has at least 1 GB.
        assert!(mb >= 1024, "available_ram_mb returned suspiciously low value: {}", mb);
    }
}
