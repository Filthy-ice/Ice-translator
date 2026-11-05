//! 文件面板（CHG-005 REQ-001 的「打开图片」、CHG-006 REQ-008 的「保存」；Q-002：自接 AppKit，
//! 不引 `rfd`）。
//!
//! 只做一件事：把面板交出去、等用户选完、把**路径**交回帧循环。像素、解码、缩放、编码一概不在
//! 这里——那几段在 `ice_vision::import` 与 `ice_vision::export`，所以本模块不需要认识图片格式。
//!
//! 为什么是 `beginWithCompletionHandler:` 而不是 `runModal`：后者把线程关进 AppKit 自己的嵌套
//! 事件循环，eframe 的帧循环在那期间一帧都不跑（REQ-001 明写 SHALL NOT）。回调只 `try_send`
//! 一条消息就返回，真正的 `admit_path` 与识别派发都发生在帧循环里——回调跑在 ObjC 的调用栈上，
//! 那里 panic 会跨过 FFI 边界，所以回调体里只做发消息与取值，不索引、不解包。
//!
//! 两个面板（打开 / 保存）共用同一套协议：同一个 `Reply` 三态、同一个 `classify`、同一条容量 4
//! 的有界通道类型、同一个"结果只能问面板身上那个 `URL`"的取法（[`reply_of`]）。**各自一条通道
//! 实例**，不共用一对收发端：回信里只有路径，共用一条就等于让"用户选了一张要打开的图"和
//! "用户选了一个要写出去的位置"长得一模一样，帧循环无从分辨，会把成品写进用户刚选来识别的图里。

use std::path::{Path, PathBuf};

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

/// 保存面板允许的扩展名（REQ-008）。与打开那一份**内容相同但各留一份**：打开那一侧说的是
/// "本版本能解什么"，跟着 `ice_vision::import` 走；这一侧说的是"本版本能编什么"，跟着
/// `ice_vision::export::Format` 走。两边今天一样，改一边不该顺带改另一边。
///
/// `png` 必须排在第一个：`NSSavePanel` 在用户没写扩展名时**拿数组第一项当扩展名补上**
/// （`objc2-app-kit-0.3.2/src/generated/NSSavePanel.rs:680` 那段文档原话），这与 Q-005
/// 定的"没写扩展名补 `.png`"正好同一档。
pub const SAVE_TYPES: [&str; 3] = ["png", "jpg", "jpeg"];

/// 用户在面板里没写扩展名时补上的那一个（Q-005）。也是 `SAVE_TYPES` 的第一项——两处必须
/// 同一个词，否则面板自己补的与我们补的会打架。
const DEFAULT_EXTENSION: &str = "png";

/// 预填名字里的那个词（REQ-008：默认名 = 源图文件名 + `-译文.png`）
const SAVE_WORD: &str = "译文";

/// 源图名与那个词之间的连接符
const SAVE_JOINER: &str = "-";

/// 保存面板里预填的那个名字：源图文件名去掉扩展名，接上 `-译文.png`。
///
/// 吃的是 `Admitted.label`——那已经是**文件名**而不是路径（`import::admit_path` 取的
/// `file_name()`，剪贴板那一路是"剪贴板"）。这里仍走一次 `file_stem()`，为的是标签万一
/// 带上了目录也不会把整条路径塞进面板的名字格。
///
/// **不许把识别出来的原文或译文拼进这个名字**：文件名会进系统的最近使用列表、进 Finder、
/// 进用户自己的备份索引，那是把用户内容写到应用外面去（NFR-3 的同一口径）。
pub fn default_save_name(label: &str) -> String {
    let stem = Path::new(label)
        .file_stem()
        .map(|name| name.to_string_lossy().trim().to_string())
        .unwrap_or_default();
    // 拿不到源图文件名时不带连接符：以 `-` 开头的文件名在 shell 里会被当成选项
    let joined = if stem.is_empty() {
        SAVE_WORD.to_string()
    } else {
        format!("{stem}{SAVE_JOINER}{SAVE_WORD}")
    };
    format!("{joined}.{DEFAULT_EXTENSION}")
}

/// 面板回来那条路径的收口：**没写扩展名就补 `.png`**，写了的原样留着（Q-005）。
///
/// 补名字发生在用户已经看过面板、敲完名字之后，所以它属于这一层而不是 `export::save`
/// ——那一律原样使用交给它的路径，不偷偷替用户改名（`export.rs` 模块注释里那条纪律）。
/// 格式仍由 `export::format_of` 按扩展名判，认不出的（`归档.tar.gz` 那种）走 PNG。
///
/// `shot.` 这种"点后面空的"算没写扩展名（`Path::extension()` 给的是空串），
/// 用 `set_extension` 收口，于是它变成 `shot.png` 而不是 `shot..png`。
pub fn resolved_save_path(picked: &str) -> PathBuf {
    let mut path = PathBuf::from(picked);
    let named = path.extension().is_some_and(|ext| !ext.is_empty());
    if !named {
        path.set_extension(DEFAULT_EXTENSION);
    }
    path
}

