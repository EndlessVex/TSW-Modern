//! tsw-core — shared download, verify, and RDB parsing library.
//!
//! This crate is consumed by both the Windows Tauri launcher (`src-tauri`)
//! and the Linux CLI (`tsw-cli`). It contains no UI code and no Tauri
//! dependency.

pub mod bxml;
pub mod client_files;
pub mod config;
pub mod download;
pub mod encoder_native;
pub mod progress;
pub mod rdb;
pub mod rdbdata;
pub mod redux;
pub mod verify;
