//! 全局快捷键（FR-2.10、CHG-009 REQ-006）。
//!
//! 与 `system::hotkeys` 的分工是"整台机器的按键"与"窗口有焦点时的按键"，两份**共用同一个
//! [`Action`] 枚举**：热键命中后交出去的就是那条既有分派，不开第二条执行路径（图钉那次
//! 两条通道各走各的，坏在细节里）。
//!
//! ## 四条写法是 2026-11-06 上午用真机按键量出来的，不是从文档抄的（事实 35~38）
//!
//! 1. **事件类型四个值 SHALL 按 SDK 抄**：类 `'keyb'`（`CarbonEvents.h:1339`）、种
//!    `kEventHotKeyPressed = 5`（`:4693`，**整数**不是四字符码）、参数名 `'----'`
//!    （`CarbonEventsCore.h:178`）、参数型 `'hkid'`（`CarbonEvents.h:376`）。
//!    上一版探针抄的是 `'kkbd'`/`'hotp'`/`'dirt'`/`'hkey'`——**都不存在**，而
//!    `InstallEventHandler` 对着不存在的事件类型照样返回 0，于是那批"注册成功、回调不响"
//!    的读数全是从错误前提读出的假结论。
//! 2. **handler 装在 `GetEventDispatcherTarget()`**：48 次派发全部落在这一份，装在
//!    app target 的那份一次都没收到（连注册在 app target 上的键也是它收的）。写错的
//!    症状最难查——设置页显示"注册成功"，按下去永远没反应。
//! 3. **回调送完意图 SHALL 立刻 `ctx.request_repaint()`**：实测窗收起、帧循环停摆 122 秒
//!    之后按下照样派发，但派发之后循环不会自醒；回调里叫一次，**104ms** 就跑起下一帧，
//!    跑完自己又睡回去。所以这条既是"必须叫"，也是"不用为它立常驻心跳"（NFR-2）。
//! 4. **改键 SHALL 先注销旧的**：同一个 `(键码, 掩码)` 再注册一次返回
//!    `-9878 eventHotKeyExistsErr`——唯一性是系统级按键+掩码去重，与注册到哪个 target
//!    无关，所以注销不是"顺手做的干净活"而是不做就注册不上。
//!
//! 另外两条负结论也记在这里，免得有人去试：`CreateEvent` 一律 `-50`（造不出 Carbon 事件，
//! 所以**没法软件自证热键**，每轮都要人在机器前按）；`_DispatchEvent` 在 arm64 的
//! HIToolbox 里**没有符号**（链接期就报 Undefined symbols），"自己泵一次"那套旧写法不存在。
//!
//! ## 键码表的证据强度不一样
//!
//! `F1=122`、`F2=120`、`F3=99`、`F7=98`、`F8=100`、`G=5`、`H=4`、`J=38`、`K=40` 这九个是
//! 本机实测过的（探针按这些码注册、他按下就命中）。其余那批是公开表抄来的，**没实测**——
//! 所以 W-5.1 走查改过键之后要真按一次，不许把没量的那半张表当成量过的。

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use ice_core::config::{BaseKey, Combo, HotkeyConfig};

use crate::system::hotkeys::Action;

/// Carbon 的事件类：`kEventClassKeyboard`（`CarbonEvents.h:1339`，值是四字符码 `'keyb'`）
const K_EVENT_CLASS_KEYBOARD: u32 = 0x6B65_7962;
/// `kEventHotKeyPressed`（`CarbonEvents.h:4693`）：**是个整数 5**，不是四字符码
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
/// `kEventParamDirectObject`（`CarbonEventsCore.h:178`），值是 `'----'`
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = 0x2D2D_2D2D;
/// `typeEventHotKeyID`（`CarbonEvents.h:376`），值是 `'hkid'`
const TYPE_EVENT_HOT_KEY_ID: u32 = 0x686B_6964;

/// Carbon 的修饰键掩码（`Events.h` 的 bit 8..12）。`rightShift`/`rightOption`/`rightControl`
/// 那几位在 Mac OS X 上不支持，用不到；拿非法掩码去注册实测返回 `-9868 eventInternalErr`
const CARBON_CMD: u32 = 1 << 8;
const CARBON_SHIFT: u32 = 1 << 9;
const CARBON_OPTION: u32 = 1 << 11;
const CARBON_CONTROL: u32 = 1 << 12;

