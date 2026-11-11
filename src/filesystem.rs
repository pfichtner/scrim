use std::collections::HashMap;
use std::ffi::{CString, OsStr, OsString};
use std::fs::{File, Metadata};
use std::io;
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    AccessFlags, BsdFileFlags, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, KernelConfig, LockOwner, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request,
    TimeOrNow, WriteFlags,
};

const TTL: Duration = Duration::from_secs(1);

pub struct PassthroughFilesystem {
    root: PathBuf,
    filter: crate::filter::Filter,
    read_only: bool,
    allow_symlinks: bool,
    preserve_permissions: bool,
    inodes: Mutex<InodeMap>,
    handles: Mutex<HashMap<u64, File>>,
    next_handle: AtomicU64,
}

struct InodeMap {
    by_ino: HashMap<u64, PathBuf>,
    by_path: HashMap<PathBuf, u64>,
    next: u64,
}

impl PassthroughFilesystem {
    pub fn new(
        source: &Path,
        filter: crate::filter::Filter,
        read_only: bool,
        allow_symlinks: bool,
        preserve_permissions: bool,
    ) -> io::Result<Self> {
        let root = std::fs::canonicalize(source)?;
        let root_ino = u64::from(INodeNo::ROOT);

        let mut inodes = InodeMap {
            by_ino: HashMap::new(),
            by_path: HashMap::new(),
            next: root_ino + 1,
        };
        inodes.by_ino.insert(root_ino, root.clone());
        inodes.by_path.insert(root.clone(), root_ino);

        Ok(Self {
            root,
            filter,
            read_only,
            allow_symlinks,
            preserve_permissions,
            inodes: Mutex::new(inodes),
            handles: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
        })
    }

    fn path_for(&self, ino: INodeNo) -> Option<PathBuf> {
        self.inodes
            .lock()
            .expect("inode map poisoned")
            .by_ino
            .get(&u64::from(ino))
            .cloned()
    }

    fn inode_for(&self, path: &Path) -> INodeNo {
        let mut inodes = self.inodes.lock().expect("inode map poisoned");
        if let Some(ino) = inodes.by_path.get(path) {
            return INodeNo(*ino);
        }

        let ino = inodes.next;
        inodes.next += 1;
        inodes.by_ino.insert(ino, path.to_path_buf());
        inodes.by_path.insert(path.to_path_buf(), ino);
        INodeNo(ino)
    }

    fn remove_path(&self, path: &Path) {
        let mut inodes = self.inodes.lock().expect("inode map poisoned");
        if let Some(ino) = inodes.by_path.remove(path) {
            inodes.by_ino.remove(&ino);
        }
    }

    fn rename_path(&self, from: &Path, to: &Path) {
        let mut inodes = self.inodes.lock().expect("inode map poisoned");
        let updates: Vec<(u64, PathBuf)> = inodes
            .by_ino
            .iter()
            .filter_map(|(ino, path)| {
                path.strip_prefix(from)
                    .ok()
                    .map(|rest| (*ino, to.join(rest)))
            })
            .collect();

        for (ino, path) in updates {
            if let Some(old) = inodes.by_ino.insert(ino, path.clone()) {
                inodes.by_path.remove(&old);
            }
            inodes.by_path.insert(path, ino);
        }
    }

