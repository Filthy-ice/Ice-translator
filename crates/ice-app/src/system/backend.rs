//! 后端档位（CHG-017 REQ-001）。
//!
//! Linux 上同一份二进制可能坐在三种环境里：X11（含 XWayland）、Wayland 原生、或者两者都
//! 连不上（headless 测试）。`capture::capability()` 那四格与置顶/热键的成败都只取决于
//! "此刻在哪一档"，所以这一格判定 SHALL 有唯一出处——散写 `env::var("DISPLAY")` 就等于有
//! 两个地方各自决定这台机器能做什么（`capture.rs:50` 那句"视图里出现 `#[cfg(target_os)]`
//! 就是缺陷"是同一条纪律）。
//!
//! ## 为什么判一次就不许翻
//!
//! 档位一变，界面上那颗按钮的灰/亮就会在一次点击之间自己变。要做到"运行中换后端"得先有
//! 重建窗口的能力，本包没有——所以 `arm()` 启动时写一次，之后只读。唯一的第二个写入口是
//! 测试用的 `set_for_test`（原子字节而不是 `OnceLock` 就是为了这个）。
//!
//! ## mac 与 Windows 走编译期常量，不是探测
//!
//! 那两个平台 SHALL NOT 去连 X、也 SHALL NOT 看环境变量：`aa6c050` 上 `capability()` 的
//! 四格就是 `cfg!` 常量，本包不得把它变成探测（REQ-012 的平台边界）。探测函数按
//! `global_hotkeys.rs` 里 `install_handler` 那套双分支写法给，免得非 Linux 上留下一堆
//! 没人调的函数——`clippy -D warnings` 会当场拦下来。

use std::sync::atomic::{AtomicU8, Ordering};

/// 一档后端。变体名刻意不叫 `None`——它挨着 `Option::None` 会读成两义
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// macOS：AppKit/Carbon 那一套，能力由编译期常量决定
    Mac,
    /// Windows：`RegisterHotKey` 那一套
    Windows,
    /// 连得上的 X11（含 XWayland）：置顶、任意移窗、`XGrabKey` 在这一档才成立
    X11,
    /// 只有 Wayland 合成器：窗口层级归合成器，本包在这一档不承诺置顶与全局热键
    WaylandNative,
    /// 两者都连不上（headless，或 XWayland 没装）
    NoBackend,
}

const CODE_UNARMED: u8 = 0;
const CODE_MAC: u8 = 1;
const CODE_WINDOWS: u8 = 2;
const CODE_X11: u8 = 3;
const CODE_WAYLAND: u8 = 4;
const CODE_NO_BACKEND: u8 = 5;

/// 本进程唯一一份档位
static TIER: AtomicU8 = AtomicU8::new(CODE_UNARMED);

fn code_of(tier: Tier) -> u8 {
    match tier {
        Tier::Mac => CODE_MAC,
        Tier::Windows => CODE_WINDOWS,
        Tier::X11 => CODE_X11,
        Tier::WaylandNative => CODE_WAYLAND,
        Tier::NoBackend => CODE_NO_BACKEND,
    }
}

/// 认不出来的码只可能是内存被写坏，那种情况下**宁可报"没有后端"**也不能报成 mac——
/// 报成 mac 会让 Linux 去走一条 Carbon 都不存在的capability 声明
fn tier_of(code: u8) -> Tier {
    match code {
        CODE_MAC => Tier::Mac,
        CODE_WINDOWS => Tier::Windows,
        CODE_X11 => Tier::X11,
        CODE_WAYLAND => Tier::WaylandNative,
        _ => Tier::NoBackend,
    }
}

impl Tier {
    /// 设置页那一行（REQ-011）。话术要求同 `HotkeyFailure::explain()`：说得出这一档的
    /// 后果，而不是只报个名词；"没有后端"那句里不许出现"可用"
    pub const fn label(self) -> &'static str {
        match self {
            Tier::Mac => "macOS",
            Tier::Windows => "Windows",
            Tier::X11 => "X11：置顶、贴边收起与全局快捷键可用",
            Tier::WaylandNative => "Wayland 原生：窗口位置由桌面决定（全局快捷键走门户授权）",
            Tier::NoBackend => "没有连上图形后端",
        }
    }
}

/// 这一档该是什么。**纯探测、不写状态**，所以 `arm()` 与测试共用同一条路径
pub fn detect() -> Tier {
    if cfg!(target_os = "macos") {
        Tier::Mac
    } else if cfg!(target_os = "windows") {
        Tier::Windows
    } else {
        probe_backend()
    }
}