/// 我们自己的 signature，回信里用来确认"这是我们注册的那批键"
const HOT_KEY_SIGNATURE: u32 = 0x4943_4531; // 'ICE1'
const ID_CAPTURE: u32 = 1;
const ID_PIN_LAST: u32 = 2;
const ID_REOPEN: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetEventDispatcherTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
        num_types: i32,
        type_list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        hot_key_id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(handle: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: i32,
        actual_size: *mut i32,
        out_data: *mut c_void,
    ) -> i32;
}

/// 回调拿不到 `self`，只能像托盘那样存一份 `Context`（`system/tray.rs` 同形状）
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();
/// 攒意图的那一格：0 = 没有。三把键各一个码，共用一个原子字节就够（同一时刻只可能有一把按下）
static PENDING: AtomicU8 = AtomicU8::new(CODE_NONE);
const CODE_NONE: u8 = 0;
const CODE_CAPTURE: u8 = 1;
const CODE_PIN_LAST: u8 = 2;
const CODE_REOPEN: u8 = 3;

/// 已经注册出去的三个句柄。**换键与退出都要注销**，只注册不注销等于每次改键多一套活着的绑定。
///
/// Carbon 给的是裸指针（不是引用计数对象，也不归我们释放内存，只作为注销凭据），
/// 而它只在主线程被创建、被注销——这份 `Send` 是照这个前提给的，不是"反正编译器要求"。
struct HandleList(std::vec::Vec<*mut c_void>);
// SAFETY: 句柄只在主线程 `apply()` 与 `unregister_all()` 里读写，跨线程搬运没有实际发生
unsafe impl Send for HandleList {}

static HANDLER_ARMED: OnceLock<Option<()>> = OnceLock::new();

static HANDLES: OnceLock<std::sync::Mutex<HandleList>> = OnceLock::new();

fn handles() -> &'static std::sync::Mutex<HandleList> {
    HANDLES.get_or_init(|| std::sync::Mutex::new(HandleList(Vec::new())))
}

/// Carbon 回调本体。
///
/// **只做两件事**：把码写进原子位、叫循环重绘。这里不索引、不解包、不分配——回调跑在
/// Carbon 的调用栈上，panic 会跨过 FFI 边界。参数顺序按 `CarbonEventsCore.h:893`
/// （大小、实际大小出参、数据出参），写反不报错、只在派发时读脏。
extern "C" fn on_hot_key(_call: *mut c_void, event: *mut c_void, _user: *mut c_void) -> i32 {
    let mut id = EventHotKeyID {
        signature: 0,
        id: 0,
    };
    let mut actual_type: u32 = 0;
    let mut actual_size: i32 = 0;
    let status = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            &mut actual_type,
            std::mem::size_of::<EventHotKeyID>() as i32,
            &mut actual_size,
            &mut id as *mut _ as *mut c_void,
        )
    };
    if status != 0 || id.signature != HOT_KEY_SIGNATURE {
        return status;
    }
    let code = match id.id {
        ID_CAPTURE => CODE_CAPTURE,
        ID_PIN_LAST => CODE_PIN_LAST,
        ID_REOPEN => CODE_REOPEN,
        other => {
            tracing::warn!(hot_key = other, "收到的热键号不是我们注册的");
            CODE_NONE
        }
    };
    if code != CODE_NONE {
        PENDING.store(code, Ordering::SeqCst);
        // 停摆的帧循环不会自己醒：这一句就是那 104 毫秒（事实 38）
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }
    0
}

/// 键位算不出虚拟键码（表里没有那一格）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyFailure {
    /// 键被别的软件占着（`-9878`，或注册时返回任何非 0）
    Taken,
    /// 这一格根本注册不上（平台不支持、Carbon 报错但没有"被占"那种形状）
    Unavailable,
}

