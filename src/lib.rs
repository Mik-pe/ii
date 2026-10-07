//! Terminal-independent navigation, directory loading, and presentation.
//! Paths stay as `PathBuf`; escaped labels are exclusively for display.

pub mod deep;
pub mod filesystem;
pub mod filter;
pub mod model;
pub mod paths;
pub mod ui;
