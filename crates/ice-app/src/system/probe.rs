//! N0 只读诊断：把四条用户报告要的读数打进日志，**一个字节的行为都不改**。
//!
//! 四条报告与各自的读数：
//!
//! 1. "截图时我们的窗口失焦透明、透出下面的字" → 要的是这一刻的 `mode + focused + capturing
//!    + 判出来的 want + 原生有没有接住`。`mode::dim_alpha` 里只有"精简 + 失焦 + 开关没关"会
//!      低于 1.0，这条读数要么坐实它，要么把它排除。
//! 2. "全量转精简后缩不下去" → 要的是**原生读回来的内容区下限**与该形态**应该**下发的下限。
//!    两个数不相等就是 `MinInnerSize` 没落到位（与 `mode.rs` 记过的 W-2.1 同形状：全量的
//!    640×420 漏到精简窗上）；相等说明是数值本身，那是改需求不是修缺陷，得用户给数。
//! 3. "精简窗隐不进左右边缘" → 停靠那条链上每一步都是布尔量（停靠边、是否 docked、锚点在不在、
//!    当前是收回还是唤出、闸门、静止判定有没有在跑），任何一个卡住后面都整段不动。
//! 4. "按 F1 有时先纯白 1-3 秒" → 要的是同一条时间轴上的几个毫秒点：热键派发 → 每块屏抓屏 →
//!    整轮冻结完成 → 遮罩第一帧。差值落在哪一段，白就在哪一段。
//!
//! 纪律：这一模块**只读不写**。不发 `ViewportCommand`、不改状态、不碰盘、不新增任何一条
//! 意图出口或唤醒点（`selection` 那几处 push/wake 的形状计数因此不变）。
//! 挂在 `debug` 级：默认 `info` 不输出，排查时 `RUST_LOG=ice_translator=debug` 打开；
//! 截图派发与遮罩首帧那两条留在 `info`，因为一次截图只有两行，不刷屏。

/// 这一刻窗口侧的读数。字段全是"帧事实"，没有一样是我们自己造的中间状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowProbe {
    pub mode: ice_core::config::UiMode,
    pub focused: bool,
    pub capturing: bool,
    /// `mode::dim_alpha` 这一帧判出来的整窗不透明度
    pub want: f32,
    /// 原生 `setAlphaValue` 这一档是不是真的落上了（沿用 `apply_native_alpha` 的缓存读数）
    pub alpha_native: bool,
    /// 原生读回来的内容区最小尺寸；`None` = 拿不到句柄 / 不在主线程 / 非 mac
    pub min_native: Option<[f32; 2]>,
    /// 该形态**应该**下发的下限（`mode::min_size_for`，数值出处是 `ice-core::config`）
    pub min_wanted: [f32; 2],
}

/// 窗口侧读数变了才记一行（形态、焦点、截图态、alpha、下限五项任一变化）。
/// 每帧无脑记会把 `/tmp/ice.log` 刷满，与 `apply_native_alpha`、`system::screen` 同一手法。
pub fn note_window(prev: &mut Option<WindowProbe>, now: WindowProbe) {
    if prev.replace(now) == Some(now) {
        return;
    }
    tracing::debug!(
        mode = ?now.mode,
        focused = now.focused,
        capturing = now.capturing,
        want = now.want,
        alpha_native = now.alpha_native,
        min_native = ?now.min_native,
        min_wanted = ?now.min_wanted,
        "窗口探针"
    );
}

/// 这一刻停靠链上的六个布尔量与那条边。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockProbe {
    pub edge: ice_core::config::DockEdge,
    /// `mode::is_docked(edge, mode)` —— 形态与停靠边要同时成立，这一条假后面全不动
    pub docked: bool,
    /// `is_docked` 成立却还没有锚点，走的是 `land_dock` 那一支
    pub needs_land: bool,
    pub shown: bool,
    pub latched: bool,
    /// 原生拖拽的静止判定还在等（等的时候这一帧什么都不判）
    pub armed: bool,
    pub hidden: bool,
}

/// 停靠链读数变了才记一行。
pub fn note_dock(prev: &mut Option<DockProbe>, now: DockProbe) {
    if prev.replace(now) == Some(now) {
        return;
    }
    tracing::debug!(
        edge = ?now.edge,
        docked = now.docked,
        needs_land = now.needs_land,
        shown = now.shown,
        latched = now.latched,
        armed = now.armed,
        hidden = now.hidden,
        "停靠探针"
    );
}

/// 主线程这一帧离上一帧超过这个数就算停顿。抓屏整轮实测只有 256–308ms，用户报的 1–3 秒
/// 白要么落在"我们的帧根本没跑"，要么落在"帧在跑但遮罩还没画上"——这两个数能把它分开。
pub const STALL_MS: u64 = 150;

/// 从派发那一刻起，这段时间内才算"截图段"。平时应用按需重画，空闲几秒不跑一帧是正常的，
/// 只有截图那一段里的停顿才是线索。
pub const SPAN_MS: u64 = 6_000;

/// 派发之后还在截图段里吗。
pub fn in_span(until: Option<u64>, now: u64) -> bool {
    until.is_some_and(|until| now <= until)
}