impl HotkeyFailure {
    /// 设置页与状态行那一句：**必须给出下一步**，不许静默降级成"只有窗口 focused 时才有效"
    pub const fn explain(self) -> &'static str {
        match self {
            HotkeyFailure::Taken => "这个键被别的软件占用了，换一个组合",
            HotkeyFailure::Unavailable => "这台机器上没能注册全局热键，只能用图片页那颗按钮",
        }
    }
}

/// 这一格能力只在 mac 有实现（Windows 的 `RegisterHotKey` 与本包的 Linux 那一档都只登记
/// 不实现，见 `docs/05` §8 与 `capability()`）。非 mac 上 `apply` 恒 `Err(Unavailable)`，
/// 而调用点本来就该被这个常量挡住——两处都留是为了"忘了判平台"时也不会真去调 Carbon。
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// 主线程调用一次：存下 `Context` 并装上 handler。
///
/// **装在 dispatcher target**（第 2 条硬规矩）。装 handler 与注册键都要在主线程：Carbon
/// 的派发挂在主 run loop 上，而实测 eframe/winit 那个 AppKit 循环会替我们派发（事实 35）。
pub fn arm(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
    // handler 只装一次：重复装会攒出多份回调，一次按下派发多遍（`PENDING` 是覆盖式的，
    // 症状反而是"只响一次"，更难查）
    let Some(once) = HANDLER_ARMED.get_or_init(|| Some(())).clone() else {
        return;
    };
    let _ = once;
    let types = [EventTypeSpec {
        event_class: K_EVENT_CLASS_KEYBOARD,
        event_kind: K_EVENT_HOT_KEY_PRESSED,
    }];
    let mut installed: *mut c_void = std::ptr::null_mut();
    let status = unsafe {
        InstallEventHandler(
            GetEventDispatcherTarget(),
            on_hot_key,
            1,
            types.as_ptr(),
            std::ptr::null_mut(),
            &mut installed,
        )
    };
    if status != 0 {
        tracing::warn!(code = status, "全局热键的处理器装不上");
    }
}

/// 把配置里那三条注册（或**换键时先注销旧的**再注册）。返回 `Err` 由界面说那句能行动的话。
pub fn apply(config: &HotkeyConfig) -> Result<(), HotkeyFailure> {
    if !SUPPORTED {
        return Err(HotkeyFailure::Unavailable);
    }
    unregister_all();
    let plan = [
        (ID_CAPTURE, &config.capture),
        (ID_PIN_LAST, &config.pin_last),
        (ID_REOPEN, &config.reopen),
    ];
    for (id, raw) in plan {
        let Some(combo) = ice_core::config::parse_combo(raw) else {
            // sanitized() 已经把解析不了的换回默认值；走到这里说明有人绕过了那道门
            tracing::warn!("快捷键配置里有一条认不出来，跳过注册");
            continue;
        };
        let Some(key_code) = virtual_key(&combo.base) else {
            tracing::warn!(key = raw.as_str(), "这一格用的键我们不支持");
            return Err(HotkeyFailure::Unavailable);
        };
        let handle = match register(key_code, carbon_mods(&combo), id) {
            Ok(handle) => handle,
            Err(status) => {
                tracing::warn!(code = status, "全局热键注册失败");
                unregister_all();
                return Err(if status == EVENT_HOT_KEY_EXISTS {
                    HotkeyFailure::Taken
                } else {
                    HotkeyFailure::Unavailable
                });
            }
        };
        if let Ok(mut list) = handles().lock() {
            list.0.push(handle);
        }
    }
    Ok(())
}

/// `eventHotKeyExistsErr`（`CarbonEventsCore.h:152`）
const EVENT_HOT_KEY_EXISTS: i32 = -9878;

fn register(key_code: u16, modifiers: u32, id: u32) -> Result<*mut c_void, i32> {
    let mut handle: *mut c_void = std::ptr::null_mut();
    let status = unsafe {
        RegisterEventHotKey(
            key_code as u32,
            modifiers,
            EventHotKeyID {
                signature: HOT_KEY_SIGNATURE,
                id,
            },
            GetEventDispatcherTarget(),
            0,
            &mut handle,
        )
    };
    if status == 0 { Ok(handle) } else { Err(status) }
}

