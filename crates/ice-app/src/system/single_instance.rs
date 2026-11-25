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
            Some(pid)
                if pid != my_pid && pid_alive(pid) && holder_is_us(pid, holder_started(&path)) =>
            {
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

    // Linux 多写一行启动时刻（见 `holder_is_us`）；写不进去就只写 PID，判据退回老口径
    #[cfg(target_os = "linux")]
    let body = match start_time(my_pid) {
        Some(started) => format!("{my_pid}\n{started}\n"),
        None => format!("{my_pid}\n"),
    };
    #[cfg(not(target_os = "linux"))]
    let body = format!("{my_pid}\n");
    file.write_all(body.as_bytes()).map_err(|e| {
        let _ = fs::remove_file(&path);
        AcquireError::Io(e.to_string())
    })?;

    Ok(InstanceLock { path, pid: my_pid })
}

/// 这个 PID **还是锁写下的那个进程**吗？
///
/// `kill(pid, 0)` 只能答"有个活进程占着这个号"，答不了"它是不是当年写下锁的那一个"。
/// 本机 2026-11-25 实测过后者：锁里写着 5712，而 5712 已经是 GNOME 的 `gsd-usb-protection`，
/// 于是新版本被"已在运行"挡在门外、桌面上却没有窗口。
///
/// Linux 上锁文件多写一行**进程启动时刻**（`/proc/<pid>/stat` 的第 22 字段），复用的 PID
/// 启动时刻必然对不上——这是内核给的最便宜的进程身份。mac/Windows 那边锁文件仍是一行
/// （格式没变），`recorded` 为 `None` 时退回"活着就算它在"的老判据。
#[cfg(target_os = "linux")]
fn holder_is_us(pid: u32, recorded: Option<u64>) -> bool {
    match recorded {
        Some(written) => match start_time(pid) {
            Some(now) => written == now,
            // 读不到 stat：判不出来时保守地当它是我们
            None => true,
        },
        // 旧版锁（只有一行）：没有启动时刻可比，退回"进程名是不是我们"。
        // 这一步正是 2026-11-25 那次的解：锁里 5712 的 comm 是 `gsd-usb-prote…`，不是我们
        None => same_program_name(pid),
    }
}

/// `/proc/<pid>/comm` 与我们自己的可执行文件名（内核截到 15 字节）是否一致。
/// 读不到 comm 时按"是它"处理——保守那一侧永远是"让位"。
#[cfg(target_os = "linux")]
fn same_program_name(pid: u32) -> bool {
    let ours = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .map(|name| name.chars().take(15).collect::<String>())
        .unwrap_or_else(|| "ice-translator".to_string());
    match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        Ok(comm) => comm.trim() == ours,
        Err(_) => true,
    }
}

#[cfg(not(target_os = "linux"))]
const fn holder_is_us(_pid: u32, _recorded: Option<u64>) -> bool {
    true
}

/// 进程启动时刻（自开机起的 clock ticks）。`/proc/<pid>/stat` 的第 22 字段；
/// `comm` 里可能有空格和括号，所以从**最后一个 `)`** 之后开始数。
#[cfg(target_os = "linux")]
fn start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.rsplit_once(')')?.1;
    // tail 从 state 字段（第 3 个）开始；第 22 个字段是它后面的第 20 项
    tail.split_whitespace().nth(19)?.parse().ok()
}

/// 锁文件里记的 PID；文件读不出来或内容不是数字时返回 `None`（按陈旧处理）。
fn holder_pid(path: &Path) -> Option<u32> {
    // 只认**第一行**：Linux 的锁还有第二行（启动时刻），整段 `parse` 会直接失败
    fs::read_to_string(path)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid != 0)
}

/// 锁文件第二行记的启动时刻；只有一行（旧格式/非 Linux）时为 `None`
fn holder_started(path: &Path) -> Option<u64> {
    let text = fs::read_to_string(path).ok()?;
    text.lines().nth(1)?.trim().parse().ok()
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
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap()
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
        // 与真实锁同形：第二行是那个进程自己的启动时刻（旧版只有一行的锁走的是另一条判据）
        let started = start_time(foreign).expect("活进程必须有 stat");
        fs::write(lock_path_in(dir.path()), format!("{foreign}\n{started}\n")).unwrap();

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

    /// 锁里那个 PID 是**活进程但从别的时代来的**（PID 被回收过）时按陈旧锁接管。
    ///
    /// 这条是 2026-11-25 真机上撞出来的：锁写着 5712，而 5712 已经是 GNOME 的
    /// `gsd-usb-protection`，新版本被"已在运行"挡在门外、桌面上却没有窗口。
    /// 造法：拿一个真活进程的 PID + 一个**对不上**的启动时刻。
    #[cfg(target_os = "linux")]
    #[test]
    fn a_lock_whose_pid_was_recycled_is_a_stale_lock() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("sleep 该起得来");
        let path = lock_path_in(dir.path());
        std::fs::write(&path, format!("{}\n{}\n", child.id(), 1)).unwrap();

        let acquired = acquire(dir.path());
        assert!(
            acquired.is_ok(),
            "PID 是别人的活进程，这明明是陈旧锁：{acquired:?}"
        );
        drop(acquired);
        let _ = child.kill();
        let _ = child.wait();
    }
}
