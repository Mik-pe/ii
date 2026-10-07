//! Resolve user-supplied locations before constructing navigation entries.
//! Call this on a filesystem worker, not in the input/render loop.

use std::io;
use std::path::{Component, Path, PathBuf};

/// Absolute paths without dot components keep their spelling and symlink aliases.
/// On POSIX, unresolved `..` needs realpath: blindly popping components would
/// give the wrong location for `symlink/..`. Windows absolute() uses native rules.
pub fn resolve(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    if absolute.components().any(|part| part == Component::ParentDir) {
        std::fs::canonicalize(absolute)
    } else {
        Ok(absolute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolves_parent_and_dot_components_before_listing_children() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("app/src")).unwrap();
        let resolved = resolve(&root.path().join("app/src/.././..")).unwrap();
        assert_eq!(fs::canonicalize(&resolved).unwrap(), fs::canonicalize(root.path()).unwrap());
        assert!(resolved.is_absolute());
        assert!(!resolved.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)));
        assert_eq!(resolve(Path::new(".")).unwrap(), std::env::current_dir().unwrap());
    }

    #[test]
    fn does_not_rewrite_already_absolute_paths() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(resolve(root.path()).unwrap(), root.path());
        assert!(resolve(Path::new("")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn resolves_symlink_parent_physically_but_preserves_ordinary_aliases() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("real/child")).unwrap();
        symlink(root.path().join("real/child"), root.path().join("alias")).unwrap();
        assert_eq!(resolve(&root.path().join("alias")).unwrap(), root.path().join("alias"));
        assert_eq!(resolve(&root.path().join("alias/..")).unwrap(), fs::canonicalize(root.path().join("real")).unwrap());
        assert!(resolve(&root.path().join("missing/..")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_relative_inputs_do_not_gain_a_verbatim_prefix() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("child")).unwrap();
        let result = resolve(&root.path().join("child/..")).unwrap();
        assert_eq!(result, std::path::absolute(root.path()).unwrap());
    }
}