fn unregister_all() {
    let Ok(mut list) = handles().lock() else {
        return;
    };
    for handle in list.0.drain(..) {
        if !handle.is_null() {
            unsafe {
                UnregisterEventHotKey(handle);
            }
        }
    }
}

/// 帧循环（`App::logic`）取走热键命中。**泵挂在 `logic()` 而不是 `ui()`**：窗口收起时
/// `ui()` 一帧都不跑，挂在 `ui()` 上就等于"只有窗口已经看得见时才听得到热键"（D-21 同族）
pub fn take_action() -> Option<Action> {
    action_of_code(PENDING.swap(CODE_NONE, Ordering::SeqCst))
}

/// 那三把键各自对一条 `Action`（抽出来才能无头断言"号没并错"）
fn action_of_code(code: u8) -> Option<Action> {
    match code {
        CODE_CAPTURE => Some(Action::Capture),
        CODE_PIN_LAST => Some(Action::PinLast),
        CODE_REOPEN => Some(Action::ReopenLast),
        _ => None,
    }
}

/// 有没有待处理的热键命中（隐藏态那一帧用它决定要不要买回一帧）
pub fn has_pending() -> bool {
    PENDING.load(Ordering::SeqCst) != CODE_NONE
}

/// Carbon 的修饰键掩码。
pub fn carbon_mods(combo: &Combo) -> u32 {
    let mut mods = 0;
    if combo.cmd {
        mods |= CARBON_CMD;
    }
    if combo.shift {
        mods |= CARBON_SHIFT;
    }
    if combo.option {
        mods |= CARBON_OPTION;
    }
    if combo.control {
        mods |= CARBON_CONTROL;
    }
    mods
}

