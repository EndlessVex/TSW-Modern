//! Sentinel error types used by the CLI to signal distinct exit codes
//! from anywhere in the call graph. These are zero-size markers that
//! `main()` downcasts to decide the process exit code.

use std::fmt;

#[derive(Debug)]
pub struct UserCancelled;

impl fmt::Display for UserCancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cancelled by user")
    }
}

impl std::error::Error for UserCancelled {}

#[derive(Debug)]
pub struct VerifyFoundCorrupted {
    pub count: u64,
}

impl fmt::Display for VerifyFoundCorrupted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} corrupted files found", self.count)
    }
}

impl std::error::Error for VerifyFoundCorrupted {}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config error: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}