    fn relative<'a>(&self, path: &'a Path) -> &'a Path {
        path.strip_prefix(&self.root).unwrap_or(path)
    }

    fn is_visible(&self, path: &Path) -> bool {
        if path == self.root {
            return true;
        }
        !self.filter.is_parent_excluded(self.relative(path))
    }

    fn symlink_visible(&self, path: &Path, metadata: &Metadata) -> bool {
        if !metadata.file_type().is_symlink() {
            return true;
        }
        if !self.allow_symlinks {
            return false;
        }
        match std::fs::canonicalize(path) {
            Ok(target) => target.starts_with(&self.root) && self.is_visible(&target),
            Err(_) => false,
        }
    }

    fn entry_visible(&self, path: &Path, metadata: &Metadata) -> bool {
        self.is_visible(path) && self.symlink_visible(path, metadata)
    }

    fn child_path(&self, parent: INodeNo, name: &OsStr) -> Result<PathBuf, Errno> {
        let parent_path = self.path_for(parent).ok_or(Errno::ENOENT)?;
        singleton_name(name)?;
        Ok(parent_path.join(name))
    }

    fn metadata_to_attr(&self, ino: INodeNo, metadata: &Metadata) -> FileAttr {
        let kind = file_type(metadata.file_type());
        let perm = if self.preserve_permissions {
            (metadata.mode() & 0o7777) as u16
        } else if kind == FileType::Directory {
            0o755
        } else {
            0o644
        };

        FileAttr {
            ino,
            size: metadata.len(),
            blocks: metadata.blocks(),
            atime: to_system_time(metadata.atime(), metadata.atime_nsec()),
            mtime: to_system_time(metadata.mtime(), metadata.mtime_nsec()),
            ctime: to_system_time(metadata.ctime(), metadata.ctime_nsec()),
            crtime: UNIX_EPOCH,
            kind,
            perm,
            nlink: metadata.nlink() as u32,
            uid: metadata.uid(),
            gid: metadata.gid(),
            rdev: metadata.rdev() as u32,
            blksize: if metadata.blksize() == 0 {
                512
            } else {
                metadata.blksize() as u32
            },
            flags: 0,
        }
    }

    fn lookup_path(&self, parent: INodeNo, name: &OsStr) -> Result<(PathBuf, Metadata), Errno> {
        let path = self.child_path(parent, name)?;
        if !self.is_visible(&path) {
            return Err(Errno::ENOENT);
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(Errno::from)?;
        if !self.symlink_visible(&path, &metadata) {
            return Err(Errno::ENOENT);
        }
        Ok((path, metadata))
    }

    fn store_handle(&self, file: File) -> FileHandle {
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
        self.handles
            .lock()
            .expect("handle map poisoned")
            .insert(handle, file);
        FileHandle(handle)
    }

    fn handle(&self, handle: FileHandle) -> Result<File, Errno> {
        self.handles
            .lock()
            .expect("handle map poisoned")
            .get(&u64::from(handle))
            .ok_or(Errno::EBADF)?
            .try_clone()
            .map_err(Errno::from)
    }

    fn reply_attr_for(&self, reply: ReplyAttr, path: &Path, ino: INodeNo) {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => reply.attr(&TTL, &self.metadata_to_attr(ino, &metadata)),
            Err(error) => reply.error(Errno::from(error)),
        }
    }
}

impl Filesystem for PassthroughFilesystem {
    fn init(&mut self, _req: &Request, _config: &mut KernelConfig) -> io::Result<()> {
        tracing::info!(source = %self.root.display(), "Scrim filesystem mounted");
        Ok(())
    }

