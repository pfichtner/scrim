use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Filter {
    globset: GlobSet,
}

impl Filter {
    pub fn new(patterns: Vec<String>, case_sensitive: bool) -> anyhow::Result<Self> {
        let mut expanded = Vec::new();

        for pattern in patterns {
            let raw = pattern.trim();
            if raw.is_empty() {
                continue;
            }

            let (pattern, is_dir) = match raw.strip_suffix('/') {
                Some(dir) => (dir, true),
                None => (raw, false),
            };
            if pattern.is_empty() {
                continue;
            }

            if pattern.contains('/') {
                expanded.push(pattern.to_string());
                if is_dir || pattern.ends_with("/**") {
                    expanded.push(format!("{pattern}/**"));
                }
                if pattern.ends_with("/**") {
                    expanded.push(pattern.trim_end_matches("/**").to_string());
                }
                // Also match the pattern relative to the source root.
                if let Some(rest) = pattern.strip_prefix("**/") {
                    expanded.push(rest.to_string());
                    if rest.ends_with("/**") {
                        expanded.push(rest.trim_end_matches("/**").to_string());
                    }
                }
            } else {
                // A bare name matches at the root and at any depth.
                expanded.push(pattern.to_string());
                expanded.push(format!("**/{pattern}"));
                if is_dir {
                    expanded.push(format!("{pattern}/**"));
                    expanded.push(format!("**/{pattern}/**"));
                }
            }
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in expanded {
            let glob = GlobBuilder::new(&pattern)
                .case_insensitive(!case_sensitive)
                .build()?;
            builder.add(glob);
        }

        Ok(Self {
            globset: builder.build()?,
        })
    }

    pub fn is_excluded(&self, path: &Path) -> bool {
        self.globset.is_match(path)
    }

    pub fn is_parent_excluded(&self, path: &Path) -> bool {
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
        assert!(filter.is_excluded(Path::new(".git")));
        assert!(filter.is_excluded(Path::new(".git/config")));
        assert!(filter.is_excluded(Path::new(".git/objects/file")));
        assert!(!filter.is_excluded(Path::new("app.py")));
    }

    #[test]
    fn test_recursive_exclusion() {
        let filter = Filter::new(vec!["**/secrets/**".to_string()], true).unwrap();
        assert!(filter.is_excluded(Path::new("config/secrets/credentials.json")));
        assert!(filter.is_excluded(Path::new("app/secrets/key.pem")));
        assert!(filter.is_excluded(Path::new("secrets/key.pem")));
        assert!(filter.is_excluded(Path::new("config/secrets")));
        assert!(!filter.is_excluded(Path::new("config/database.json")));
    }

    #[test]
    fn test_bare_pattern_matches_at_any_depth() {
        let filter = Filter::new(vec![".env".to_string()], true).unwrap();
        assert!(filter.is_excluded(Path::new(".env")));
        assert!(filter.is_excluded(Path::new("config/.env")));
        assert!(filter.is_excluded(Path::new("a/b/.env")));
    }

    #[test]
    fn test_nested_directory_pattern() {
        let filter = Filter::new(vec![".git/".to_string()], true).unwrap();
        assert!(filter.is_excluded(Path::new("vendor/repo/.git/config")));
        assert!(filter.is_parent_excluded(Path::new("vendor/repo/.git/objects/x")));
    }

    #[test]
    fn test_case_insensitive() {
        let filter = Filter::new(vec![".env".to_string()], false).unwrap();
        assert!(filter.is_excluded(Path::new(".ENV")));
        assert!(filter.is_excluded(Path::new("config/.Env")));
    }

    #[test]
    fn test_empty_patterns_hide_nothing() {
        let filter = Filter::new(Vec::new(), true).unwrap();
        assert!(!filter.is_excluded(Path::new(".env")));
    }
}