/// 扩展名列表 → AppKit 那个 `NSArray<NSString>`。两个面板同一条，因为
/// `setAllowedFileTypes:` 收的就是这个类型。
#[cfg(target_os = "macos")]
fn type_list(
    types: &[&str],
) -> objc2::rc::Retained<objc2_foundation::NSArray<objc2_foundation::NSString>> {
    use objc2::rc::Retained;
    use objc2_foundation::{NSArray, NSString};
    NSArray::from_retained_slice(
        &types
            .iter()
            .map(|ext| NSString::from_str(ext))
            .collect::<Vec<Retained<NSString>>>(),
    )
}

/// 面板关闭之后向它问结果。`URL()` 是结果的唯一载体，而 `NSOpenPanel : NSSavePanel`，
/// 所以两个面板共用这一问——"取消 / OK 无路径 / OK 有路径"三档的判定也就只有一处（[`classify`]）。
///
/// `NSModalResponse` 就是 `NSInteger`（64 位目标机上是 `isize`），纯函数按 `i64` 判定，
/// 这里转一次：数值本身远小于 `isize` 上限，转换不会丢信息。
#[cfg(target_os = "macos")]
fn reply_of(panel: &objc2_app_kit::NSSavePanel, response: objc2_app_kit::NSModalResponse) -> Reply {
    use objc2::rc::Retained;
    use objc2_foundation::NSURL;
    let path = panel
        .URL()
        .and_then(|url: Retained<NSURL>| url.path().map(|p| p.to_string()));
    classify(response as i64, path)
}

/// 回调体的三件事，两个面板一字不差：问结果、`try_send`、叫醒帧循环。
///
/// 第三条是这条链上最容易在第二个宿主里被忘掉的一件——面板那一下点击不属于我们的窗口，
/// 没有它就没人来读这条回信（与 `system::dblclick` 的 `hand_over` 同一条教训，D-25）。
/// 发送失败等于界面已经不读了（退出中），不重试也不记录内容。
#[cfg(target_os = "macos")]
fn hand_back(
    panel: &objc2_app_kit::NSSavePanel,
    response: objc2_app_kit::NSModalResponse,
    sender: &ReplySender,
    repaint: &egui::Context,
) {
    let _ = sender.try_send(reply_of(panel, response));
    repaint.request_repaint();
}

/// 请求打开面板。返回 `false` 表示这条路走不通（非 macOS、不在主线程），调用方据此提示降级。
///
/// 不阻塞：`beginWithCompletionHandler:` 立刻返回，结果走 `tx`。
/// `ctx` 只为在回调里请求一次重绘——面板是 AppKit 自己的窗口，用户点"打开"的那一下
/// 不会给我们的窗口产生任何输入事件，帧循环因此不会自动醒来读这条回信。
#[cfg(target_os = "macos")]
pub fn request_open(tx: &ReplySender, ctx: &egui::Context) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponse, NSOpenPanel};
    use objc2_foundation::NSString;

    // 面板必须在主线程创建与展示。拿不到标记就是不在主线程——宁可不弹，也不在别的线程碰 AppKit。
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(true);
    // 替代品 `setAllowedContentTypes:` 收的是 `NSArray<UTType>`，而 `UTType` 住在
    // `objc2-core-services` 里——为了三个扩展名字符串往依赖图里再加一个 Apple 绑定 crate 不值。
    // 这条被标记弃用的 setter 在 macOS 26 上仍然是可用入口（文档只说
    // "use allowedContentTypes instead"，没有删除它），且语义就是我们要的"按扩展名过滤"。
    let types = type_list(&ALLOWED_TYPES);
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&types));
    panel.setMessage(Some(&NSString::from_str("选择要识别的图片")));

    let sender = tx.clone();
    // block 要求 `'static`，所以搬进去的是一份 `Context`（它内部就是 `Arc`，clone 不复制状态）。
    let repaint = ctx.clone();
    // 回调要在面板关闭之后还能问它要结果，所以把一份强引用搬进 block；
    // `Retained` 不是 `Copy`，外面那次 `beginWithCompletionHandler` 用的是另一份。
    // 交进 `hand_back` 时沿父类链退一步（`NSOpenPanel : NSSavePanel`），两个面板于是同一问法。
    let result_panel = panel.clone();
    let block = block2::RcBlock::new(move |response: NSModalResponse| {
        hand_back(&result_panel, response, &sender, &repaint);
    });
    panel.beginWithCompletionHandler(&block);
    true
}

/// 非 macOS 没有 AppKit 面板：恒 `false`，由调用方退回「手输路径」那条入口并给出降级提示（NFR-7）。
#[cfg(not(target_os = "macos"))]
pub fn request_open(_tx: &ReplySender, _ctx: &egui::Context) -> bool {
    false
}