/// X11 这一格不是"环境变量里有 `DISPLAY`"就完事：XWayland 没起、或 `DISPLAY` 是从别处
/// 继承来的死值时，变量在而连接建不起来——那时置顶与热键全都做不到，报 `X11` 就是撒谎
/// （`global_hotkeys.rs` 文件头那条"设置页显示注册成功、按下去没反应"的同族症状）
#[cfg(target_os = "linux")]
fn probe_backend() -> Tier {
    let display = std::env::var_os("DISPLAY").filter(|v| !v.is_empty());
    if display.is_some() && x11_reachable() {
        return Tier::X11;
    }
    if std::env::var_os("WAYLAND_DISPLAY")
        .filter(|v| !v.is_empty())
        .is_some()
    {
        return Tier::WaylandNative;
    }
    Tier::NoBackend
}

#[cfg(not(target_os = "linux"))]
fn probe_backend() -> Tier {
    Tier::NoBackend
}

/// 真去建一条 X 连接再丢掉。花的是启动时的几毫秒，换的是"这一档报得诚实"。
///
/// 连接不留在手里：XCB 连接不是 `Send`，窗口操作各自开自己的连接（贴边与热键线程各一条）
#[cfg(target_os = "linux")]
fn x11_reachable() -> bool {
    // 只探活：连接出了作用域就断，不留进结构体（`RustConnection` 不是 `Send`，
    // 窗口操作与热键各自开自己的连接）
    x11rb::connect(None).is_ok()
}

/// 启动时调一次。已经定过档就不重算——翻转档位会让按钮在一次点击之间变灰
pub fn arm() {
    let _ = TIER.compare_exchange(
        CODE_UNARMED,
        code_of(detect()),
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

/// 当前档位。没 `arm()` 过时现算一次（探针与测试不该被迫记得先装配）
pub fn current() -> Tier {
    let code = TIER.load(Ordering::SeqCst);
    if code == CODE_UNARMED {
        detect()
    } else {
        tier_of(code)
    }
}

/// 只给测试摆档位用；生产没有第二个写入口（见模块头"判一次就不许翻"）
#[cfg(test)]
pub fn set_for_test(tier: Tier) {
    TIER.store(code_of(tier), Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 五档互不相同且能往返。并码的代价是"设置页显示的档位与实际能力不是一回事"
    #[test]
    fn each_tier_round_trips_through_its_own_code() {
        for tier in [
            Tier::Mac,
            Tier::Windows,
            Tier::X11,
            Tier::WaylandNative,
            Tier::NoBackend,
        ] {
            assert_eq!(tier_of(code_of(tier)), tier, "{tier:?} 的码被并了");
        }
    }

    /// 认不出的码落到"没有后端"，绝不落到 mac（那等于让 Linux 声明一套 Carbon 能力）
    #[test]
    fn an_unknown_code_falls_back_to_no_backend_never_to_mac() {
        assert_eq!(tier_of(CODE_UNARMED), Tier::NoBackend);
        assert_eq!(tier_of(201), Tier::NoBackend);
        assert_ne!(tier_of(201), Tier::Mac);
    }

    /// 设置页那一行说得出后果；"没有后端"不许写成成功
    #[test]
    fn the_settings_line_names_the_consequence_not_just_the_backend() {
        assert!(Tier::X11.label().contains("全局快捷键"));
        assert!(Tier::WaylandNative.label().contains("由桌面决定"));
        assert!(!Tier::NoBackend.label().contains("可用"));
    }

    /// 本机（非 mac/Win）上 detect 不许凭空报出那两个档
    #[test]
    fn detect_never_invents_a_foreign_tier() {
        let tier = detect();
        if !cfg!(target_os = "macos") {
            assert_ne!(tier, Tier::Mac);
        }
        if !cfg!(target_os = "windows") {
            assert_ne!(tier, Tier::Windows);
        }
    }

    /// `arm()` 只在没定过档时写——这条就是"判一次就不许翻"
    #[test]
    fn arm_does_not_overwrite_an_already_latched_tier() {
        set_for_test(Tier::WaylandNative);
        arm();
        assert_eq!(
            current(),
            Tier::WaylandNative,
            "已定档之后 arm() 把后端翻了，按钮会在一次点击之间变灰"
        );
        set_for_test(Tier::X11);
    }
}
