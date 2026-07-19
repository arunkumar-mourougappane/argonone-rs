//! Cross-process advisory locking (`flock(2)`), used to serialize I2C
//! bus access (`hardware::i2c`) between the long-running daemon and the
//! one-shot `SHUTDOWN`/`FANOFF` CLI commands, which are invoked
//! independently of the daemon (a systemd shutdown hook script, or a
//! manual admin command) and so can genuinely run concurrently with it —
//! two separate processes each opening `/dev/i2c-1` on their own file
//! descriptor, with no in-process `Mutex` between them. `flock` locks
//! are per *open file description*, not per process, which is exactly
//! why this needs its own file rather than reusing the in-process
//! `Mutex<LinuxI2CDevice>` `hardware::i2c::I2cFan` already holds.
//!
//! Not `cfg(target_os = "linux")`-gated like `hardware::i2c` itself:
//! `flock` and the `libc` crate both work identically on macOS, which
//! keeps this module's core mutual-exclusion behavior unit-testable on
//! a non-Linux dev machine, the same reasoning `board::probe_with_retries`
//! already uses for its own testability.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// Holds an exclusive `flock` for as long as it's alive — dropping it
/// (closing the underlying file descriptor) releases the lock, same as
/// letting the file go out of scope.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct FileLock(
    // Never read — held purely so `Drop`ping it (closing the fd) is what
    // releases the flock. This is the RAII guard itself, not a value.
    #[allow(dead_code)] File,
);

/// Blocks until an exclusive lock on `path` is acquired, creating the
/// file first if it doesn't exist. Intentionally blocking, not
/// try-lock: a one-shot `SHUTDOWN`/`FANOFF` command should wait out a
/// daemon poll cycle (each I2C operation is quick) rather than silently
/// skip signaling the case MCU.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn acquire_exclusive(path: &Path) -> io::Result<FileLock> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: `file`'s fd is valid and owned by us for the duration of
    // this call; `flock` only affects lock state, not the fd itself.
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileLock(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn acquire_exclusive_blocks_a_second_locker_until_the_first_releases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("i2c.lock");

        let first = acquire_exclusive(&path).expect("first lock should succeed immediately");

        let (unlocked_tx, unlocked_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let path2 = path.clone();
        let handle = std::thread::spawn(move || {
            // Blocks here until `first` is dropped below.
            let _second = acquire_exclusive(&path2).expect("second lock should eventually succeed");
            acquired_tx.send(()).unwrap();
            // Hold it open until the test's done asserting.
            unlocked_rx.recv().ok();
        });

        // The second locker must not have acquired it yet — give it a
        // moment to prove it's genuinely blocked, not just slow to start.
        assert!(
            acquired_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "second locker acquired the lock while the first was still held"
        );

        drop(first);

        acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second locker should acquire the lock once the first releases");

        unlocked_tx.send(()).unwrap();
        handle.join().unwrap();
    }
}
