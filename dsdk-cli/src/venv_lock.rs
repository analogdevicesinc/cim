// Copyright (c) 2026 Analog Devices, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A small advisory lock guarding the one operation on a shared mirror
//! Python virtual environment that can actually corrupt it for every
//! workspace using that mirror: (re)creating the `.venv` directory itself,
//! whether because it didn't exist yet or because it was found broken and
//! is being wiped and recreated. Routine package installs into an
//! already-functional mirror venv are intentionally left unlocked -- pip/uv
//! installs are already expected to tolerate being run repeatedly, and
//! gating every install would slow down the common case for no benefit.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A lock file older than this is assumed to belong to a crashed/killed
/// process and is stolen. Generous on purpose: (re)creating a venv involves
/// several subprocess invocations (uv/pip, network access) and can
/// legitimately take a while.
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

/// How long to wait for a live lock to be released before giving up.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(60);

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// RAII guard for the advisory lock at `<mirror_venv_path>.lock`. The lock
/// file is removed when the guard is dropped.
pub(crate) struct MirrorLock {
    lock_path: PathBuf,
}

impl MirrorLock {
    /// Acquire the lock guarding `mirror_venv_path`. Waits up to
    /// `ACQUIRE_TIMEOUT` for a live holder to finish; a lock file older than
    /// `STALE_AFTER` is treated as abandoned and stolen automatically.
    pub(crate) fn acquire(mirror_venv_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        // The lock file lives next to `.venv`, not inside it -- the whole
        // point is to guard operations that remove and recreate that
        // directory.
        let lock_path = lock_path_for(mirror_venv_path);
        let deadline = SystemTime::now() + ACQUIRE_TIMEOUT;

        loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}\n{}", std::process::id(), now_secs());
                    return Ok(MirrorLock { lock_path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&lock_path) {
                        let _ = std::fs::remove_file(&lock_path);
                        continue;
                    }
                    if SystemTime::now() >= deadline {
                        return Err(format!(
                            "Timed out waiting for another cim process to finish updating the \
                             shared virtual environment (lock file: {}). If no other `cim \
                             install pip` is actually running, remove that file and retry.",
                            lock_path.display()
                        )
                        .into());
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to create lock file {}: {}",
                        lock_path.display(),
                        e
                    )
                    .into())
                }
            }
        }
    }
}

impl Drop for MirrorLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

fn lock_path_for(mirror_venv_path: &Path) -> PathBuf {
    let mut name = mirror_venv_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".lock");
    match mirror_venv_path.parent() {
        Some(parent) => parent.join(&name),
        None => PathBuf::from(&name),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn lock_is_stale(lock_path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(lock_path) else {
        // Can't read it (e.g. a race with the holder removing it) -- not
        // stale as far as we know, the retry loop will re-check.
        return false;
    };
    let Some(timestamp_line) = contents.lines().nth(1) else {
        return false;
    };
    let Ok(created_secs) = timestamp_line.trim().parse::<u64>() else {
        return false;
    };
    now_secs().saturating_sub(created_secs) > STALE_AFTER.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_and_release() {
        let temp_dir = TempDir::new().unwrap();
        let venv_path = temp_dir.path().join(".venv");
        let lock_path = lock_path_for(&venv_path);

        {
            let _lock = MirrorLock::acquire(&venv_path).unwrap();
            assert!(lock_path.exists());
        }
        assert!(!lock_path.exists(), "lock file should be removed on drop");
    }

    #[test]
    fn test_stale_lock_is_stolen() {
        let temp_dir = TempDir::new().unwrap();
        let venv_path = temp_dir.path().join(".venv");
        let lock_path = lock_path_for(&venv_path);

        // Fabricate an old lock file as if left behind by a crashed process.
        let ancient = now_secs().saturating_sub(STALE_AFTER.as_secs() + 60);
        std::fs::write(&lock_path, format!("999999\n{}", ancient)).unwrap();

        let lock = MirrorLock::acquire(&venv_path).expect("stale lock should be stolen");
        drop(lock);
    }

    #[test]
    fn test_fresh_lock_is_not_stale() {
        let temp_dir = TempDir::new().unwrap();
        let venv_path = temp_dir.path().join(".venv");
        let lock_path = lock_path_for(&venv_path);

        // A freshly-created lock file is not stale.
        std::fs::write(
            &lock_path,
            format!("{}\n{}", std::process::id(), now_secs()),
        )
        .unwrap();
        assert!(!lock_is_stale(&lock_path));
    }
}
