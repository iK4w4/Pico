//! Resolving a local path into a servable (folder, name) pair —
//! the shared core of EVERY file-open entry point: recent files,
//! the file picker, drag-and-drop, argv, and address-bar paths
//! all land here through the app's `open_path` (AUDIT V17-2).
//!
//! This logic lived in the GUI layer until v17, where no host
//! test could reach it — which is how the v13-era defect survived
//! four releases: the existence check joined the file name onto
//! the file's own canonical path, producing `<file>/<name>`,
//! which is never a file, so every *file* open failed silently.
//! The check below joins onto the *folder*, and
//! `tests/action_tests.rs::resolve_file_points_at_the_file_itself`
//! pins the file case against real files on disk.

use std::path::{Path, PathBuf};

use crate::core::addr::is_safe_local_path;

/// A path resolved to the folder the static server should serve
/// and the name of the file inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOpen {
    /// Folder to register as a server root.
    pub folder: PathBuf,
    /// File name within `folder`.
    pub rel: String,
    /// `folder.join(rel)` — the file that will actually be served.
    pub file: PathBuf,
}

/// Why a local path could not be resolved for opening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenResolveError {
    /// UNC / device-namespace path, refused lexically (before any
    /// filesystem call could trigger network authentication).
    UnsafePath,
    /// `canonicalize` failed (missing file, permissions, …).
    CannotResolve(String),
    /// The path has no parent folder to serve from.
    NoParent,
    /// The resolved target (e.g. a directory's index.html) is not
    /// a file on disk.
    MissingFile(PathBuf),
}

/// Resolve `path` (a file or a directory) into its servable parts.
///
/// Order is a security property: the lexical UNC/device gate runs
/// BEFORE `canonicalize` (mere resolution of a UNC path can make
/// Windows authenticate to a hostile server), and the canonical
/// result is re-gated before it is used.
pub fn resolve_open_target(path: &Path) -> Result<ResolvedOpen, OpenResolveError> {
    if !is_safe_local_path(path) {
        return Err(OpenResolveError::UnsafePath);
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|e| OpenResolveError::CannotResolve(e.to_string()))?;
    if !is_safe_local_path(&canonical) {
        return Err(OpenResolveError::UnsafePath);
    }
    let (folder, rel) = if canonical.is_dir() {
        (canonical.clone(), "index.html".to_string())
    } else if let Some(parent) = canonical.parent() {
        let name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        (parent.to_path_buf(), name)
    } else {
        return Err(OpenResolveError::NoParent);
    };
    // AUDIT V17-2: the target is FOLDER.join(rel). Joining onto
    // `canonical` instead (the v13–v16 code) appends the name to
    // the file itself for the file case and fails every file open.
    let file = folder.join(&rel);
    if !file.is_file() {
        return Err(OpenResolveError::MissingFile(file));
    }
    Ok(ResolvedOpen { folder, rel, file })
}
