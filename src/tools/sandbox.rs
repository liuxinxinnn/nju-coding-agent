use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

const SENSITIVE_NAMES: &[&str] = &[
    ".ssh",
    ".gnupg",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
];

#[derive(Debug, Clone)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    pub fn new(root: PathBuf) -> Result<Self> {
        let root = root
            .canonicalize()
            .map_err(|error| Error::Tool(format!("cannot resolve workspace: {error}")))?;
        if !root.is_dir() {
            return Err(Error::Tool(format!(
                "workspace is not a directory: {}",
                root.display()
            )));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve_existing(&self, raw: &str) -> Result<PathBuf> {
        let candidate = self.input_path(raw);
        let resolved = candidate
            .canonicalize()
            .map_err(|error| Error::Tool(format!("cannot resolve '{raw}': {error}")))?;
        self.ensure_allowed(&resolved)?;
        Ok(resolved)
    }

    pub fn resolve_writable(&self, raw: &str) -> Result<PathBuf> {
        if raw.trim().is_empty() {
            return Err(Error::Tool("path cannot be empty".to_owned()));
        }

        let target = lexical_normalize(&self.input_path(raw));
        self.ensure_inside(&target)?;
        self.ensure_not_sensitive(&target)?;

        if target.exists() {
            let resolved = target.canonicalize().map_err(|error| {
                Error::Tool(format!("cannot resolve write target '{raw}': {error}"))
            })?;
            self.ensure_allowed(&resolved)?;
            return Ok(resolved);
        }

        let parent = target
            .parent()
            .ok_or_else(|| Error::Tool("write target has no parent".to_owned()))?;
        let ancestor = existing_ancestor(parent)
            .ok_or_else(|| Error::Tool("cannot find existing parent directory".to_owned()))?;
        let resolved_ancestor = ancestor
            .canonicalize()
            .map_err(|error| Error::Tool(format!("cannot resolve parent directory: {error}")))?;
        self.ensure_allowed(&resolved_ancestor)?;
        Ok(target)
    }

    pub fn display_relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    pub fn is_sensitive_path(&self, path: &Path) -> bool {
        path.components().any(|component| {
            let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
            value == ".env"
                || value.starts_with(".env.")
                || SENSITIVE_NAMES.iter().any(|name| value == *name)
        })
    }

    fn input_path(&self, raw: &str) -> PathBuf {
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    fn ensure_allowed(&self, path: &Path) -> Result<()> {
        self.ensure_inside(path)?;
        self.ensure_not_sensitive(path)
    }

    fn ensure_inside(&self, path: &Path) -> Result<()> {
        if path.starts_with(&self.root) {
            Ok(())
        } else {
            Err(Error::Tool(format!(
                "path escapes workspace sandbox: {}",
                path.display()
            )))
        }
    }

    fn ensure_not_sensitive(&self, path: &Path) -> Result<()> {
        if self.is_sensitive_path(path) {
            Err(Error::Tool(format!(
                "access to sensitive path is blocked: {}",
                path.display()
            )))
        } else {
            Ok(())
        }
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                result.push(component.as_os_str());
            }
        }
    }
    result
}

fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::Sandbox;

    #[test]
    fn rejects_parent_traversal() {
        let parent = tempdir().expect("temp dir");
        let root = parent.path().join("workspace");
        fs::create_dir(&root).expect("workspace");
        let sandbox = Sandbox::new(root).expect("sandbox");

        assert!(sandbox.resolve_writable("../outside.txt").is_err());
    }

    #[test]
    fn rejects_sensitive_files() {
        let root = tempdir().expect("temp dir");
        let sandbox = Sandbox::new(root.path().to_path_buf()).expect("sandbox");

        assert!(sandbox.resolve_writable(".env").is_err());
        assert!(sandbox.resolve_writable("config/.env.local").is_err());
    }
}