/// 请求保存面板（REQ-008）。协议与 [`request_open`] 完全一致：异步、回信只带路径、
/// 取消与失败分得开、信道满就丢回信不阻塞回调。
///
/// **同名覆盖一个 setter 都不碰**：`NSSavePanel` 自己就会问"要替换吗"，那是系统语义。
/// 自己实现一套 `-1`/`-2` 自动改名会让用户以为没保存上（REQ-008 的 SHALL NOT）。
/// `setCanCreateDirectories(true)` 是给"保存那一刻才想单开一个目录"的人，不是替用户建目录。
#[cfg(target_os = "macos")]
pub fn request_save(
    tx: &ReplySender,
    ctx: &egui::Context,
    default_name: &str,
    allowed_types: &[&str],
) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponse, NSSavePanel};
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let panel = NSSavePanel::savePanel(mtm);
    panel.setCanCreateDirectories(true);
    panel.setNameFieldStringValue(&NSString::from_str(default_name));
    let types = type_list(allowed_types);
    // 弃用理由与 `request_open` 那处同一条，见上
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&types));
    panel.setMessage(Some(&NSString::from_str("选择译文图片保存到哪里")));

    let sender = tx.clone();
    let repaint = ctx.clone();
    let result_panel = panel.clone();
    let block = block2::RcBlock::new(move |response: NSModalResponse| {
        hand_back(&result_panel, response, &sender, &repaint);
    });
    panel.beginWithCompletionHandler(&block);
    true
}

/// 非 macOS 没有 AppKit 面板：恒 `false`，调用方给出降级提示（那台机器上「复制图片」还在，
/// 所以这条路走不通不等于成品出不去，NFR-7）。
#[cfg(not(target_os = "macos"))]
pub fn request_save(
    _tx: &ReplySender,
    _ctx: &egui::Context,
    _default_name: &str,
    _allowed_types: &[&str],
) -> bool {
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

    /// 预填名的组装规则（REQ-008）。两条不许破的：目录不进名字格，识别出的原文也不进
    /// ——名字会进系统的最近使用列表，那是把用户内容写到应用外面去。
    #[test]
    fn the_prefilled_name_is_the_source_file_name_plus_a_suffix() {
        assert_eq!(default_save_name("shot.png"), "shot-译文.png");
        assert_eq!(
            default_save_name("截图 2026-11-05.jpeg"),
            "截图 2026-11-05-译文.png"
        );
        // 只脱最后一个扩展名：`归档.tar` 是名字的一部分
        assert_eq!(default_save_name("归档.tar.gz"), "归档.tar-译文.png");
        // 剪贴板那一路的标签没有扩展名
        assert_eq!(default_save_name("剪贴板"), "剪贴板-译文.png");
        // 标签万一带着目录，也只取文件名那一截
        assert_eq!(
            default_save_name("/Users/me/Desktop/shot.png"),
            "shot-译文.png"
        );
        // 拿不到名字时不以 `-` 开头（那种文件名在 shell 里会被当成选项）
        assert_eq!(default_save_name(""), "译文.png");
        assert_eq!(default_save_name("   "), "译文.png");

        for label in ["shot.png", "/a/b/c.jpg", "", "剪贴板"] {
            let name = default_save_name(label);
            assert!(
                !name.contains('/') && name.ends_with(".png"),
                "预填名 {name:?} 要么带了目录要么没落在 PNG 上"
            );
        }
    }

    /// 补扩展名这一档（Q-005）：没写的补 `.png`，写了的一个字符都不改——包括那两个
    /// 认不出的（`归档.tar.gz`），改名等于替用户决定文件叫什么。
    ///
    /// 最后一段是把两处口径钉在一起：补完名字再问 `export::format_of`，得到的格式必须
    /// 还是用户选的那个。两边各写一遍扩展名判定的话，这里就会红。
    #[test]
    fn a_nameless_pick_gains_png_and_a_named_one_is_left_alone() {
        use ice_vision::export::{Format, format_of};

        assert_eq!(resolved_save_path("/tmp/成品"), Path::new("/tmp/成品.png"));
        assert_eq!(
            resolved_save_path("/tmp/成品.jpg"),
            Path::new("/tmp/成品.jpg")
        );
        assert_eq!(
            resolved_save_path("/tmp/成品.JPEG"),
            Path::new("/tmp/成品.JPEG")
        );
        assert_eq!(resolved_save_path("/tmp/成品."), Path::new("/tmp/成品.png"));
        assert_eq!(
            resolved_save_path("/tmp/归档.tar.gz"),
            Path::new("/tmp/归档.tar.gz")
        );

        for (picked, want) in [
            ("/tmp/成品", Format::Png),
            ("/tmp/成品.", Format::Png),
            ("/tmp/成品.png", Format::Png),
            ("/tmp/成品.jpg", Format::Jpeg),
            ("/tmp/成品.jpeg", Format::Jpeg),
            ("/tmp/归档.tar.gz", Format::Png),
        ] {
            let path = resolved_save_path(picked);
            assert_eq!(format_of(&path), want, "{picked} 补完之后格式变了");
        }
        // 面板允许的三个扩展名，补名这一层一个都不该改写
        for ext in SAVE_TYPES {
            let picked = format!("/tmp/成品.{ext}");
            assert_eq!(
                resolved_save_path(&picked),
                Path::new(&picked),
                "{ext} 是面板自己给的类型，不该被补名动过"
            );
        }
        assert_eq!(
            DEFAULT_EXTENSION, SAVE_TYPES[0],
            "面板补的与我们补的要同一个词"
        );
    }
}
