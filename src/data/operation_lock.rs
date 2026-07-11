use super::sibling_path;
use anyhow::{anyhow, Context, Result};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(20 * 60);

#[derive(Debug)]
pub(super) struct OperationLock {
    _file: File,
}

pub(super) fn acquire(data_path: &Path, timeout: Duration) -> Result<OperationLock> {
    let lock_path = sibling_path(data_path, ".lock")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("打开数据操作锁失败: {}", lock_path.display()))?;
    let started = Instant::now();
    let mut reported_wait = false;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(OperationLock { _file: file }),
            Err(TryLockError::WouldBlock) => {
                if started.elapsed() >= timeout {
                    return Err(anyhow!(
                        "等待其他 fojin 数据操作超时: {}",
                        lock_path.display()
                    ));
                }
                if !reported_wait {
                    eprintln!("检测到另一个 fojin 数据操作,正在等待...");
                    reported_wait = true;
                }
                let remaining = timeout.saturating_sub(started.elapsed());
                std::thread::sleep(POLL_INTERVAL.min(remaining));
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("获取数据操作锁失败: {}", lock_path.display()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn competing_lock_times_out_then_succeeds_after_drop() {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data.sqlite");
        let first = acquire(&data, Duration::from_millis(100)).unwrap();
        let error = acquire(&data, Duration::from_millis(20))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("等待") && error.contains("超时"),
            "got: {error}"
        );
        drop(first);
        acquire(&data, Duration::from_millis(100)).unwrap();
        assert!(data.with_file_name("data.sqlite.lock").exists());
    }
}
