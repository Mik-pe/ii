//! Terminal-independent navigation, directory loading, and presentation.
//! Paths stay as `PathBuf`; escaped labels are exclusively for display.

pub mod filter;
pub mod filesystem;
pub mod model;
pub mod ui;