/// 主键 → macOS 虚拟键码。表头那九个键位是实测过的（文件头写了），其余按公开表抄。
pub fn virtual_key(base: &BaseKey) -> Option<u16> {
    Some(match base {
        BaseKey::Function(n) => match n {
            1 => 122,
            2 => 120,
            3 => 99,
            4 => 118,
            5 => 96,
            6 => 97,
            7 => 98,
            8 => 100,
            9 => 101,
            10 => 109,
            11 => 103,
            12 => 111,
            _ => return None,
        },
        BaseKey::Letter(ch) => match ch {
            'a' => 0,
            's' => 1,
            'd' => 2,
            'f' => 3,
            'h' => 4,
            'g' => 5,
            'z' => 6,
            'x' => 7,
            'c' => 8,
            'v' => 9,
            'b' => 11,
            'q' => 12,
            'w' => 13,
            'e' => 14,
            'r' => 15,
            'y' => 16,
            't' => 17,
            'o' => 31,
            'u' => 32,
            'i' => 34,
            'p' => 35,
            'l' => 37,
            'j' => 38,
            'k' => 40,
            'n' => 45,
            'm' => 46,
            _ => return None,
        },
        BaseKey::Digit(ch) => match ch {
            '1' => 18,
            '2' => 19,
            '3' => 20,
            '4' => 21,
            '5' => 23,
            '6' => 22,
            '7' => 26,
            '8' => 28,
            '9' => 25,
            '0' => 29,
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_core::config::parse_combo;

    /// 他报的三个默认值都能翻成键码与掩码（**裸键**：掩码为 0）
    #[test]
    fn the_three_defaults_register_as_bare_function_keys() {
        let config = HotkeyConfig::default();
        for raw in [&config.capture, &config.pin_last, &config.reopen] {
            let combo = parse_combo(raw).unwrap_or_else(|| panic!("{raw} 该能解析"));
            assert!(virtual_key(&combo.base).is_some(), "{raw} 没有键码");
            assert_eq!(carbon_mods(&combo), 0, "{raw} 是裸键，不该带掩码");
        }
        // 实测确认过的三个码（探针按这些值注册、他按下命中）
        assert_eq!(virtual_key(&BaseKey::Function(1)), Some(122));
        assert_eq!(virtual_key(&BaseKey::Function(2)), Some(120));
        assert_eq!(virtual_key(&BaseKey::Function(3)), Some(99));
    }

    /// 掩码那一位一位都不能错：错的掩码实测返回 `-9868 eventInternalErr`，
    /// 而"⌃⌥ 当成 ⌥+大小写锁"这种混法正是上一版探针踩过的坑
    #[test]
    fn the_carbon_modifier_bits_are_the_measured_ones() {
        assert_eq!(CARBON_CMD, 256);
        assert_eq!(CARBON_SHIFT, 512);
        assert_eq!(CARBON_OPTION, 2048);
        assert_eq!(CARBON_CONTROL, 4096);
        let combo = parse_combo("ctrl+option+f2").unwrap();
        assert_eq!(carbon_mods(&combo), CARBON_CONTROL | CARBON_OPTION);
        assert_eq!(carbon_mods(&parse_combo("cmd+shift+4").unwrap()), 256 | 512);
    }

    /// 事件类型那四个值：**这就是上一版整批读数为假的根因**，锁死字面值
    #[test]
    fn the_carbon_event_type_constants_are_the_ones_from_the_sdk() {
        assert_eq!(K_EVENT_CLASS_KEYBOARD, u32::from_be_bytes(*b"keyb"));
        assert_eq!(K_EVENT_HOT_KEY_PRESSED, 5);
        assert_eq!(K_EVENT_PARAM_DIRECT_OBJECT, u32::from_be_bytes(*b"----"));
        assert_eq!(TYPE_EVENT_HOT_KEY_ID, u32::from_be_bytes(*b"hkid"));
    }

    /// 三个号各对一条 `Action`；没有命中时什么都不给
    #[test]
    fn each_registered_id_maps_to_its_own_action() {
        assert_eq!(action_of_code(CODE_CAPTURE), Some(Action::Capture));
        assert_eq!(action_of_code(CODE_PIN_LAST), Some(Action::PinLast));
        assert_eq!(action_of_code(CODE_REOPEN), Some(Action::ReopenLast));
        assert_eq!(action_of_code(CODE_NONE), None);
        // 三把互不相同，共用一个原子字节才不会被并成一把
        assert_ne!(CODE_CAPTURE, CODE_PIN_LAST);
        assert_ne!(CODE_PIN_LAST, CODE_REOPEN);
    }

    /// 失败那句话得给出下一步（REQ-006：不许静默降级成"只有窗口 focused 才有效"）
    #[test]
    fn a_failed_binding_says_what_to_do_next() {
        assert!(HotkeyFailure::Taken.explain().contains("换"));
        assert!(HotkeyFailure::Unavailable.explain().contains("按钮"));
        assert!(!HotkeyFailure::Unavailable.explain().contains("成功"));
    }

    #[test]
    fn the_pending_slot_holds_one_action_at_a_time() {
        PENDING.store(CODE_NONE, Ordering::SeqCst);
        assert!(!has_pending());
        assert_eq!(take_action(), None);
        PENDING.store(CODE_REOPEN, Ordering::SeqCst);
        assert!(has_pending());
        // 取走即清空：同一次按下不许在下一帧又被当成一次新的
        assert_eq!(take_action(), Some(Action::ReopenLast));
        assert_eq!(take_action(), None);
    }

    /// 形状核查：**不许出现"自己泵 Carbon 队列"那套写法**（`_DispatchEvent` 在 arm64
    /// 根本没有符号，链接期就失败；旧 Carbon 书里的那段代码照抄进来只会白占一段注释）
    #[test]
    fn the_module_never_claims_to_pump_carbon_itself() {
        let source = include_str!("global_hotkeys.rs");
        // 只看实现：注释里提过那个 API（说为什么不能用），而这条断言自己的字符串里
        // 也含那个名字——不先剥掉测试模块与注释，它就会抓到自己的文本
        let code = source
            .split("#[cfg(test)]")
            .next()
            .unwrap_or(source)
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("DispatchEvent("),
            "出现了手动泵 Carbon 的写法"
        );
        assert!(!code.contains("ReceiveEvent("), "同上");
        assert!(
            code.contains("GetEventDispatcherTarget()"),
            "handler 与注册的落点就是 dispatcher target，这一处不许换"
        );
    }
}
