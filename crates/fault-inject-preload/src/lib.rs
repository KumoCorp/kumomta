//! LD_PRELOAD shim that faults writes aimed at configured path prefixes, used
//! by integration tests to reproduce storage faults (a full disk, a failing or
//! slow device) that are otherwise hard to provoke deterministically without
//! root.
//!
//! A test scopes injection by directory, filename, and time, letting it fault
//! one subsystem while the rest of the process keeps working. The directory
//! scope is a colon-separated list of absolute prefixes in
//! `KUMO_FAULT_PATH_PREFIXES`. The filename scope is an optional
//! colon-separated list of suffixes in `KUMO_FAULT_PATH_SUFFIXES` (e.g. `.sst`
//! to hit only flushed table files and leave the write-ahead log untouched).
//! The time scope is a sentinel file at `KUMO_FAULT_ACTIVE_FILE`: faults apply
//! only while that file exists. A test lets startup complete and creates the
//! sentinel to begin the fault, then removes it to restore service.
//!
//! Two fault kinds are available. By default a faulted write fails with an
//! errno (`ENOSPC`, overridable via `KUMO_FAULT_ERRNO`), modelling a full or
//! failing disk. If `KUMO_FAULT_DELAY_MS` is non-zero a faulted write instead
//! sleeps that long and then succeeds, modelling slow storage -- which, applied
//! to flush writes, stalls RocksDB into rejecting foreground writes with
//! `Incomplete` and exercises the backpressure paths.

#[cfg(target_os = "linux")]
mod linux {
    use libc::{c_char, c_int, c_void, iovec, mode_t, off_t, size_t, ssize_t};
    use std::ffi::CStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use std::time::Duration;

    // Upper bound on the fd numbers `TRACKED` covers. rocksdb keeps its
    // descriptors well below this bound. Anything above it is treated as
    // untracked.
    const MAX_FD: usize = 1 << 16;

    // Whether each fd falls under a faulted prefix, indexed by fd number.
    //
    // Only open/openat/close update this table. A duplicate of a tracked fd via
    // dup/dup2/fcntl(F_DUPFD) would come up untracked and bypass injection
    // through the duplicate. The file layer of RocksDB does not duplicate fds
    // today, and the current test is unaffected. A future use of this shim
    // against code that duplicates fds would need those interposed too.
    static TRACKED: [AtomicBool; MAX_FD] = [const { AtomicBool::new(false) }; MAX_FD];

    struct Config {
        prefixes: Vec<Vec<u8>>,
        suffixes: Vec<Vec<u8>>,
        sentinel: Option<Vec<u8>>,
        errno: c_int,
        delay: Duration,
    }

