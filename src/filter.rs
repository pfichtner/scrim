use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Filter {
    globset: GlobSet,
    case_sensitive: bool,
}

impl Filter {
    pub fn new(patterns: Vec<String>, case_sensitive: bool) -> anyhow::Result<Self> {
        let mut builder = GlobSetBuilder::new();

        for pattern in patterns {
            if let Some(dir) = pattern.strip_suffix('/') {
                // Directory pattern: hide the directory itself and everything below it.
                if !dir.is_empty() {
                    builder.add(Glob::new(dir)?);
                    builder.add(Glob::new(&format!("{dir}/**"))?);
                }
            } else {
                builder.add(Glob::new(&pattern)?);
            }
        }

        let globset = builder.build()?;
        Ok(Self {
            globset,
            case_sensitive,
        })
    }

    pub fn is_excluded(&self, path: &Path) -> bool {
        if self.case_sensitive {
            self.globset.is_match(path)
        } else {
            // Convert to lowercase path for case-insensitive matching
            let path_str = path.to_string_lossy().to_lowercase();
            self.globset.is_match(&path_str)
        }
    }

    pub fn is_parent_excluded(&self, path: &Path) -> bool {
        // Check if any parent directory is excluded
        let mut current = Some(path);
        while let Some(p) = current {
            if self.is_excluded(p) {
                return true;
            }
            current = p.parent();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_basic_exclusion() {
        let filter = Filter::new(vec![".env".to_string()], true).unwrap();
        assert!(filter.is_excluded(Path::new(".env")));
        assert!(!filter.is_excluded(Path::new("app.py")));
    }

    #[test]
    fn test_wildcard_exclusion() {
        let filter = Filter::new(vec![".env.*".to_string()], true).unwrap();
        assert!(filter.is_excluded(Path::new(".env.local")));
        assert!(filter.is_excluded(Path::new(".env.production")));
        assert!(!filter.is_excluded(Path::new(".env")));
    }

    #[test]
    fn test_directory_exclusion() {
        let filter = Filter::new(vec![".git/".to_string()], true).unwrap();
        assert!(filter.is_excluded(Path::new(".git/config")));
        assert!(filter.is_excluded(Path::new(".git/objects/file")));
        assert!(!filter.is_excluded(Path::new("app.py")));
    }

    #[test]
    fn test_recursive_exclusion() {
        let filter = Filter::new(vec!["**/secrets/**".to_string()], true).unwrap();
        assert!(filter.is_excluded(Path::new("config/secrets/credentials.json")));
        assert!(filter.is_excluded(Path::new("app/secrets/key.pem")));
        assert!(!filter.is_excluded(Path::new("config/database.json")));
    }
}
