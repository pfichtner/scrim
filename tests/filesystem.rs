// Integration tests for FUSE filesystem behavior.
// Full mount tests require /dev/fuse and root; they are kept as ignored
// placeholders to be exercised on a native Linux host or via integration-test.sh.

#[test]
#[ignore = "requires /dev/fuse and a mounted Scrim filesystem"]
fn list_visible_files_omits_excluded() {}

#[test]
#[ignore = "requires /dev/fuse and a mounted Scrim filesystem"]
fn write_through_visible_files() {}

#[test]
#[ignore = "requires /dev/fuse and a mounted Scrim filesystem"]
fn excluded_files_return_enoent() {}
