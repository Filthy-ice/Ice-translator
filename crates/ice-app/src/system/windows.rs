//! 认窗（CHG-010 REQ-108）：把"鼠标底下那扇窗"算出来。
//!
//! 这一格**不读屏幕内容、不做对比度分析**：系统本来就给着一张在线窗口表
//! （矩形 + 层级 + 归属 + 从前往后的顺序），本机实测能读到别家应用的窗口边界。
//! 所以这里只做纯挑选，读取那张表的那一步在 `windows_native`，两者分开是为了
//! 让"排除自己、排除桌面、取第一条包含该点的"这些判断能在无头里被断言
//! —— 真机跑一次只给一个样本，而这几条判断恰恰是最容易只对一半的。

use egui::{Pos2, Rect};

/// 窗口表里的一条。`rect` 是**全局点坐标**（左上原点，与 `CGWindowList` 一致）。
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub rect: Rect,
    /// 窗口层级：普通应用窗 0、贴图那一档 3、Dock 20、菜单栏 24、控制中心 25、
    /// 输入法候选窗 2147483629、桌面与壁纸是负数（全是本机真表读数）
    pub layer: i64,
    /// 归属进程名。用来排除**我们自己**的窗（不排就永远命中自家遮罩）
    pub owner: String,
}

/// 我们自己的归属名（`ice-translator` 与贴图/遮罩的标题都归它）
pub const SELF_OWNER: &str = "ice-translator";

/// 取鼠标那一点上**最靠前的那扇窗**的矩形；`origin` 是这扇遮罩所在屏的全局左上角，
/// 返回值换算回**该屏的本地坐标**（选区与裁剪都在本地坐标里算）。
///
/// 四条判据都在这一个函数里，别处不许再各判一半：
/// - **顺序就是叠放顺序**（表从前往后给），第一个命中的就是屏幕上真看见的那扇；
/// - **只认层级 0**（`NSNormalWindowLevel`，普通应用窗）。这一条是 2026-11-09 真表量出来的，
///   不是拍的：`程序坞` 在层级 **20** 上有一扇 **铺满整屏** 的窗（`0,0,1920x1080`），
///   而且排在所有应用窗**前面**；菜单栏是 `Window Server` 的层级 24，输入法候选窗在
///   2147483629。只要放行"层级 ≥ 0"，鼠标在主屏任何位置都会先命中那扇全屏的 Dock 窗
///   —— 表现就是"单击永远截整屏"。桌面与壁纸是负数，同样被这一条挡在外面；
/// - 自家窗要排掉：遮罩本身、还开着的贴图、主窗都在这张表里（贴图是层级 3，本就进不来，
///   但主窗与遮罩不能只靠层级挡）；
/// - 零面积排掉（真表里有两条层级 0 的 `0x0` 记账窗）。
///
/// 代价说清楚：浮动面板（层级 3/8 那一类）不被认。悬停没框时拖选照常，不是死路。
pub fn topmost_at(entries: &[Entry], origin: Pos2, point: Pos2, self_owner: &str) -> Option<Rect> {
    let global = point + origin.to_vec2();
    let hits = entries.iter().find(|entry| {
        entry.layer == 0
            && entry.owner != self_owner
            && entry.rect.width() > 0.0
            && entry.rect.height() > 0.0
            && entry.rect.contains(global)
    })?;
    // 全局 → 本地：一扇窗可能只有一半在这块屏上，夹取由调用方按那块屏的矩形做
    let shifted = Rect::from_min_max(
        hits.rect.min - origin.to_vec2(),
        hits.rect.max - origin.to_vec2(),
    );
    (shifted.width() > 0.0 && shifted.height() > 0.0).then_some(shifted)
}

/// 把候选框夹进这块屏（点坐标，本地）。副屏在负方向，所以夹取的两端都得用调用给的矩形，
/// 不许写死 `0..尺寸`（那是"副屏永远看不见"的写法）。
pub fn clamp_to_screen(rect: Rect, screen: Rect) -> Rect {
    let x = |v: f32| v.clamp(screen.min.x, screen.max.x);
    let y = |v: f32| v.clamp(screen.min.y, screen.max.y);
    let (a, b, c, d) = (x(rect.min.x), y(rect.min.y), x(rect.max.x), y(rect.max.y));
    Rect::from_min_max(Pos2::new(a.min(c), b.min(d)), Pos2::new(a.max(c), b.max(d)))
}

