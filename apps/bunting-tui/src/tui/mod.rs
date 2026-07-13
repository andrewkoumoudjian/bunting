// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

pub mod app;
mod keys;
mod nav;
mod popup;
mod render;
mod ui;
mod views;
mod widgets;

pub use app::run;
