//! 「打开图片」面板（CHG-005 REQ-001、Q-002：自接 `NSOpenPanel`，不引 `rfd`）。
//!
//! 只做一件事：把面板交出去、等用户选完、把**路径**交回帧循环。像素、解码、缩放一概不在这里——
//! 那三段在 `ice_vision::import`，所以本模块不需要认识图片格式。
//!
//! 为什么是 `beginWithCompletionHandler:` 而不是 `runModal`：后者把线程关进 AppKit 自己的嵌套
//! 事件循环，eframe 的帧循环在那期间一帧都不跑（REQ-001 明写 SHALL NOT）。回调只 `try_send`
//! 一条消息就返回，真正的 `admit_path` 与识别派发都发生在帧循环里——回调跑在 ObjC 的调用栈上，
//! 那里 panic 会跨过 FFI 边界，所以回调体里只做发消息与取值，不索引、不解包。

/// 面板回来后给界面的一条消息
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// 用户选中了一个文件，值为路径
    Picked(String),
    /// 用户按了取消：什么都不该改
    Cancelled,
    /// 面板没弹起来、或回来时既没有 OK 也没有 URL
    Failed,
}

pub type ReplySender = tokio::sync::mpsc::Sender<Reply>;
pub type ReplyReceiver = tokio::sync::mpsc::Receiver<Reply>;

/// 容量 4：同一时刻只可能有一个面板在用户手上，余量留给"回调连着来、帧循环还没读到"的那一两帧
pub fn channel() -> (ReplySender, ReplyReceiver) {
    tokio::sync::mpsc::channel(4)
}

/// `NSModalResponse` 的三个取值（`objc2-app-kit-0.3.2/src/generated/NSApplication.rs:264`
/// 把它定义成 `NSInteger`，同文件 269-272 与 `NSWindow.rs:69-73` 给出常量）。
/// 那两个常量锁在 `feature = "NSApplication"` 后面，纯函数要能被非 mac 编译面测试，所以这里
/// 按数值判定，`-1001`（Abort，面板弹不起来）与其余一切取值都归 `Failed`。
const OK: i64 = 1;
const CANCEL: i64 = 0;

/// 一次面板回应的判定：OK 且拿到路径才算 `Picked`，OK 但没路径算 `Failed`。
///
/// 单独拎出来是因为它是这条链上唯一能被无头断言的一段——面板本体只能真机验证。
pub fn classify(response: i64, path: Option<String>) -> Reply {
    match response {
        OK => path.map(Reply::Picked).unwrap_or(Reply::Failed),
        CANCEL => Reply::Cancelled,
        _ => Reply::Failed,
    }
}

/// 本版本能解的扩展名，与 `ice_vision::import` 的解码能力同口径（Q-005 选 (a)：PNG/JPEG）。
/// 面板里灰掉别的类型是**提示**，不是**限制**：用户仍能选到 `.jpg` 之外的名字，
/// 那时由 `admit_path` 给出「这张图解不开」那一档文案。
const ALLOWED_TYPES: [&str; 3] = ["png", "jpg", "jpeg"];