/// 一次截图的认窗快照：在线表 + 我们自己在表里报的名字。
///
/// **一次截图取一次，之后整段遮罩期间不再问系统**。理由是画面已经冻结：冻结之后才冒出来
/// 或挪了位置的窗，在用户看到的这张图里根本不存在，认它就是认错。顺带也把成本钉死在
/// 一次同步问答上（本仓刚为"每帧重下原生命令"付过代价，D-31~D-34）。
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub self_owner: String,
}

impl Snapshot {
    /// 真去问系统那一处。非 macOS 上 `read` 恒空，于是这一档自动缺席、拖选照常。
    pub fn take() -> Self {
        Self {
            entries: read(),
            self_owner: self_owner(),
        }
    }

    /// 鼠标底下那扇窗（**该屏本地坐标**，已夹进这块屏）。`origin` 是这块屏的全局左上角。
    pub fn candidate(&self, origin: Pos2, point: Pos2, screen: Rect) -> Option<Rect> {
        let rect = topmost_at(&self.entries, origin, point, &self.self_owner)?;
        Some(clamp_to_screen(rect, screen))
    }
}

/// 读系统的在线窗口表（`CGWindowListCopyWindowInfo`），顺序就是**从前往后**的叠放顺序。
///
/// **一次截图只读一次**（遮罩开的那一帧），不是每帧：这是一次到窗口服务器的同步问答，
/// 而画面已经冻结 —— 每帧重读既贵，又会认出"冻结之后才冒出来的窗"，那扇窗在用户看到的
/// 这张图里根本不存在。
///
/// 走裸 C 接口而不是 `core-graphics` + `core-foundation` 那两个 crate：2026-11-09 试过
/// typed 包装，这张表交回来的是**无类型**的 `CFArray<*const c_void>`，元素要按字典再取
/// 嵌套的 bounds，而 core-foundation 0.10 的 `ItemRef` 连 `get` 都要自己按规则包装，
/// 当场撞了三类签名问题。这里只声明用到的十几个符号，链接的两个框架本来就由 AppKit
/// 带进进程，**一个 crate 名都不新增**（判据仍看 `cargo tree`）。
#[cfg(target_os = "macos")]
pub fn read() -> Vec<Entry> {
    native::read()
}