    fn split_env(name: &str) -> Vec<Vec<u8>> {
        std::env::var(name)
            .ok()
            .map(|v| {
                v.split(':')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.as_bytes().to_vec())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn config() -> &'static Config {
        static CONFIG: OnceLock<Config> = OnceLock::new();
        CONFIG.get_or_init(|| {
            let prefixes = split_env("KUMO_FAULT_PATH_PREFIXES");
            let suffixes = split_env("KUMO_FAULT_PATH_SUFFIXES");
            let sentinel = std::env::var("KUMO_FAULT_ACTIVE_FILE").ok().map(|mut v| {
                v.push('\0');
                v.into_bytes()
            });
            let errno = std::env::var("KUMO_FAULT_ERRNO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(libc::ENOSPC);
            let delay = std::env::var("KUMO_FAULT_DELAY_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(Duration::from_millis)
                .unwrap_or_default();
            Config {
                prefixes,
                suffixes,
                sentinel,
                errno,
                delay,
            }
        })
    }

    // True while the sentinel file exists, meaning faults should be applied.
    fn faults_active() -> bool {
        match &config().sentinel {
            Some(path) => unsafe { libc::access(path.as_ptr() as *const c_char, libc::F_OK) == 0 },
            None => false,
        }
    }

    fn path_is_faulted(path: *const c_char) -> bool {
        if path.is_null() {
            return false;
        }
        let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
        let cfg = config();
        let under_prefix = cfg.prefixes.iter().any(|prefix| bytes.starts_with(prefix));
        let suffix_ok =
            cfg.suffixes.is_empty() || cfg.suffixes.iter().any(|suffix| bytes.ends_with(suffix));
        under_prefix && suffix_ok
    }

    fn set_tracked(fd: c_int, faulted: bool) {
        if (0..MAX_FD as c_int).contains(&fd) {
            TRACKED[fd as usize].store(faulted, Ordering::Relaxed);
        }
    }

    fn is_tracked(fd: c_int) -> bool {
        (0..MAX_FD as c_int).contains(&fd) && TRACKED[fd as usize].load(Ordering::Relaxed)
    }

    // Decides how to fault a write to `fd`. Returns true when the caller should
    // inject the error now, which it signals by skipping the real write and
    // returning that error instead. In delay mode a faulted write sleeps here
    // and returns false, so the caller falls through to the real (now slowed)
    // write. An unfaulted fd returns false immediately, with no delay.
    //
    // `close` clears the tracked bit for its fd. A later `open` that the kernel
    // assigns the same fd number is tracked from the path it opens, independent
    // of what that fd number faulted on before.
    fn should_fail(fd: c_int) -> bool {
        if !(is_tracked(fd) && faults_active()) {
            return false;
        }
        let delay = config().delay;
        if delay.is_zero() {
            return true;
        }
        std::thread::sleep(delay);
        false
    }

    unsafe fn set_errno() {
        *libc::__errno_location() = config().errno;
    }

    // Resolves the libc implementation that our interposed symbol shadows.
    macro_rules! real {
        ($cell:ident, $ty:ty, $sym:literal) => {{
            static $cell: OnceLock<$ty> = OnceLock::new();
            *$cell.get_or_init(|| unsafe {
                let ptr = libc::dlsym(
                    libc::RTLD_NEXT,
                    concat!($sym, "\0").as_ptr() as *const c_char,
                );
                assert!(!ptr.is_null(), concat!("dlsym failed for ", $sym));
                std::mem::transmute::<*mut c_void, $ty>(ptr)
            })
        }};
    }

    type OpenFn = unsafe extern "C" fn(*const c_char, c_int, mode_t) -> c_int;
    type OpenatFn = unsafe extern "C" fn(c_int, *const c_char, c_int, mode_t) -> c_int;
    type WriteFn = unsafe extern "C" fn(c_int, *const c_void, size_t) -> ssize_t;
    type PwriteFn = unsafe extern "C" fn(c_int, *const c_void, size_t, off_t) -> ssize_t;
    type WritevFn = unsafe extern "C" fn(c_int, *const iovec, c_int) -> ssize_t;
    type PwritevFn = unsafe extern "C" fn(c_int, *const iovec, c_int, off_t) -> ssize_t;
    type FsyncFn = unsafe extern "C" fn(c_int) -> c_int;
    type FallocateFn = unsafe extern "C" fn(c_int, c_int, off_t, off_t) -> c_int;
    type FtruncateFn = unsafe extern "C" fn(c_int, off_t) -> c_int;
    type CloseFn = unsafe extern "C" fn(c_int) -> c_int;

    // The `mode` argument of `open`/`openat` is variadic in C and only read
    // when `O_CREAT`/`O_TMPFILE` is set. Declaring it as a fixed parameter
    // and forwarding it is sound because the real call ignores it otherwise.
    unsafe extern "C" fn open_impl(fd: c_int, path: *const c_char) {
        set_tracked(fd, fd >= 0 && path_is_faulted(path));
    }

    #[no_mangle]
    pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        let fd = real!(REAL_OPEN, OpenFn, "open")(path, flags, mode);
        open_impl(fd, path);
        fd
    }

    #[no_mangle]
    pub unsafe extern "C" fn open64(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        let fd = real!(REAL_OPEN64, OpenFn, "open64")(path, flags, mode);
        open_impl(fd, path);
        fd
    }

    #[no_mangle]
    pub unsafe extern "C" fn openat(
        dirfd: c_int,
        path: *const c_char,
        flags: c_int,
        mode: mode_t,
    ) -> c_int {
        let fd = real!(REAL_OPENAT, OpenatFn, "openat")(dirfd, path, flags, mode);
        open_impl(fd, path);
        fd
    }

    #[no_mangle]
    pub unsafe extern "C" fn openat64(
        dirfd: c_int,
        path: *const c_char,
        flags: c_int,
        mode: mode_t,
    ) -> c_int {
        let fd = real!(REAL_OPENAT64, OpenatFn, "openat64")(dirfd, path, flags, mode);
        open_impl(fd, path);
        fd
    }

    #[no_mangle]
    pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
        if should_fail(fd) {
            set_errno();
            return -1;
        }
        real!(REAL_WRITE, WriteFn, "write")(fd, buf, count)
    }

    #[no_mangle]
    pub unsafe extern "C" fn pwrite(
        fd: c_int,
        buf: *const c_void,
        count: size_t,
        offset: off_t,
    ) -> ssize_t {
        if should_fail(fd) {
            set_errno();
            return -1;
        }
        real!(REAL_PWRITE, PwriteFn, "pwrite")(fd, buf, count, offset)
    }

    #[no_mangle]
    pub unsafe extern "C" fn writev(fd: c_int, iov: *const iovec, iovcnt: c_int) -> ssize_t {
        if should_fail(fd) {
            set_errno();
            return -1;
        }
        real!(REAL_WRITEV, WritevFn, "writev")(fd, iov, iovcnt)
    }

    #[no_mangle]
    pub unsafe extern "C" fn pwritev(
        fd: c_int,
        iov: *const iovec,
        iovcnt: c_int,
        offset: off_t,
    ) -> ssize_t {
        if should_fail(fd) {
            set_errno();
            return -1;
        }
        real!(REAL_PWRITEV, PwritevFn, "pwritev")(fd, iov, iovcnt, offset)
    }

    #[no_mangle]
    pub unsafe extern "C" fn fsync(fd: c_int) -> c_int {
        if should_fail(fd) {
            set_errno();
            return -1;
        }
        real!(REAL_FSYNC, FsyncFn, "fsync")(fd)
    }

    #[no_mangle]
    pub unsafe extern "C" fn fdatasync(fd: c_int) -> c_int {
        if should_fail(fd) {
            set_errno();
            return -1;
        }
        real!(REAL_FDATASYNC, FsyncFn, "fdatasync")(fd)
    }

    #[no_mangle]
    pub unsafe extern "C" fn fallocate(fd: c_int, mode: c_int, offset: off_t, len: off_t) -> c_int {
        if should_fail(fd) {
            set_errno();
            return -1;
        }
        real!(REAL_FALLOCATE, FallocateFn, "fallocate")(fd, mode, offset, len)
    }

    #[no_mangle]
    pub unsafe extern "C" fn ftruncate(fd: c_int, length: off_t) -> c_int {
        if should_fail(fd) {
            set_errno();
            return -1;
        }
        real!(REAL_FTRUNCATE, FtruncateFn, "ftruncate")(fd, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn close(fd: c_int) -> c_int {
        set_tracked(fd, false);
        real!(REAL_CLOSE, CloseFn, "close")(fd)
    }
}
