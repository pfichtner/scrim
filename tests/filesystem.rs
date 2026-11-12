use scrim::filter::Filter;
use scrim::mount::{self, MountContext};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

struct Mounted {
    _source: tempfile::TempDir,
    _mountpoint: tempfile::TempDir,
    mountpoint: PathBuf,
    source: PathBuf,
    handle: Option<thread::JoinHandle<scrim::errors::Result<()>>>,
}

impl Mounted {
    fn new(patterns: Vec<&str>, allow_symlinks: bool) -> Self {
        let source = tempfile::tempdir().unwrap();
        let mountpoint = tempfile::tempdir().unwrap();
        let filter = Filter::new(patterns.into_iter().map(str::to_string).collect(), true).unwrap();
        let ctx = MountContext {
            source: source.path().to_path_buf(),
            mountpoint: mountpoint.path().to_path_buf(),
            filter,
            read_only: false,
            allow_symlinks,
            preserve_permissions: true,
        };
        let mp = mountpoint.path().to_path_buf();
        let handle = thread::spawn(move || mount::mount(ctx));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if mount::is_mounted(&mp) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(mount::is_mounted(&mp), "mount did not become ready");
        Self {
            source: source.path().to_path_buf(),
            mountpoint: mp,
            _source: source,
            _mountpoint: mountpoint,
            handle: Some(handle),
        }
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.mountpoint.join(rel)
    }
    fn src(&self, rel: &str) -> PathBuf {
        self.source.join(rel)
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        if mount::is_mounted(&self.mountpoint) {
            let _ = mount::unmount(&self.mountpoint);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn require_mount() {
    if !Path::new("/dev/fuse").exists() || unsafe { libc::geteuid() } != 0 {
        panic!("requires /dev/fuse and root");
    }
}

#[test]
#[ignore = "requires /dev/fuse and root"]
fn list_visible_files_omits_excluded() {
    require_mount();
    let m = Mounted::new(vec![".env", ".env.*", ".git/"], false);
    fs::write(m.src("app.py"), "ok").unwrap();
    fs::write(m.src(".env"), "SECRET=1").unwrap();
    fs::write(m.src(".env.local"), "SECRET=2").unwrap();
    fs::create_dir(m.src(".git")).unwrap();
    fs::write(m.src(".git/config"), "x").unwrap();

    let mut names: Vec<_> = fs::read_dir(&m.mountpoint)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert!(names.contains(&"app.py".to_string()));
    assert!(!names.iter().any(|n| n.starts_with(".env")));
    assert!(!names.contains(&".git".to_string()));
}

#[test]
#[ignore = "requires /dev/fuse and root"]
fn read_write_visible_and_write_through() {
    require_mount();
    let m = Mounted::new(vec![".env"], false);
    fs::write(m.src("app.py"), "one").unwrap();

    assert_eq!(fs::read_to_string(m.p("app.py")).unwrap(), "one");
    fs::write(m.p("app.py"), "two").unwrap();
    assert_eq!(fs::read_to_string(m.src("app.py")).unwrap(), "two");
    fs::write(m.p("new.txt"), "created").unwrap();
    assert_eq!(fs::read_to_string(m.src("new.txt")).unwrap(), "created");
    fs::remove_file(m.p("new.txt")).unwrap();
    assert!(!m.src("new.txt").exists());
}

#[test]
#[ignore = "requires /dev/fuse and root"]
fn excluded_files_return_enoent() {
    require_mount();
    let m = Mounted::new(vec![".env", ".env.*", "*.key", "**/secrets/**"], false);
    fs::write(m.src(".env"), "SECRET").unwrap();
    fs::write(m.src(".env.production"), "SECRET").unwrap();
    fs::write(m.src("private.key"), "SECRET").unwrap();
    fs::create_dir_all(m.src("config/secrets")).unwrap();
    fs::write(m.src("config/secrets/cred.json"), "SECRET").unwrap();

    for path in [
        ".env",
        ".env.production",
        "private.key",
        "config/secrets/cred.json",
    ] {
        let err = fs::read(m.p(path)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "read {path}");
        let err = fs::metadata(m.p(path)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "stat {path}");
    }
    let err = fs::read_dir(m.p("config/secrets")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[test]
#[ignore = "requires /dev/fuse and root"]
fn rename_to_excluded_name_rejected() {
    require_mount();
    let m = Mounted::new(vec![".env"], false);
    fs::write(m.src("visible.txt"), "ok").unwrap();
    let err = fs::rename(m.p("visible.txt"), m.p(".env")).unwrap_err();
    assert!(
        err.kind() == io::ErrorKind::PermissionDenied || err.raw_os_error() == Some(libc::EACCES)
    );
    assert_eq!(fs::read_to_string(m.src("visible.txt")).unwrap(), "ok");
    assert!(!m.src(".env").exists());
}

#[test]
#[ignore = "requires /dev/fuse and root"]
fn mkdir_rmdir_visible() {
    require_mount();
    let m = Mounted::new(vec![".env"], false);
    fs::create_dir(m.p("subdir")).unwrap();
    assert!(m.src("subdir").is_dir());
    fs::remove_dir(m.p("subdir")).unwrap();
    assert!(!m.src("subdir").exists());
}
