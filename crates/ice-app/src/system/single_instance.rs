//! 单实例互斥（需求 §5 第 5 条，REQ-009）。
//!
//! 这是"防同一份用户数据被两个进程并发写坏"的保险，不是授权机制：不做跨机器互斥，
//! 也不把第二个实例的意图转给第一个。
//!
//! 锁就是配置目录下的一个文本文件，内容是持有者 PID。判定陈旧锁靠 `kill(pid, 0)`——
//! macOS 没有 `/proc`，这是唯一可靠的存活探测。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const LOCK_NAME: &str = "ice-translator.lock";

/// 持锁句柄。`Drop` 时释放，且只删自己创建的那一份。
#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
    pid: u32,
}

impl InstanceLock {
    pub const fn pid(&self) -> u32 {
        self.pid
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug)]
pub enum AcquireError {
    /// 另一个仍然活着的进程持锁
    AlreadyRunning { pid: u32 },
    /// 文件系统层面的失败
    Io(String),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // 只提锁文件，不泄露其它路径细节
            AcquireError::AlreadyRunning { pid } => write!(
                f,
                "Ice Translator 已在运行（PID {pid}）。\n如果确实没有窗口，说明上次异常退出留下了锁文件，\n可在配置目录删除 {LOCK_NAME} 后重试。"
            ),
            AcquireError::Io(message) => {
                write!(
                    f,
                    "Ice Translator 无法取得单实例锁（{LOCK_NAME}）：{message}"
                )
            }
        }
    }
}

/// PID 是否对应一个还活着的进程。
///
/// `kill(-1, 0)` 会给所有可发信号的进程发信号，所以非正数与装不进 `i32` 的值一律按"不存活"处理。
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: 纯探测，信号 0 不投递；pid 已确认为正且能装进 i32。
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    // EPERM 表示进程存在但属于别的用户，仍算活着；ESRCH 才是真的没了
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub fn pid_alive(_pid: u32) -> bool {
    false
}

pub fn lock_path_in(dir: &Path) -> PathBuf {
    dir.join(LOCK_NAME)
}

pub fn acquire(dir: &Path) -> Result<InstanceLock, AcquireError> {
    fs::create_dir_all(dir).map_err(|e| AcquireError::Io(e.to_string()))?;
    let path = lock_path_in(dir);
    let my_pid = std::process::id();

    if path.exists() {
        match holder_pid(&path) {
            // 记着自己的 PID 只可能是 PID 被回收过的陈旧锁（本进程启动时必然还没持锁），一并接管
            Some(pid) if pid != my_pid && pid_alive(pid) => {
                return Err(AcquireError::AlreadyRunning { pid });
            }
            // None（内容损坏）或 PID 已消失：陈旧锁，接管
            _ => {
                let _ = fs::remove_file(&path);
            }
        }
    }

    // create_new 把"检查存在"和"创建"合成一步，避免两个实例同时通过上面的检查
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                AcquireError::AlreadyRunning {
                    pid: holder_pid(&path).unwrap_or(0),
                }
            } else {
                AcquireError::Io(e.to_string())
            }
        })?;

    writeln!(file, "{my_pid}").map_err(|e| {
        let _ = fs::remove_file(&path);
        AcquireError::Io(e.to_string())
    })?;

    Ok(InstanceLock { path, pid: my_pid })
}

/// 锁文件里记的 PID；文件读不出来或内容不是数字时返回 `None`（按陈旧处理）。
fn holder_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid != 0)
}

impl InstanceLock {
    /// 释放锁。只删 PID 与自己相符的那一份：别的实例可能在我们之后接管了同一个路径。
    /// `Drop` 里也做同一件事，重复调用安全。
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if holder_pid(&self.path) == Some(self.pid) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// 拿一个确定已经消失的 PID：起一个进程再等它结束。
    fn dead_pid() -> u32 {
        let child = Command::new("/usr/bin/true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /usr/bin/true");
        let pid = child.id();
        child.wait_with_output().expect("wait /usr/bin/true");
        pid
    }

    fn recorded_pid(path: &Path) -> u32 {
        fs::read_to_string(path).unwrap().trim().parse().unwrap()
    }

    #[test]
    fn acquire_creates_lock_with_own_pid_and_release_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path_in(dir.path());
        let guard = acquire(dir.path()).unwrap();
        assert_eq!(guard.pid(), std::process::id());
        assert_eq!(recorded_pid(&path), guard.pid());
        guard.release();
        assert!(!path.exists());
    }

    #[test]
    fn second_acquire_in_same_dir_is_rejected_while_holder_lives() {
        let dir = tempfile::tempdir().unwrap();
        // 锁必须由"别的进程"持有：`acquire` 认得自己的 PID 并按陈旧锁接管
        let mut holder = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /bin/sleep");
        let foreign = holder.id();
        fs::write(lock_path_in(dir.path()), format!("{foreign}\n")).unwrap();

        let err = acquire(dir.path()).unwrap_err();
        match &err {
            AcquireError::AlreadyRunning { pid } => assert_eq!(*pid, foreign),
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
        // 提示里出现"已在运行"，且不泄露锁文件之外的路径细节
        let text = err.to_string();
        assert!(text.contains("已在运行"));
        assert!(!text.contains("config.toml"));
        assert!(!text.contains("history"));

        let _ = holder.kill();
        let _ = holder.wait();
    }

    #[test]
    fn stale_lock_from_dead_pid_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path_in(dir.path());
        fs::write(&path, format!("{}\n", dead_pid())).unwrap();
        let guard = acquire(dir.path()).unwrap();
        assert_eq!(guard.pid(), std::process::id());
        assert_eq!(recorded_pid(&path), guard.pid());
    }

    #[test]
    fn a_stale_lock_recording_our_own_recycled_pid_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path_in(dir.path());
        fs::write(&path, format!("{}\n", std::process::id())).unwrap();
        let guard = acquire(dir.path()).unwrap();
        assert_eq!(guard.pid(), std::process::id());
        assert_eq!(recorded_pid(&path), guard.pid());
    }

    #[test]
    fn corrupt_lock_content_is_treated_as_stale() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(lock_path_in(dir.path()), "not-a-pid").unwrap();
        let guard = acquire(dir.path()).unwrap();
        assert_eq!(guard.pid(), std::process::id());
    }

    #[test]
    fn release_does_not_remove_someone_elses_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path_in(dir.path());
        let guard = acquire(dir.path()).unwrap();
        // 模拟另一个实例接管：锁文件被改写成别人的 PID
        let other = std::process::id().wrapping_add(1);
        fs::write(&path, format!("{other}\n")).unwrap();
        guard.release();
        assert!(path.exists(), "不属于自己的锁不应被删除");
        assert_eq!(recorded_pid(&path), other);
    }

    #[test]
    fn pid_alive_rejects_dangerous_values() {
        assert!(!pid_alive(0));
        assert!(!pid_alive(u32::MAX));
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(dead_pid()));
    }
}
