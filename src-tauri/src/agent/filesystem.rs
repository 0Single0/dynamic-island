use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathPolicyError {
    OutsideAuthorizedRoot,
    UncPathNotAllowed,
    SymlinkNotAllowed,
    MissingParent,
    Io(String),
}

#[derive(Debug, Clone)]
pub struct PathPolicy {
    authorized_roots: Vec<PathBuf>,
    allow_unc: bool,
}

impl PathPolicy {
    pub fn new(authorized_roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            authorized_roots: authorized_roots
                .into_iter()
                .filter_map(|path| fs::canonicalize(path).ok())
                .collect(),
            allow_unc: false,
        }
    }

    pub fn resolve_existing(&self, requested: &Path) -> Result<PathBuf, PathPolicyError> {
        self.reject_unc(requested)?;
        let resolved =
            fs::canonicalize(requested).map_err(|error| PathPolicyError::Io(error.to_string()))?;
        self.reject_symlink_chain(requested)?;
        self.require_authorized(&resolved)
    }

    pub fn resolve_for_write(&self, requested: &Path) -> Result<PathBuf, PathPolicyError> {
        self.reject_unc(requested)?;
        let parent = requested.parent().ok_or(PathPolicyError::MissingParent)?;
        let canonical_parent =
            fs::canonicalize(parent).map_err(|error| PathPolicyError::Io(error.to_string()))?;
        self.reject_symlink_chain(parent)?;
        let filename = requested
            .file_name()
            .ok_or(PathPolicyError::MissingParent)?;
        let resolved = canonical_parent.join(filename);
        self.require_authorized(&canonical_parent).map(|_| resolved)
    }

    fn require_authorized(&self, path: &Path) -> Result<PathBuf, PathPolicyError> {
        if self
            .authorized_roots
            .iter()
            .any(|root| path == root || path.starts_with(root))
        {
            Ok(path.to_path_buf())
        } else {
            Err(PathPolicyError::OutsideAuthorizedRoot)
        }
    }

    fn reject_unc(&self, path: &Path) -> Result<(), PathPolicyError> {
        let raw = path.to_string_lossy();
        if !self.allow_unc && (raw.starts_with("\\\\") || raw.starts_with("//")) {
            Err(PathPolicyError::UncPathNotAllowed)
        } else {
            Ok(())
        }
    }

    fn reject_symlink_chain(&self, path: &Path) -> Result<(), PathPolicyError> {
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component);
            if fs::symlink_metadata(&current)
                .map_err(|error| PathPolicyError::Io(error.to_string()))?
                .file_type()
                .is_symlink()
            {
                return Err(PathPolicyError::SymlinkNotAllowed);
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
fn _io_error(error: io::Error) -> PathPolicyError {
    PathPolicyError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn selected_root_allows_read_and_new_file_but_not_sibling() {
        let root = tempfile_dir("path-policy");
        let file = root.join("note.md");
        fs::write(&file, "hello").unwrap();
        let policy = PathPolicy::new([root.clone()]);
        let canonical_root = fs::canonicalize(&root).unwrap();
        assert_eq!(
            policy.resolve_existing(&file).unwrap(),
            fs::canonicalize(&file).unwrap()
        );
        assert_eq!(
            policy.resolve_for_write(&root.join("out.pdf")).unwrap(),
            canonical_root.join("out.pdf")
        );
        let sibling = root
            .parent()
            .unwrap()
            .join(format!("dynamic-island-outside-{}.txt", std::process::id()));
        fs::write(&sibling, "outside").unwrap();
        assert_eq!(
            policy.resolve_existing(&sibling),
            Err(PathPolicyError::OutsideAuthorizedRoot)
        );
        let _ = fs::remove_file(sibling);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unc_paths_are_rejected_by_default() {
        let policy = PathPolicy::new([]);
        assert_eq!(
            policy.resolve_existing(Path::new("\\\\server\\share\\file.txt")),
            Err(PathPolicyError::UncPathNotAllowed)
        );
    }

    fn tempfile_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("dynamic-island-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