/// 非 macOS 没有这张表：X11 那边要自己遍历 `_NET_CLIENT_LIST_STACKING` 并逐个问几何，
/// Wayland 连全局窗口列表都不给客户端。恒空 = 认窗这一档自动缺席，拖选照常（NFR-7 的分档）。
/// Linux 档（T-013）：X11 给得出叠放顺序与几何，就照 `_NET_CLIENT_LIST_STACKING` 认；
/// Wayland 原生档没有这张表（连全局窗口列表都不给客户端），恒空、认窗自动缺席。
#[cfg(target_os = "linux")]
pub fn read() -> Vec<Entry> {
    crate::system::window_x11::stacked_entries()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn read() -> Vec<Entry> {
    Vec::new()
}

/// 我们自己在窗口表里的归属名。**按自己的 pid 去表里认**，不写死字符串：
/// 打包成 `.app` 之后系统报的名字可能不再是 `ice-translator`，写死就排不掉自家遮罩，
/// 而遮罩铺满整屏 —— 排不掉的后果正是"单击永远截整屏"。认不出来才退回 [`SELF_OWNER`]。
///
/// **只缓存认出来的那一次**：主窗收着的时候在线表里没有我们，那一回只能退回常量；
/// 把退回值也缓存住，就等于第一次开遮罩时主窗恰好藏着 → 此后整个进程都认不出自家窗。
#[cfg(target_os = "macos")]
pub fn self_owner() -> String {
    use std::sync::OnceLock;
    static OWNER: OnceLock<String> = OnceLock::new();
    if let Some(known) = OWNER.get() {
        return known.clone();
    }
    match native::self_owner() {
        Some(name) => {
            let _ = OWNER.set(name.clone());
            name
        }
        None => SELF_OWNER.to_string(),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn self_owner() -> String {
    SELF_OWNER.to_string()
}

#[cfg(target_os = "macos")]
mod native {
    use super::Entry;
    use egui::{Pos2, Rect, Vec2};
    use std::ffi::{CStr, c_char, c_void};

    /// CF 的对象引用一律是无类型指针；类型靠 `CFGetTypeID` 现问，不靠猜。
    type Ref = *const c_void;

    const UTF8: u32 = 0x0800_0100;
    /// `kCGWindowListOptionOnScreenOnly`。用 `optionAll` 会把每家应用那条离屏菜单栏
    /// （`0,0,1920x30`）和几百条记账窗一起给回来 —— 本机实测 200+ 条里只有 45 条在线。
    const ON_SCREEN_ONLY: u32 = 1;
    const NULL_WINDOW: u32 = 0;
    /// `kCFNumberFloat64Type`：系统存的是整数还是 double 都按 f64 取出来，一次问清。
    const FLOAT64: i32 = 6;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFDictionaryGetValue(dict: Ref, key: Ref) -> Ref;
        fn CFStringCreateWithCString(alloc: Ref, text: *const c_char, encoding: u32) -> Ref;
        fn CFStringGetCStringPtr(text: Ref, encoding: u32) -> *const c_char;
        fn CFStringGetCString(text: Ref, buf: *mut c_char, len: isize, encoding: u32) -> u8;
        fn CFNumberGetValue(number: Ref, kind: i32, out: *mut c_void) -> u8;
        fn CFGetTypeID(value: Ref) -> std::os::raw::c_ulong;
        fn CFDictionaryGetTypeID() -> std::os::raw::c_ulong;
        fn CFStringGetTypeID() -> std::os::raw::c_ulong;
        fn CFNumberGetTypeID() -> std::os::raw::c_ulong;
        fn CFRelease(value: Ref);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(options: u32, relative_to: u32) -> Ref;
    }

    /// 七个键只造一次、用完释放：每条窗口都现造一遍就是几百次分配。
    struct Keys {
        bounds: Ref,
        layer: Ref,
        owner: Ref,
        pid: Ref,
        x: Ref,
        y: Ref,
        w: Ref,
        h: Ref,
    }

    impl Keys {
        fn new() -> Option<Self> {
            let names = [
                c"kCGWindowBounds",
                c"kCGWindowLayer",
                c"kCGWindowOwnerName",
                c"kCGWindowOwnerPID",
                c"X",
                c"Y",
                c"Width",
                c"Height",
            ];
            let mut made: [Ref; 8] = [std::ptr::null(); 8];
            for (slot, name) in made.iter_mut().zip(names) {
                // SAFETY: 键是常量 C 字符串；造出来的 CFString 由本结构负责释放一次，
                // 中途失败就把已经造出来的那几个还回去，不留泄漏。
                let value =
                    unsafe { CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), UTF8) };
                if value.is_null() {
                    for earlier in made.iter().filter(|r| !r.is_null()) {
                        unsafe { CFRelease(*earlier) };
                    }
                    return None;
                }
                *slot = value;
            }
            Some(Self {
                bounds: made[0],
                layer: made[1],
                owner: made[2],
                pid: made[3],
                x: made[4],
                y: made[5],
                w: made[6],
                h: made[7],
            })
        }
    }

    impl Drop for Keys {
        fn drop(&mut self) {
            for key in [
                self.bounds,
                self.layer,
                self.owner,
                self.pid,
                self.x,
                self.y,
                self.w,
                self.h,
            ] {
                if !key.is_null() {
                    // SAFETY: 每个键都是 `Keys::new` 造出来且只释放这一次。
                    unsafe { CFRelease(key) };
                }
            }
        }
    }

    pub(super) fn read() -> Vec<Entry> {
        let Some(keys) = Keys::new() else {
            return Vec::new();
        };
        // SAFETY: 只碰这一次调用拿回来的那张表。表由我们释放一次；元素指针只在表活着
        // 的时候用，不存出去，所以不存在悬垂。
        unsafe {
            let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, NULL_WINDOW);
            if list.is_null() {
                return Vec::new();
            }
            let mut out = Vec::new();
            for index in 0..CFArrayGetCount(list) {
                let item = CFArrayGetValueAtIndex(list, index);
                if !is_dict(item) {
                    continue;
                }
                if let Some(entry) = entry_of(item, &keys) {
                    out.push(entry);
                }
            }
            CFRelease(list);
            out
        }
    }

    /// 表里属于本进程的那一行报的归属名。找不到（自家窗全隐藏着）就返回 `None`。
    pub(super) fn self_owner() -> Option<String> {
        let keys = Keys::new()?;
        let mine = std::process::id() as i32;
        // SAFETY: 同 `read`：表与元素都在这一次调用内用完即释放。
        unsafe {
            let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, NULL_WINDOW);
            if list.is_null() {
                return None;
            }
            let mut found = None;
            for index in 0..CFArrayGetCount(list) {
                let item = CFArrayGetValueAtIndex(list, index);
                if !is_dict(item) {
                    continue;
                }
                let pid = number(CFDictionaryGetValue(item, keys.pid)).unwrap_or(-1.0);
                if pid as i32 == mine {
                    found = text(CFDictionaryGetValue(item, keys.owner));
                    break;
                }
            }
            CFRelease(list);
            found
        }
    }

    /// 一条窗口 → `Entry`。缺层级、缺归属、缺 bounds 任一样就整条丢掉：
    /// 半个矩形比没有矩形更坏（会认出一扇量不出大小的窗）。
    unsafe fn entry_of(dict: Ref, keys: &Keys) -> Option<Entry> {
        unsafe {
            let layer = number(CFDictionaryGetValue(dict, keys.layer))? as i64;
            let owner = text(CFDictionaryGetValue(dict, keys.owner))?;
            let bounds = CFDictionaryGetValue(dict, keys.bounds);
            if !is_dict(bounds) {
                return None;
            }
            let x = number(CFDictionaryGetValue(bounds, keys.x))?;
            let y = number(CFDictionaryGetValue(bounds, keys.y))?;
            let w = number(CFDictionaryGetValue(bounds, keys.w))?;
            let h = number(CFDictionaryGetValue(bounds, keys.h))?;
            Some(Entry {
                // 表给的就是全局左上原点、y 向下，与 egui/winit 同一坐标系，不再翻转
                rect: Rect::from_min_size(
                    Pos2::new(x as f32, y as f32),
                    Vec2::new(w as f32, h as f32),
                ),
                layer,
                owner,
            })
        }
    }

    unsafe fn is_dict(value: Ref) -> bool {
        unsafe { !value.is_null() && CFGetTypeID(value) == CFDictionaryGetTypeID() }
    }

    unsafe fn number(value: Ref) -> Option<f64> {
        unsafe {
            if value.is_null() || CFGetTypeID(value) != CFNumberGetTypeID() {
                return None;
            }
            let mut out = 0.0f64;
            let ok = CFNumberGetValue(value, FLOAT64, (&mut out as *mut f64).cast::<c_void>());
            (ok != 0).then_some(out)
        }
    }

    unsafe fn text(value: Ref) -> Option<String> {
        unsafe {
            if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
                return None;
            }
            let direct = CFStringGetCStringPtr(value, UTF8);
            if !direct.is_null() {
                return Some(CStr::from_ptr(direct).to_string_lossy().into_owned());
            }
            // 内部不是 UTF-8 排布时上面那条返回 null，得让它自己转一份出来
            let mut buf: [c_char; 512] = [0; 512];
            let ok = CFStringGetCString(value, buf.as_mut_ptr(), buf.len() as isize, UTF8);
            (ok != 0).then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    fn entry(layer: i64, owner: &str, r: Rect) -> Entry {
        Entry {
            rect: r,
            layer,
            owner: owner.into(),
        }
    }

    /// 取的是**第一条**命中的：表从前往后给，第一条就是看得见那扇
    #[test]
    fn the_first_entry_containing_the_point_wins() {
        let entries = vec![
            entry(0, "Safari", rect(0.0, 0.0, 900.0, 700.0)),
            entry(0, "Finder", rect(0.0, 0.0, 1920.0, 1080.0)),
        ];
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(100.0, 100.0), SELF_OWNER),
            Some(rect(0.0, 0.0, 900.0, 700.0))
        );
        // 那一点只在后面那扇里：拿后面的
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(1200.0, 900.0), SELF_OWNER),
            Some(rect(0.0, 0.0, 1920.0, 1080.0))
        );
    }

    /// 自家那几扇（遮罩、贴图、主窗）**必须排掉**：不排的话鼠标永远命中遮罩自己，
    /// 而遮罩铺满整屏 —— 结果就是"单击永远截整屏"。
    #[test]
    fn our_own_windows_are_never_the_candidate() {
        let entries = vec![
            entry(25, SELF_OWNER, rect(0.0, 0.0, 1920.0, 1080.0)),
            entry(3, SELF_OWNER, rect(40.0, 40.0, 400.0, 300.0)),
            entry(0, "Xcode", rect(0.0, 0.0, 1200.0, 900.0)),
        ];
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(60.0, 60.0), SELF_OWNER),
            Some(rect(0.0, 0.0, 1200.0, 900.0))
        );
        assert_eq!(
            topmost_at(&entries[..2], Pos2::ZERO, pos2(60.0, 60.0), SELF_OWNER),
            None,
            "只剩自家窗时宁可不认，也不许把自家遮罩当目标"
        );
    }

    /// 桌面与壁纸是负层级且铺满整块屏；0x0 的记账窗也不能算。
    #[test]
    fn the_desktop_and_zero_sized_entries_are_skipped() {
        let entries = vec![
            entry(
                -2_147_483_623,
                "Dynamic Wallpaper",
                rect(0.0, 0.0, 1920.0, 1080.0),
            ),
            entry(0, "SomeAgent", rect(500.0, 500.0, 500.0, 560.0)),
        ];
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(600.0, 600.0), SELF_OWNER),
            None
        );
    }

    /// 系统那几条**铺满整屏或压在同一块地方**的窗不许当候选。这一组数字全是 2026-11-09
    /// 真表读数（`the_live_table_gives_bounds_layer_and_owner` 打出来的那 53 条）：
    /// Dock 在层级 20 上有一扇整屏窗且排在应用窗前面，菜单栏是 Window Server 的 24，
    /// 输入法候选窗在 2147483629。放行任何一条，"单击截整窗"就变成"单击截整屏"。
    #[test]
    fn the_system_chrome_windows_ahead_of_app_windows_are_not_candidates() {
        let entries = vec![
            entry(
                2_147_483_630,
                "Window Server",
                rect(-174.0, 1.0, -146.0, 29.0),
            ),
            entry(
                2_147_483_629,
                "搜狗输入法",
                rect(-206.0, 1126.0, 0.0, 1196.0),
            ),
            entry(25, "控制中心", rect(1299.0, 0.0, 1337.0, 30.0)),
            entry(24, "Window Server", rect(0.0, 0.0, 1920.0, 30.0)),
            entry(20, "程序坞", rect(0.0, 0.0, 1920.0, 1080.0)),
            entry(0, "IntelliJ IDEA", rect(0.0, 31.0, 1920.0, 1016.0)),
        ];
        // 屏幕正中：前面那扇整屏的 Dock 窗必须让位给真正的应用窗
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(900.0, 500.0), SELF_OWNER),
            Some(rect(0.0, 31.0, 1920.0, 1016.0))
        );
        // 菜单栏那一条：宁可不认，也不许截下整条系统菜单栏
        assert_eq!(
            topmost_at(&entries, Pos2::ZERO, pos2(900.0, 10.0), SELF_OWNER),
            None
        );
        // 只剩系统窗时同样不认
        assert_eq!(
            topmost_at(&entries[..5], Pos2::ZERO, pos2(900.0, 500.0), SELF_OWNER),
            None
        );
    }

    /// 副屏在**负方向**（本机实测左置那块在 `(-1920,-120)`）：候选框要换算成那块屏的
    /// 本地坐标，负偏移不许被当成"没有命中"。
    #[test]
    fn a_secondary_screen_at_negative_coordinates_is_converted_back_to_local() {
        let origin = pos2(-1920.0, -120.0);
        let entries = vec![entry(0, "IDEA", rect(-1900.0, -100.0, -1200.0, 500.0))];
        assert_eq!(
            topmost_at(&entries, origin, pos2(20.0, 20.0), SELF_OWNER),
            Some(rect(20.0, 20.0, 720.0, 620.0))
        );
    }

    /// 夹取两端都用调用给的屏矩形：写死 `0..尺寸` 在副屏上会把框整个推没。
    #[test]
    fn clamping_uses_the_given_screen_not_a_zero_origin_assumption() {
        let screen = rect(-1920.0, -120.0, 0.0, 1080.0);
        assert_eq!(
            clamp_to_screen(rect(-2100.0, -400.0, 900.0, 4000.0), screen),
            rect(-1920.0, -120.0, 0.0, 1080.0)
        );
        assert_eq!(
            clamp_to_screen(rect(-1500.0, 0.0, -1000.0, 300.0), screen).min,
            pos2(-1500.0, 0.0)
        );
    }

    /// 真表核对（要真窗口服务器）：
    /// `cargo test -p ice-app system::windows -- --ignored --nocapture`
    ///
    /// 判据不是"没崩"，是**三样读数都取到了**：在线表非空、里头有层级 0 的正面积窗、
    /// 归属名不是空串。任缺一样都说明 FFI 取错了键或类型（那正是上一版放弃的地方）。
    #[test]
    #[ignore = "要真窗口服务器，读数与仓外 Swift 探针核对"]
    fn the_live_table_gives_bounds_layer_and_owner() {
        let entries = read();
        for entry in &entries {
            println!(
                "WIN|layer={}|owner={}|rect={:?}",
                entry.layer, entry.owner, entry.rect
            );
        }
        println!(
            "共 {} 条在线窗；自家归属名认作 {}",
            entries.len(),
            self_owner()
        );
        assert!(!entries.is_empty(), "在线表不该是空的（桌面上总有窗）");
        assert!(
            entries.iter().any(|e| e.layer == 0 && e.rect.area() > 0.0),
            "该有一条层级 0 的正面积普通窗"
        );
        assert!(
            entries.iter().all(|e| !e.owner.is_empty()),
            "归属名取不到就是键或类型取错了"
        );
    }
}
