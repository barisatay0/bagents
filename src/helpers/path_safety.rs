use std::path::{Path, PathBuf, Component};

/// Joins a base path and a relative path, returning None if the relative path attempts to escape the base path.
pub fn safe_join(base: &Path, relative: &str) -> Option<PathBuf> {
    let rel_path = Path::new(relative);
    if rel_path.is_absolute() {
        return None;
    }

    let mut components = Vec::new();
    for component in rel_path.components() {
        match component {
            Component::Prefix(_) => return None,
            Component::RootDir => return None,
            Component::CurDir => {}
            Component::ParentDir => {
                if components.pop().is_none() {
                    // Escalation above the root folder
                    return None;
                }
            }
            Component::Normal(c) => {
                components.push(c);
            }
        }
    }

    let mut joined = base.to_path_buf();
    for c in components {
        joined.push(c);
    }

    // If the canonicalized version of base and joined both exist, double check that joined starts with base.
    if let Ok(canon_joined) = joined.canonicalize() {
        if let Ok(canon_base) = base.canonicalize() {
            if !canon_joined.starts_with(&canon_base) {
                return None;
            }
        }
    }

    Some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_join() {
        let base = Path::new("/tmp/workspace");
        
        // Safe relative path
        assert_eq!(safe_join(base, "src/main.rs"), Some(PathBuf::from("/tmp/workspace/src/main.rs")));
        assert_eq!(safe_join(base, "foo/bar/../baz"), Some(PathBuf::from("/tmp/workspace/foo/baz")));

        // Traversal attempt (absolute path)
        assert_eq!(safe_join(base, "/etc/passwd"), None);

        // Traversal attempt (relative escaping base)
        assert_eq!(safe_join(base, "../passwd"), None);
        assert_eq!(safe_join(base, "foo/../../etc/passwd"), None);
    }
}