/// 帧间隔：缓存每帧都往前挪，**只有段内且超过 `STALL_MS` 才记一行**。
pub fn note_frame_gap(prev: &mut Option<u64>, now: u64, in_span: bool) {
    let gap = prev.map_or(0, |at| now.saturating_sub(at));
    *prev = Some(now);
    if in_span && gap >= STALL_MS {
        tracing::debug!(gap_ms = gap, t_ms = now, "截图段内的帧间隔");
    }
}

/// 同一条时间轴上的毫秒点，用来把四段串起来对差值。
/// 用挂钟而不是 `Instant`：日志要跨线程（抓屏在 blocking 线程）也能并到一条轴上。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// 读原生那扇窗的内容区下限。
///
/// 这是"用户鼠标拉不动"唯一的现场凭证：egui 侧只能报出我们**请求**过的尺寸，
/// 真正把拖动夹住的是 AppKit 的 `contentMinSize`。只读，不改。
#[cfg(target_os = "macos")]
pub fn content_min_size(frame: &eframe::Frame) -> Option<[f32; 2]> {
    let ns_window = super::window::ns_window(frame)?;
    let size = ns_window.contentMinSize();
    Some([size.width as f32, size.height as f32])
}

/// 非 mac 没有这个读数（那一侧的最小尺寸由窗口管理器决定），恒 `None`。
#[cfg(not(target_os = "macos"))]
pub fn content_min_size(_frame: &eframe::Frame) -> Option<[f32; 2]> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_core::config::{DockEdge, UiMode};

    fn window(mode: UiMode, want: f32) -> WindowProbe {
        WindowProbe {
            mode,
            focused: true,
            capturing: false,
            want,
            alpha_native: true,
            min_native: Some([400.0, 160.0]),
            min_wanted: [400.0, 160.0],
        }
    }

    fn dock(edge: DockEdge, shown: bool) -> DockProbe {
        DockProbe {
            edge,
            docked: edge != DockEdge::None,
            needs_land: false,
            shown,
            latched: false,
            armed: false,
            hidden: false,
        }
    }

    /// 探针只认"变了才记"：同一组读数连喂三帧，缓存里只有一份，且一帧都不写。
    #[test]
    fn the_same_readings_are_recorded_once() {
        let mut prev = None;
        note_window(&mut prev, window(UiMode::Compact, 1.0));
        note_window(&mut prev, window(UiMode::Compact, 1.0));
        assert_eq!(prev, Some(window(UiMode::Compact, 1.0)));
    }

    /// 形态与 alpha 都算"变了"：截图时那一路要的就是这两项能各自翻身被记下来。
    #[test]
    fn a_changed_mode_or_alpha_replaces_the_record() {
        let mut prev = None;
        note_window(&mut prev, window(UiMode::Compact, 1.0));
        note_window(&mut prev, window(UiMode::Compact, 0.75));
        note_window(&mut prev, window(UiMode::Full, 0.75));
        assert_eq!(prev, Some(window(UiMode::Full, 0.75)));
    }

    /// 读不到原生下限（非 mac / 没句柄）时不许假装读到了：那一项必须是 `None`，
    /// 日志里它就是 `None`，看的人才知道这一档没有现场数据。
    #[test]
    fn an_unreadable_native_minimum_stays_none() {
        let mut prev = None;
        let mut probe = window(UiMode::Compact, 1.0);
        probe.min_native = None;
        note_window(&mut prev, probe);
        assert_eq!(prev.unwrap().min_native, None);
    }

    /// 停靠链：只有那条边变了才重新记，同一档反复喂不动缓存。
    #[test]
    fn the_dock_chain_records_the_edge_and_the_gates() {
        let mut prev = None;
        note_dock(&mut prev, dock(DockEdge::Right, false));
        note_dock(&mut prev, dock(DockEdge::Right, false));
        assert_eq!(prev, Some(dock(DockEdge::Right, false)));
        note_dock(&mut prev, dock(DockEdge::Right, true));
        assert!(prev.unwrap().shown);
    }

    /// 时间戳单调：四段要按它排序，倒着走的数会把结论带到别处去。
    #[test]
    fn the_timeline_moves_forward() {
        let first = now_ms();
        std::thread::sleep(std::time::Duration::from_millis(3));
        assert!(now_ms() >= first);
    }

    /// 截图段的口径：派发后 6 秒内算段内，过了或压根没派发过都不算。
    #[test]
    fn the_capture_span_has_a_start_and_an_end() {
        assert!(in_span(Some(11_000), 5_000));
        assert!(!in_span(Some(4_000), 5_000));
        assert!(!in_span(None, 5_000));
    }

    /// 空闲时的长停顿不许当线索：缓存照样往前挪，段外一次都不记，
    /// 下一次进段时量到的是"离上一帧"的间隔，不是"离昨天那一帧"。
    #[test]
    fn the_gap_cache_moves_even_outside_the_span() {
        let mut prev = None;
        note_frame_gap(&mut prev, 1_000, false);
        note_frame_gap(&mut prev, 9_000, false);
        assert_eq!(prev, Some(9_000));
        note_frame_gap(&mut prev, 9_200, true);
        assert_eq!(prev, Some(9_200));
    }
}
