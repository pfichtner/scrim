use scrim::filter::Filter;
use std::path::Path;

#[test]
fn excluded_secret_not_matched_as_visible() {
    let filter = Filter::new(
        vec![
            ".env".to_string(),
            ".env.*".to_string(),
            "*.key".to_string(),
        ],
        true,
    )
    .unwrap();
    assert!(filter.is_excluded(Path::new(".env")));
    assert!(filter.is_excluded(Path::new(".env.production")));
    assert!(filter.is_excluded(Path::new("private.key")));
}

#[test]
fn directory_traversal_name_is_not_a_normal_path_match() {
    // Filter matches relative paths; traversal rejection is enforced in filesystem
    // path handling (singleton component + canonicalization), not by the matcher.
    let filter = Filter::new(vec![".env".to_string()], true).unwrap();
    assert!(!filter.is_excluded(Path::new("..")));
}

#[test]
fn nested_secret_paths_are_excluded() {
    let filter = Filter::new(vec!["**/secrets/**".to_string()], true).unwrap();
    assert!(filter.is_excluded(Path::new("app/secrets/token")));
    assert!(filter.is_parent_excluded(Path::new("app/secrets/deep/token")));
}