/// 请求打开面板。返回 `false` 表示这条路走不通（非 macOS、不在主线程），调用方据此提示降级。
///
/// 不阻塞：`beginWithCompletionHandler:` 立刻返回，结果走 `tx`。
/// `ctx` 只为在回调里请求一次重绘——面板是 AppKit 自己的窗口，用户点"打开"的那一下
/// 不会给我们的窗口产生任何输入事件，帧循环因此不会自动醒来读这条回信。
#[cfg(target_os = "macos")]
pub fn request_open(tx: &ReplySender, ctx: &egui::Context) -> bool {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSModalResponse, NSOpenPanel};
    use objc2_foundation::{NSArray, NSString, NSURL};

    // 面板必须在主线程创建与展示。拿不到标记就是不在主线程——宁可不弹，也不在别的线程碰 AppKit。
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(true);
    let types = NSArray::from_retained_slice(
        &ALLOWED_TYPES
            .iter()
            .map(|ext| NSString::from_str(ext))
            .collect::<Vec<Retained<NSString>>>(),
    );
    // 替代品 `setAllowedContentTypes:` 收的是 `NSArray<UTType>`，而 `UTType` 住在
    // `objc2-core-services` 里——为了三个扩展名字符串往依赖图里再加一个 Apple 绑定 crate 不值。
    // 这条被标记弃用的 setter 在 macOS 26 上仍然是 `NSOpenPanel` 的可用入口（文档只说
    // "use allowedContentTypes instead"，没有删除它），且语义就是我们要的"按扩展名过滤"。
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&types));
    panel.setMessage(Some(&NSString::from_str("选择要识别的图片")));

    let sender = tx.clone();
    // block 要求 `'static`，所以搬进去的是一份 `Context`（它内部就是 `Arc`，clone 不复制状态）。
    let repaint = ctx.clone();
    // 回调要在面板关闭之后还能问它要结果，所以把一份强引用搬进 block；
    // `Retained` 不是 `Copy`，外面那次 `beginWithCompletionHandler` 用的是另一份。
    let result_panel = panel.clone();
    let block = block2::RcBlock::new(move |response: NSModalResponse| {
        // `URL()` 只能在回调里问：面板关闭之后它自己就是结果的唯一载体
        let path = result_panel
            .URL()
            .and_then(|url: Retained<NSURL>| url.path().map(|p| p.to_string()));
        // `NSModalResponse` 就是 `NSInteger`（64 位目标机上是 `isize`），纯函数按 `i64` 判定，
        // 这里转一次：数值本身远小于 `isize` 上限，转换不会丢信息。
        let reply = classify(response as i64, path);
        // 发送失败等于界面已经不读了（退出中），这里不重试也不记录内容
        let _ = sender.try_send(reply);
        // 无论发没发成都要叫醒帧循环：面板那一下点击不属于我们的窗口，没有这条就没人来读
        repaint.request_repaint();
    });
    panel.beginWithCompletionHandler(&block);
    true
}

/// 非 macOS 没有 AppKit 面板：恒 `false`，由调用方退回「手输路径」那条入口并给出降级提示（NFR-7）。
#[cfg(not(target_os = "macos"))]
pub fn request_open(_tx: &ReplySender, _ctx: &egui::Context) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_with_a_path_is_the_only_pick() {
        assert_eq!(
            classify(OK, Some("/tmp/a.png".into())),
            Reply::Picked("/tmp/a.png".into())
        );
    }

    /// OK 却问不出路径（沙盒书签失效、文件在选中瞬间被移走）不能当成"用户什么都没做"，
    /// 否则界面停在空闲态、用户以为面板还没回来
    #[test]
    fn ok_without_a_path_reads_as_failure_not_cancellation() {
        assert_eq!(classify(OK, None), Reply::Failed);
        assert_ne!(classify(OK, None), Reply::Cancelled);
    }

    #[test]
    fn cancel_and_abort_are_told_apart() {
        assert_eq!(classify(CANCEL, None), Reply::Cancelled);
        // -1001 = NSModalResponseAbort：面板弹不起来。取消是用户的选择，这条不是
        assert_eq!(classify(-1001, None), Reply::Failed);
        // 取消时即便带着路径也不认：用户没确认，这张图不该入场
        assert_eq!(
            classify(CANCEL, Some("/tmp/a.png".into())),
            Reply::Cancelled
        );
    }

    #[test]
    fn an_unknown_response_fails_closed() {
        assert_eq!(classify(-1002, Some("/tmp/a.png".into())), Reply::Failed);
    }

    /// 面板回信走的是一条有界通道：满了就丢，不阻塞回调
    #[test]
    fn a_full_channel_drops_the_reply_instead_of_blocking() {
        let (tx, mut rx) = channel();
        for _ in 0..4 {
            assert!(tx.try_send(Reply::Cancelled).is_ok());
        }
        assert!(
            tx.try_send(Reply::Failed).is_err(),
            "容量 4 用满后必须拒收而不是等待"
        );
        assert_eq!(rx.try_recv(), Ok(Reply::Cancelled));
    }
}
