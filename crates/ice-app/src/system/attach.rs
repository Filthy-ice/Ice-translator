//! 把药丸窗用原生 `addChildWindow:` 绑成贴图窗的**子窗**（CHG-011 治跟随的根本办法）。
//!
//! 为什么要有这一步：egui 给贴图与药丸各开的是两扇**平级顶层窗**，系统不认识它们的父子关系。
//! 之前靠我们每帧把药丸位置推过去追贴图——可拖动一旦被系统 `StartDrag` 接管，我们的每帧回调
//! 在那段嵌套循环里基本不被叫醒，追不上就成了"一卡一卡"（用户 2026-11-10 真机反馈）。
//! 原生 `NSWindow.addChildWindow:ordered:` 让 macOS 知道"这扇跟着那扇走"，之后**系统按固定
//! 偏移带着子窗移动**，不再需要我们每帧去追——这才和 某截图工具 / 某聊天软件那种浮条一个原理。
//!
//! 只在 macOS 上有意义；非 mac 一律返回 `false`，不假装绑上了（跟随退回每帧推那一条）。

/// 按标题在 AppKit 的窗列表里找那扇窗。同名取**最后一个**（我们刚开的那一扇）。
#[cfg(target_os = "macos")]
pub fn find_window(title: &str) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let want = NSString::from_str(title);
    let mut found = None;
    for w in app.windows().iter() {
        if w.title() == want {
            found = Some(w.clone());
        }
    }
    found
}

/// 把这号贴图的药丸窗绑成贴图窗的子窗。两扇窗都建出来了才成功（`true`）。
///
/// 幂等：已经是它的子窗就直接返回 `true`，不重复 add（每帧调也安全）。药丸窗是 `show_viewport_deferred`
/// 建的，头一两帧可能还不存在 → 返回 `false`，调用方下一帧再试。绑上之后贴图被拖动由系统带子窗，
/// 我们 SHALL NOT 再每帧给它发位置命令（会和系统抢位置）。
#[cfg(target_os = "macos")]
pub fn attach_pill_to_pin(pin_id: u64) -> bool {
    use objc2_app_kit::NSWindowOrderingMode;
    let parent = match find_window(&crate::system::pin::window_title(pin_id)) {
        Some(w) => w,
        None => return false,
    };
    let child = match find_window(&crate::system::pill::window_title(pin_id)) {
        Some(w) => w,
        None => return false,
    };
    // 已经是子窗就别重复 add：`addChildWindow:` 对已在列表里的窗行为不稳，先查。
    // 药丸窗标题每号唯一（`贴图操作栏 {id}`），按标题认身份就够，不必比对象指针。
    if let Some(kids) = parent.childWindows() {
        let want = child.title();
        for w in kids.iter() {
            if w.title() == want {
                return true;
            }
        }
    }
    unsafe {
        parent.addChildWindow_ordered(&child, NSWindowOrderingMode::Above);
    }
    true
}

/// 解绑（贴图收起 / 关掉时调）。拿不到窗就算了——窗都没了，绑定自然随之消失。
#[cfg(target_os = "macos")]
pub fn detach_pill_from_pin(pin_id: u64) {
    if let Some(parent) = find_window(&crate::system::pin::window_title(pin_id)) {
        if let Some(child) = find_window(&crate::system::pill::window_title(pin_id)) {
            parent.removeChildWindow(&child);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn attach_pill_to_pin(_pin_id: u64) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn detach_pill_from_pin(_pin_id: u64) {}
