use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::errors::{AppError, Result, EXIT_MOUNT, EXIT_SOURCE};
use crate::filesystem::PassthroughFilesystem;
use crate::filter::Filter;
use fuser::{Config, MountOption};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

pub struct MountContext {
    pub source: PathBuf,
    pub mountpoint: PathBuf,
    pub filter: Filter,
    pub read_only: bool,
    pub allow_symlinks: bool,
    pub preserve_permissions: bool,
}

pub fn mount(ctx: MountContext) -> Result<()> {
    setup_signal_handler();

    let source = std::fs::canonicalize(&ctx.source).map_err(|error| {
        AppError::io(
            EXIT_SOURCE,
            &format!("source {} is unavailable", ctx.source.display()),
            error,
        )
    })?;
    if !source.is_dir() {
        return Err(AppError::source(format!(
            "source {} is not a directory",
            source.display()
        )));
    }

    let mountpoint = ctx.mountpoint.clone();
    if !mountpoint.exists() {
        return Err(AppError::mount(format!(
            "mountpoint {} does not exist",
            mountpoint.display()
        )));
    }
    if !mountpoint.is_dir() {
        return Err(AppError::mount(format!(
            "mountpoint {} is not a directory",
            mountpoint.display()
        )));
    }
    if is_mounted(&mountpoint) {
        return Err(AppError::mount(format!(
            "mountpoint {} is already mounted",
            mountpoint.display()
        )));
    }
    let empty = std::fs::read_dir(&mountpoint)
        .map_err(|error| AppError::io(EXIT_MOUNT, "failed to read mountpoint", error))?
        .next()
        .is_none();
    if !empty {
        return Err(AppError::mount(format!(
            "mountpoint {} is not empty",
            mountpoint.display()
        )));
    }

    let filesystem = PassthroughFilesystem::new(
        &source,
        ctx.filter,
        ctx.read_only,
        ctx.allow_symlinks,
        ctx.preserve_permissions,
    )
    .map_err(|error| AppError::io(EXIT_MOUNT, "failed to initialize filesystem", error))?;

    let options = vec![
        MountOption::FSName(source.display().to_string()),
        MountOption::Subtype("scrim".to_string()),
        MountOption::DefaultPermissions,
        if ctx.read_only {
            MountOption::RO
        } else {
            MountOption::RW
        },
    ];
    let mut config = Config::default();
    config.mount_options = options;
    config.n_threads = Some(4);

    tracing::info!(
        source = %source.display(),
        mountpoint = %mountpoint.display(),
        read_only = ctx.read_only,
        "mounting Scrim filesystem"
    );
    fuser::mount(filesystem, &mountpoint, &config).map_err(|error| {
        let _ = unmount(&mountpoint);
        AppError::io(EXIT_MOUNT, "mount failed", error)
    })?;
    tracing::info!("filesystem mounted");

    wait_for_shutdown_signal();

    let _ = unmount(&mountpoint);
    Ok(())
}

fn setup_signal_handler() {
    thread::spawn(move || {
        let mut signals =
            Signals::new([SIGTERM, SIGINT]).expect("failed to install signal handlers");
        for _ in signals.forever() {
            tracing::info!(signal = "received", "shutdown signal received");
        }
    });
}

fn wait_for_shutdown_signal() {
    thread::sleep(Duration::from_secs(30));
}

pub fn unmount(mountpoint: &Path) -> Result<()> {
    if !is_mounted(mountpoint) {
        return Err(AppError::general(format!(
            "{} is not a Scrim mount point",
            mountpoint.display()
        )));
    }

    let c_path = std::ffi::CString::new(mountpoint.as_os_str().as_encoded_bytes())
        .map_err(|_| AppError::mount("mountpoint contains a NUL byte"))?;
    if unsafe { libc::umount2(c_path.as_ptr(), 0) } == 0 {
        tracing::info!(mountpoint = %mountpoint.display(), "unmounted filesystem");
        return Ok(());
    }
    let error = std::io::Error::last_os_error();

    for candidate in ["fusermount3", "fusermount"] {
        let status = std::process::Command::new(candidate)
            .arg("-u")
            .arg(mountpoint)
            .status();
        if let Ok(status) = status {
            if status.success() {
                tracing::info!(mountpoint = %mountpoint.display(), "unmounted filesystem");
                return Ok(());
            }
        }
    }

    Err(AppError::io(
        EXIT_MOUNT,
        "failed to unmount filesystem",
        error,
    ))
}

pub fn is_mounted(mountpoint: &Path) -> bool {
    let Ok(mountinfo) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let target = std::fs::canonicalize(mountpoint).unwrap_or_else(|_| mountpoint.to_path_buf());

    mountinfo.lines().any(|line| {
        line.split_whitespace()
            .nth(4)
            .map(unescape_mountinfo)
            .is_some_and(|field| Path::new(&field) == target)
    })
}

fn unescape_mountinfo(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_spaces_in_mountinfo() {
        assert_eq!(unescape_mountinfo("/tmp/a\\040b"), "/tmp/a b");
    }

    #[test]
    fn non_mountpoint_is_not_mounted() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_mounted(dir.path()));
    }
}