    fn destroy(&mut self) {
        tracing::info!("Scrim filesystem unmounted");
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        match self.lookup_path(parent, name) {
            Ok((path, metadata)) => {
                let ino = self.inode_for(&path);
                reply.entry(&TTL, &self.metadata_to_attr(ino, &metadata), Generation(0));
            }
            Err(error) => reply.error(error),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let Some(path) = self.path_for(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !self.is_visible(&path) {
            reply.error(Errno::ENOENT);
            return;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !self.symlink_visible(&path, &metadata) {
                    reply.error(Errno::ENOENT);
                    return;
                }
                reply.attr(&TTL, &self.metadata_to_attr(ino, &metadata));
            }
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let Some(path) = self.path_for(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !self.is_visible(&path) {
            reply.error(Errno::ENOENT);
            return;
        }

        let changing = mode.is_some()
            || uid.is_some()
            || gid.is_some()
            || size.is_some()
            || atime.is_some()
            || mtime.is_some();
        if self.read_only && changing {
            reply.error(Errno::EROFS);
            return;
        }

        let Ok(c_path) = c_string(&path) else {
            reply.error(Errno::EINVAL);
            return;
        };

        unsafe {
            if let Some(size) = size {
                if libc::truncate(c_path.as_ptr(), size as libc::off_t) != 0 {
                    reply.error(Errno::from(io::Error::last_os_error()));
                    return;
                }
            }
            if let Some(mode) = mode {
                if libc::chmod(c_path.as_ptr(), (mode & 0o7777) as libc::mode_t) != 0 {
                    reply.error(Errno::from(io::Error::last_os_error()));
                    return;
                }
            }
            if uid.is_some() || gid.is_some() {
                let uid = uid.map_or(!0, |value| value as libc::uid_t);
                let gid = gid.map_or(!0, |value| value as libc::gid_t);
                if libc::chown(c_path.as_ptr(), uid, gid) != 0 {
                    reply.error(Errno::from(io::Error::last_os_error()));
                    return;
                }
            }
            if atime.is_some() || mtime.is_some() {
                let times = [
                    atime.as_ref().map(time_or_now).unwrap_or(libc::timespec {
                        tv_sec: 0,
                        tv_nsec: libc::UTIME_OMIT,
                    }),
                    mtime.as_ref().map(time_or_now).unwrap_or(libc::timespec {
                        tv_sec: 0,
                        tv_nsec: libc::UTIME_OMIT,
                    }),
                ];
                if libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) != 0 {
                    reply.error(Errno::from(io::Error::last_os_error()));
                    return;
                }
            }
        }

        self.reply_attr_for(reply, &path, ino);
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        let Some(path) = self.path_for(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !self.is_visible(&path) {
            reply.error(Errno::ENOENT);
            return;
        }
        match std::fs::read_link(&path) {
            Ok(target) => reply.data(target.as_os_str().as_bytes()),
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        if self.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let path = match self.child_path(parent, name) {
            Ok(path) => path,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        if !self.is_visible(&path) {
            reply.error(Errno::EACCES);
            return;
        }
        let Ok(c_path) = c_string(&path) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let mode = (mode & !umask & 0o7777) as libc::mode_t;
        if unsafe { libc::mkdir(c_path.as_ptr(), mode) } != 0 {
            reply.error(Errno::from(io::Error::last_os_error()));
            return;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let ino = self.inode_for(&path);
                reply.entry(&TTL, &self.metadata_to_attr(ino, &metadata), Generation(0));
            }
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        if self.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let path = match self.lookup_path(parent, name) {
            Ok((path, _)) => path,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        match std::fs::remove_file(&path) {
            Ok(()) => {
                self.remove_path(&path);
                reply.ok();
            }
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        if self.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let path = match self.lookup_path(parent, name) {
            Ok((path, _)) => path,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        match std::fs::remove_dir(&path) {
            Ok(()) => {
                self.remove_path(&path);
                reply.ok();
            }
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        _flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        if self.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let source = match self.lookup_path(parent, name) {
            Ok((path, _)) => path,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let destination = match self.child_path(newparent, newname) {
            Ok(path) => path,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        if !self.is_visible(&destination) {
            reply.error(Errno::EACCES);
            return;
        }
        match std::fs::rename(&source, &destination) {
            Ok(()) => {
                self.rename_path(&source, &destination);
                reply.ok();
            }
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    fn link(
        &self,
        _req: &Request,
        ino: INodeNo,
        newparent: INodeNo,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        if self.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let Some(source) = self.path_for(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !self.is_visible(&source) {
            reply.error(Errno::ENOENT);
            return;
        }
        let destination = match self.child_path(newparent, newname) {
            Ok(path) => path,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        if !self.is_visible(&destination) {
            reply.error(Errno::EACCES);
            return;
        }
        match std::fs::hard_link(&source, &destination) {
            Ok(()) => match std::fs::symlink_metadata(&destination) {
                Ok(metadata) => {
                    let destination_ino = self.inode_for(&destination);
                    reply.entry(
                        &TTL,
                        &self.metadata_to_attr(destination_ino, &metadata),
                        Generation(0),
                    );
                }
                Err(error) => reply.error(Errno::from(error)),
            },
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    fn symlink(
        &self,
        _req: &Request,
        parent: INodeNo,
        link_name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        if self.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        if !self.allow_symlinks {
            reply.error(Errno::EPERM);
            return;
        }
        if target.is_absolute() || target.components().any(|c| c == Component::ParentDir) {
            reply.error(Errno::EPERM);
            return;
        }
        let path = match self.child_path(parent, link_name) {
            Ok(path) => path,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        if !self.is_visible(&path) {
            reply.error(Errno::EACCES);
            return;
        }
        let (Ok(c_path), Ok(c_target)) = (c_string(&path), c_string(target)) else {
            reply.error(Errno::EINVAL);
            return;
        };
        if unsafe { libc::symlink(c_target.as_ptr(), c_path.as_ptr()) } != 0 {
            reply.error(Errno::from(io::Error::last_os_error()));
            return;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let ino = self.inode_for(&path);
                reply.entry(&TTL, &self.metadata_to_attr(ino, &metadata), Generation(0));
            }
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        if self.read_only && (flags.0 & libc::O_ACCMODE) != libc::O_RDONLY {
            reply.error(Errno::EROFS);
            return;
        }
        let Some(path) = self.path_for(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !self.is_visible(&path) {
            reply.error(Errno::ENOENT);
            return;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if !self.symlink_visible(&path, &metadata) => {
                reply.error(Errno::ENOENT);
                return;
            }
            Ok(_) => {}
            Err(error) => {
                reply.error(Errno::from(error));
                return;
            }
        }

        let Ok(c_path) = c_string(&path) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let fd = unsafe { libc::open(c_path.as_ptr(), flags.0 | libc::O_CLOEXEC) };
        if fd < 0 {
            reply.error(Errno::from(io::Error::last_os_error()));
            return;
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let handle = self.store_handle(file);
        reply.opened(handle, FopenFlags::empty());
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let file = match self.handle(fh) {
            Ok(file) => file,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let mut buffer = vec![0u8; size as usize];
        match file.read_at(&mut buffer, offset) {
            Ok(read) => reply.data(&buffer[..read]),
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        if self.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let file = match self.handle(fh) {
            Ok(file) => file,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let mut written = 0usize;
        while written < data.len() {
            match file.write_at(&data[written..], offset + written as u64) {
                Ok(0) => break,
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    reply.error(Errno::from(error));
                    return;
                }
            }
        }
        reply.written(written as u32);
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.handles
            .lock()
            .expect("handle map poisoned")
            .remove(&u64::from(fh));
        reply.ok();
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        let file = match self.handle(fh) {
            Ok(file) => file,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let result = if datasync {
            file.sync_data()
        } else {
            file.sync_all()
        };
        match result {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(Errno::from(error)),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let Some(path) = self.path_for(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !self.is_visible(&path) {
            reply.error(Errno::ENOENT);
            return;
        }

        let parent_ino = if ino == INodeNo::ROOT {
            INodeNo::ROOT
        } else {
            self.inode_for(path.parent().unwrap_or(&self.root))
        };

        let mut entries: Vec<(INodeNo, FileType, OsString)> = Vec::new();
        match std::fs::read_dir(&path) {
            Ok(read_dir) => {
                for entry in read_dir {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(error) => {
                            reply.error(Errno::from(error));
                            return;
                        }
                    };
                    let entry_path = path.join(entry.file_name());
                    let metadata = match std::fs::symlink_metadata(&entry_path) {
                        Ok(metadata) => metadata,
                        Err(_) => continue,
                    };
                    if !self.entry_visible(&entry_path, &metadata) {
                        continue;
                    }
                    entries.push((
                        self.inode_for(&entry_path),
                        file_type(metadata.file_type()),
                        entry.file_name(),
                    ));
                }
            }
            Err(error) => {
                reply.error(Errno::from(error));
                return;
            }
        }
        entries.sort_by(|left, right| left.2.cmp(&right.2));
        entries.insert(0, (parent_ino, FileType::Directory, OsString::from("..")));
        entries.insert(0, (ino, FileType::Directory, OsString::from(".")));

        for (index, (entry_ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
            if reply.add(*entry_ino, (index + 1) as u64, *kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn access(&self, _req: &Request, ino: INodeNo, _mask: AccessFlags, reply: ReplyEmpty) {
        match self.path_for(ino) {
            Some(path) if self.is_visible(&path) => reply.ok(),
            _ => reply.error(Errno::ENOENT),
        }
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        let Ok(c_path) = c_string(&self.root) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            reply.error(Errno::from(io::Error::last_os_error()));
            return;
        }
        let stats = unsafe { stats.assume_init() };
        reply.statfs(
            stats.f_blocks,
            stats.f_bfree,
            stats.f_bavail,
            stats.f_files,
            stats.f_ffree,
            stats.f_bsize as u32,
            stats.f_namemax as u32,
            if stats.f_frsize == 0 {
                stats.f_bsize as u32
            } else {
                stats.f_frsize as u32
            },
        );
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        if self.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let path = match self.child_path(parent, name) {
            Ok(path) => path,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        if !self.is_visible(&path) {
            reply.error(Errno::EACCES);
            return;
        }
        let Ok(c_path) = c_string(&path) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let mode = (mode & !umask & 0o7777) as libc::mode_t;
        let fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                flags | libc::O_CREAT | libc::O_CLOEXEC,
                mode,
            )
        };
        if fd < 0 {
            reply.error(Errno::from(io::Error::last_os_error()));
            return;
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                reply.error(Errno::from(error));
                return;
            }
        };
        let ino = self.inode_for(&path);
        let handle = self.store_handle(file);
        reply.created(
            &TTL,
            &self.metadata_to_attr(ino, &metadata),
            Generation(0),
            handle,
            FopenFlags::empty(),
        );
    }
}

fn singleton_name(name: &OsStr) -> Result<(), Errno> {
    if name.is_empty() {
        return Err(Errno::EINVAL);
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(Errno::EINVAL),
    }
}

fn c_string(path: &Path) -> Result<CString, Errno> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| Errno::EINVAL)
}

fn file_type(file_type: std::fs::FileType) -> FileType {
    if file_type.is_dir() {
        FileType::Directory
    } else if file_type.is_file() {
        FileType::RegularFile
    } else if file_type.is_symlink() {
        FileType::Symlink
    } else if file_type.is_fifo() {
        FileType::NamedPipe
    } else if file_type.is_socket() {
        FileType::Socket
    } else if file_type.is_char_device() {
        FileType::CharDevice
    } else if file_type.is_block_device() {
        FileType::BlockDevice
    } else {
        FileType::RegularFile
    }
}

fn to_system_time(seconds: i64, nanoseconds: i64) -> SystemTime {
    let nanos = nanoseconds.clamp(0, 999_999_999) as u32;
    if seconds >= 0 {
        UNIX_EPOCH + Duration::new(seconds as u64, nanos)
    } else {
        UNIX_EPOCH - Duration::new(seconds.unsigned_abs(), 0) + Duration::new(0, nanos)
    }
}

fn time_or_now(value: &TimeOrNow) -> libc::timespec {
    match value {
        TimeOrNow::Now => libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_NOW,
        },
        TimeOrNow::SpecificTime(time) => system_time_to_timespec(time),
    }
}

fn system_time_to_timespec(time: &SystemTime) -> libc::timespec {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => libc::timespec {
            tv_sec: duration.as_secs() as libc::time_t,
            tv_nsec: duration.subsec_nanos() as libc::c_long,
        },
        Err(error) => {
            let duration = error.duration();
            libc::timespec {
                tv_sec: -(duration.as_secs() as libc::time_t),
                tv_nsec: duration.subsec_nanos() as libc::c_long,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::Filter;

    fn filesystem(patterns: Vec<String>) -> (tempfile::TempDir, PassthroughFilesystem) {
        let dir = tempfile::tempdir().unwrap();
        let filter = Filter::new(patterns, true).unwrap();
        let fs = PassthroughFilesystem::new(dir.path(), filter, false, false, true).unwrap();
        (dir, fs)
    }

    #[test]
    fn rejects_parent_traversal_names() {
        let (_dir, fs) = filesystem(Vec::new());
        assert_eq!(
            fs.child_path(INodeNo::ROOT, OsStr::new("..")).unwrap_err(),
            Errno::EINVAL
        );
        assert_eq!(
            fs.child_path(INodeNo::ROOT, OsStr::new("../secret"))
                .unwrap_err(),
            Errno::EINVAL
        );
        assert_eq!(
            fs.child_path(INodeNo::ROOT, OsStr::new("a/b")).unwrap_err(),
            Errno::EINVAL
        );
    }

    #[test]
    fn hidden_paths_are_not_visible() {
        let (_dir, fs) = filesystem(vec![".env".to_string(), ".git/".to_string()]);
        assert!(!fs.is_visible(&fs.root.join(".env")));
        assert!(!fs.is_visible(&fs.root.join("config/.env")));
        assert!(!fs.is_visible(&fs.root.join(".git/config")));
        assert!(fs.is_visible(&fs.root.join("app.py")));
        assert!(fs.is_visible(&fs.root));
    }

    #[test]
    fn inode_mapping_is_stable() {
        let (_dir, fs) = filesystem(Vec::new());
        let first = fs.inode_for(&fs.root.join("app.py"));
        let second = fs.inode_for(&fs.root.join("app.py"));
        assert_eq!(first, second);
        assert_ne!(first, INodeNo::ROOT);
    }
}
