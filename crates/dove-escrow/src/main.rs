//! Dove's escrow image.
//!
//! On Windows every agent Dove starts goes through a job object and an inert
//! parent process that holds the agent's inherited handles (see
//! `vendor/zeron-harness/src/process/windows/escrow.rs`). That parent is
//! created suspended and never runs a single instruction of this program;
//! only its image is mapped. Using this tiny binary instead of `dove.exe`
//! keeps each spawn's helper to about a megabyte instead of the full app's
//! mapped image. If it is missing, the harness falls back to `dove.exe`.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {}
