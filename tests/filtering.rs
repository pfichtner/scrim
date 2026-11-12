use scrim::filter::Filter;
use std::path::Path;

#[test]
fn basic_exclusion() {
    let filter = Filter::new(vec![".env".to_string()], true).unwrap();
    assert!(filter.is_excluded(Path::new(".env")));
    assert!(!filter.is_excluded(Path::new("app.py")));
}

#[test]
fn wildcard_exclusion_env_star() {
    let filter = Filter::new(vec![".env.*".to_string()], true).unwrap();
    assert!(filter.is_excluded(Path::new(".env.local")));
    assert!(filter.is_excluded(Path::new(".env.production")));
    assert!(!filter.is_excluded(Path::new(".env")));
}

#[test]
fn wildcard_extension_patterns() {
    let filter = Filter::new(vec!["*.pem".to_string(), "*.key".to_string()], true).unwrap();
    assert!(filter.is_excluded(Path::new("cert.pem")));
    assert!(filter.is_excluded(Path::new("private.key")));
    assert!(filter.is_excluded(Path::new("nested/tls/server.pem")));
    assert!(!filter.is_excluded(Path::new("key.txt")));
}

#[test]
fn directory_exclusion_hides_descendants() {
    let filter = Filter::new(vec![".git/".to_string()], true).unwrap();
    assert!(filter.is_excluded(Path::new(".git")));
    assert!(filter.is_excluded(Path::new(".git/config")));
    assert!(filter.is_excluded(Path::new(".git/objects/pack/file")));
    assert!(filter.is_parent_excluded(Path::new(".git/objects/pack/file")));
    assert!(!filter.is_excluded(Path::new("app.py")));
}

#[test]
fn nested_directory_exclusion() {
    let filter = Filter::new(vec![".git/".to_string()], true).unwrap();
    assert!(filter.is_excluded(Path::new("vendor/repo/.git/config")));
    assert!(filter.is_parent_excluded(Path::new("vendor/repo/.git/HEAD")));
}

#[test]
fn recursive_secrets_pattern() {
    let filter = Filter::new(vec!["**/secrets/**".to_string()], true).unwrap();
    assert!(filter.is_excluded(Path::new("config/secrets/credentials.json")));
    assert!(filter.is_excluded(Path::new("app/secrets/key.pem")));
    assert!(filter.is_excluded(Path::new("secrets/key.pem")));
    assert!(!filter.is_excluded(Path::new("config/database.json")));
}

#[test]
fn bare_name_matches_any_depth() {
    let filter = Filter::new(vec![".env".to_string()], true).unwrap();
    assert!(filter.is_excluded(Path::new(".env")));
    assert!(filter.is_excluded(Path::new("config/.env")));
    assert!(filter.is_excluded(Path::new("a/b/.env")));
}

#[test]
fn multiple_patterns() {
    let filter = Filter::new(
        vec![
            ".env".to_string(),
            ".env.*".to_string(),
            "*.key".to_string(),
            ".git/".to_string(),
        ],
        true,
    )
    .unwrap();
    assert!(filter.is_excluded(Path::new(".env")));
    assert!(filter.is_excluded(Path::new(".env.production")));
    assert!(filter.is_excluded(Path::new("private.key")));
    assert!(filter.is_excluded(Path::new(".git/HEAD")));
    assert!(!filter.is_excluded(Path::new("index.js")));
}

#[test]
fn case_insensitive_matching() {
    let filter = Filter::new(vec![".env".to_string(), "*.PEM".to_string()], false).unwrap();
    assert!(filter.is_excluded(Path::new(".ENV")));
    assert!(filter.is_excluded(Path::new("config/.Env")));
    assert!(filter.is_excluded(Path::new("server.pem")));
    assert!(filter.is_excluded(Path::new("SERVER.PEM")));
}

#[test]
fn case_sensitive_by_default() {
    let filter = Filter::new(vec![".env".to_string()], true).unwrap();
    assert!(!filter.is_excluded(Path::new(".ENV")));
}

#[test]
fn visible_paths_not_excluded() {
    let filter = Filter::new(vec![".env".to_string(), "**/secrets/**".to_string()], true).unwrap();
    for path in ["app.py", "config/database.json", "README.md", "src/main.rs"] {
        assert!(
            !filter.is_excluded(Path::new(path)),
            "{path} should be visible"
        );
        assert!(
            !filter.is_parent_excluded(Path::new(path)),
            "{path} parent chain should be visible"
        );
    }
}
