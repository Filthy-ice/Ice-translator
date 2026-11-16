//! 界面状态与状态转移。
//!
//! 这里只放纯逻辑，不碰 egui 绘制，因此 REQ-001~REQ-004、REQ-007 的行为都能无头断言。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use eframe::egui::Rect;
use egui::{Pos2, Vec2};
use ice_core::config::{Config, UiMode};
use ice_core::document::OcrDocument;
use ice_core::glossary::{Glossary, Term};
use ice_core::history::{HISTORY_CAPACITY, HistoryRecord, HistoryStore};
use ice_core::language::{EngineKind, LangPair, Language};
use ice_providers::error::ProviderError;
use ice_providers::request::{TranslationRequest, TranslationResult};
use ice_vision::composite::Composed;
use ice_vision::error::VisionError;
use ice_vision::import::Admitted;
use ice_vision::typeset::CompositeMode;
use image::RgbaImage;

use crate::system::capture;

pub type RequestId = u64;

/// 全量模式的三个页签（`03-ui-design.md` §2 F-01）。术语表是设置页内的一个分区（F-12），不占页签。
/// 图片页签 2026-11-16 按用户要求删除：图片只靠 F1 截图入场，识别/翻译/合成引擎与贴图链保留。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Text,
    History,
    Settings,
}

impl Tab {
    pub const fn all() -> [Tab; 3] {
        [Tab::Text, Tab::History, Tab::Settings]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Tab::Text => "文本",
            Tab::History => "历史",
            Tab::Settings => "设置",
        }
    }
}

/// 只有需要"给出动作按钮"的错误才映射到动作（架构 §5 的规则）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    GoSettings,
    UseMachine,
}

impl FailureAction {
    pub const fn label(self) -> &'static str {
        match self {
            FailureAction::GoSettings => "去设置",
            FailureAction::UseMachine => "改用机翻",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub action: Option<FailureAction>,
}

/// 错误文案一律取自 `ProviderError` 的 `Display`，UI 不重新拼语义（FR-1.7）。
/// `current` 用来避免在机翻通道上给出"改用机翻"这种没有意义的动作。
pub fn failure_of(err: &ProviderError, current: EngineKind) -> Failure {
    let action = match err {
        ProviderError::Unauthorized | ProviderError::NotConfigured => {
            Some(FailureAction::GoSettings)
        }
        ProviderError::Timeout(..) if current == EngineKind::Ai => Some(FailureAction::UseMachine),
        _ => None,
    };
    Failure {
        message: err.to_string(),
        action,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Warn,
    Error,
}

/// 采信检测结果的置信度门槛，与机翻引擎内部同源（`machine.rs:62`）。
/// 低于它的属于"猜的"，拿它去改用户的语言设置比重试一次更糟。
const DETECT_CONFIDENCE: f32 = 0.3;

/// 图片那一批定不出方向时给的那句话（REQ-001 第四条：`auto` 不许原样交给机翻端点）。
///
/// 说清缺的是哪一个选择，而不是笼统一句"翻译失败"：这一条不重试也不换引擎就能解决。
const NO_DIRECTION_TEXT: &str = "认不出图上的文字是什么语言，请在语言对里指定源语言";

/// REQ-006：检出的语言与当前目标语言相同（等于"自己译自己"）时，给出该换成哪个目标语言。
///
/// 返回 `None` 表示"不动"。中文（简/繁）↔ 英语是默认通道，其余任何语言撞到目标时都换回
/// 简体中文（用户的母语）。只翻**目标**语言，源语言永远不动——源语言留在"自动检测"才有
/// 这一步，显式选了源语言是用户的决定，应用只能拦不能改。
pub fn auto_reverse_target(detected: Language, target: Language) -> Option<Language> {
    if detected != target {
        return None;
    }
    match detected {
        Language::ZhHans | Language::ZhHant => Some(Language::En),
        _ => Some(Language::ZhHans),
    }
}

/// 提示条从出现到**完全淡出**的总时长。整条 lifetime 就是一次线性淡出：到点为 0，
/// 不留可点的东西。
///
/// 用户 2026-10-28 变更 I 定的是 500ms（"所有的提示类弹窗都是中心弹窗500ms内淡化消失，
/// 不应该有关闭的操作和按钮"），2026-10-29 改判成 1s（"所有弹窗改为1s淡化消失吧，
/// 500ms太短了"）。**只有这一族受影响**：二次确认框（`Confirm`）要的是用户做一个决定，
/// 自动消失等于替他答"否"，所以它不计时、也不淡。
pub const TOAST_LIFETIME: std::time::Duration = std::time::Duration::from_secs(1);

/// 淡出期间的重绘步长。线性降透明度要连续几帧才看得出"在淡"，一帧 16ms 把
/// `TOAST_LIFETIME` 切成约 60 帧；只在还有提示条在屏上时按这个步长续重绘，
/// 空闲时一次都不续（NFR-2 的空闲 1% 不受影响）。
pub const TOAST_FADE_STEP: std::time::Duration = std::time::Duration::from_millis(16);

#[derive(Debug, Clone)]
pub struct Toast {
    pub tone: Tone,
    pub text: String,
    pub shown_at: std::time::Instant,
}

impl Toast {
    pub fn expired(&self) -> bool {
        self.shown_at.elapsed() >= TOAST_LIFETIME
    }

    /// 此刻该用几分不透明度画自己：出生那一刻 1.0，`TOAST_LIFETIME` 那一刻 0.0，中间线性。
    ///
    /// 这是**画**的一帧里现算的，不是记下来的：一帧只画一次，重绘到了就该更淡。
    /// 越界（年龄为负或远超 lifetime）夹回 0~1，宁可不画也不给出半透明的实心字。
    pub fn opacity(&self) -> f32 {
        let age = self.shown_at.elapsed().as_secs_f32();
        let life = TOAST_LIFETIME.as_secs_f32();
        (1.0 - age / life).clamp(0.0, 1.0)
    }
}

/// 破坏性操作的二次确认（`03-ui-design.md` §1）
/// 三个变体都叫 `Clear*` 是有意的：它们对应界面上三颗"清除"类操作，
/// 名字与按钮文案一一对上比避开前缀更重要
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum Confirm {
    ClearHistory,
    ClearKey,
    /// 「清除所有截图」也要二次确认：那是用户截到的东西，一键就删是另一种事故
    ClearShots,
}

impl Confirm {
    pub const fn prompt(self) -> &'static str {
        match self {
            Confirm::ClearHistory => "清空全部历史？此操作不可撤销。",
            Confirm::ClearShots => "清空全部截图历史？截图里可能有你的账号、单据或聊天内容。",
            Confirm::ClearKey => "清除钥匙串里的 API Key？清除后 AI 通道不可用。",
        }
    }

    pub const fn confirm_label(self) -> &'static str {
        match self {
            Confirm::ClearHistory => "确认清空",
            Confirm::ClearShots => "确认清除截图",
            Confirm::ClearKey => "确认清除",
        }
    }
}

/// 界面上关于 Key 的全部可见信息。**不含明文，也不含任何可还原片段**（FR-4.2）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum KeyState {
    #[default]
    Unknown,
    Missing,
    Configured {
        len: usize,
        hash8: String,
    },
    Error(String),
}

impl KeyState {
    pub const fn configured(&self) -> bool {
        matches!(self, KeyState::Configured { .. })
    }

    pub const fn label(&self) -> &'static str {
        match self {
            KeyState::Configured { .. } => "已配置",
            KeyState::Missing => "未配置",
            KeyState::Unknown => "未知",
            KeyState::Error(_) => "读取失败",
        }
    }
}

/// 一次待派发的任务：文本翻译，或贴图链上的两段（逐块翻译 / 合成），外加截图各支。
///
/// 序号在每一支里都显式放着：派发方（`AppState::submit*`）写它，接活的一方按支取出——
/// 不给它加 `fn id()`，因为除了测试没有第二个调用点，一个只有测试在用的访问器只是装饰。
///
/// 只派生 `Debug`/`Clone`：`ImageTranslate` 带的是停止哨兵，`AtomicBool` 没有 `PartialEq`，
/// 而手写一个"忽略哨兵"的相等只会埋雷——两条哨兵不同、其余全同的任务本来就相等。
#[derive(Debug, Clone)]
pub enum Job {
    Translation {
        id: RequestId,
        engine: EngineKind,
        request: TranslationRequest,
    },
    /// 逐块翻译（REQ-001）。每一块带的就是**将要发给引擎的那一份请求**——定方向是派发方
    /// （[`AppState::submit_pin_translate`]）的事，机翻端点从不认识 `auto`，而工作循环
    /// 拿到的是已经填好具体语言对的请求，它没有第二次改方向的机会。
    ///
    /// `blocks` 是 `(块 id, 该块请求)`，已按块 id 升序：同一份文档必须每次派发出同一个次序，
    /// 否则"重试这一块"的块号会和界面上那行对不上。
    ImageTranslate {
        id: RequestId,
        /// 这批译文落在哪张贴图上（`Pin::document`）
        pin: RequestId,
        engine: EngineKind,
        blocks: Vec<(u32, TranslationRequest)>,
        /// 块边界停止的哨兵：界面置真，工作循环在**下一块请求之前**看到它就收手。
        /// 在飞的那一次请求不撤回（撤不回），所以停止最少要等一块的时间。
        cancel: Arc<AtomicBool>,
    },
    /// 发起一次区域截图（CHG-009 REQ-001）。**只带自增号，不带路径**：历史目录在哪
    /// 只有 `system::capture` 知道，状态层不许知道文件系统的事（REQ-005 那条分层纪律）。
    /// 跑在 `spawn_blocking` 里，因为它会**阻塞到用户画完为止**。
    ///
    /// CHG-010 之后派发口只发 [`Job::Freeze`]，这一支**留着不接线**：新旧两条链的切换
    /// 就改 `submit_capture` 那一格，真机再报缺陷时退回来只动一行（用户 2026-11-09 定的纪律）。
    #[allow(dead_code)]
    ScreenCapture {
        id: RequestId,
        /// 历史文件名的第三段（同 pid 同秒的两次连点靠它分开）
        seq: u64,
    },
    /// 抓每块屏的冻结帧（CHG-010 REQ-101）。`displays` 是要点名的屏序号（`-D`，1 起）；
    /// 序号由 `app.rs` 从 `NSScreen` 列表按位置编出来，回信再按它配对（不做缓存，
    /// 用户当场改分辨率这件事今天实测过）。
    Freeze {
        id: RequestId,
        seq: u64,
        displays: Vec<u8>,
    },
    /// 一张贴图自己的"只识别"（REQ-005）。**不带翻译**：那颗「翻译」是人按的。
    /// `gen` 是派发那一刻的代序号，回信原样带回来，对不上就丢弃。
    PinRecognize {
        pin: RequestId,
        generation: u64,
        path: PathBuf,
    },
    /// 合成一张成品（REQ-003~REQ-006）。整段是一次同步的 CPU 活，走 `spawn_blocking`。
    ImageCompose {
        id: RequestId,
        /// 成品画进哪张贴图（`Pin::product`）
        pin: RequestId,
        source: RgbaImage,
        document: OcrDocument,
        mode: CompositeMode,
    },
}

/// 任务回来的结果。带 `id` 用于丢弃过期结果，带 `engine` 用于错误动作判定。
/// 不派生 `Clone`：`ProviderError` 不是 `Clone`，而结果本来也只该被消费一次。
#[derive(Debug)]
pub enum Outcome {
    Translation {
        id: RequestId,
        engine: EngineKind,
        result: Result<TranslationResult, ProviderError>,
    },
    /// 逐块翻译的一段推进（REQ-009 的"N/M 块"）：20 个块要等十几秒，用户得看见它在走。
    ImageTranslateProgress {
        id: RequestId,
        pin: RequestId,
        done: usize,
        total: usize,
    },
    /// 逐块翻译收工。`results` 是 `(块 id, 该块结果)`：一块 `Err` 只作废它自己，
    /// 其余块的 `Ok` 照样落地（REQ-001 第二条）。`cancelled` 表示这是在块边界停下来的，
    /// 已翻好的那些照样有效、不是半成品（Q-006 裁定"留"）。
    ImageTranslated {
        id: RequestId,
        pin: RequestId,
        results: Vec<(u32, Result<String, ProviderError>)>,
        cancelled: bool,
        elapsed_ms: u64,
    },
    ImageComposed {
        id: RequestId,
        pin: RequestId,
        result: Result<Composed, VisionError>,
        elapsed_ms: u64,
    },
    /// 一次框选回来了。三态（成功/取消/失败）原样带，**不在工作线程里判该干什么**
    ScreenCaptured {
        id: RequestId,
        outcome: crate::system::capture::Outcome,
    },
    /// 冻结帧回来了，按屏一块一张（**临时文件已经在回信之前删掉**）。
    /// 空表 = 一块都没拿到，与 `ScreenCaptured(Failed)` 同一档处理。
    Froze { id: RequestId, frames: Vec<Freeze> },
    /// 某张贴图的识别回来了，带着它那一代的序号。
    /// `admitted` 一起带回来：合成要有底图，而**底图必须是入场缩放之后的那一份**——
    /// 现读文件会拿到未缩放的原始像素，回填的坐标就整体错位（CHG-005 那条同量纲规则）
    PinRecognized {
        pin: RequestId,
        generation: u64,
        result: Result<(Admitted, ice_core::document::OcrDocument), VisionError>,
        elapsed_ms: u64,
    },
}

/// 提示条最多同时显示这么多条，避免把界面挤没
const MAX_TOASTS: usize = 3;

/// 截图那一段自己的状态（CHG-009 REQ-001/002/004，第四条通道）。
///
/// 与识别/翻译/合成三段**并列**而不是嵌在里面：一次框选回来的第一件事是"钉住"，
/// 识别是钉住之后**每张各自**跑的（[`CaptureStage::Pinned`] 之后才有 [`Pin`] 的 stage）。
/// 把两件事塞进一个枚举，就会出现"第三张贴图在识别、第二张的状态被顶掉"这类只对一半的画面。
/// `Cancelled` 单列一态：它既不是失败也不是空闲，那一条路上什么都不该发生（REQ-002）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CaptureStage {
    #[default]
    Idle,
    /// 抓屏子进程还在飞（CHG-009 那一档"等人画完"的语义已经随系统工具退役，
    /// 这里只剩"几百毫秒的抓取"，所以这句话不许再写成"在框选里"）
    Capturing,
    /// 冻结帧拿到了，遮罩开着、用户正在选（CHG-010 REQ-102）
    Frozen,
    /// 拿到图并且钉上了。贴图的细节在 [`AppState::pins`] 里，这一态只说"这一段成功了"
    Pinned,
    /// 工具报错、权限被拒、图坏了——装的是一句能行动的话（`capture` 那侧算好的）
    Failed(String),
    /// 用户 esc 取消：**什么都不做**的那一条，状态行不许说"失败"
    Cancelled,
}

/// 贴图首用提示这一格的状态（CHG-011 REQ-005）。**一次启动弹一次**：
/// - `NotShown`：本进程还没弹过（第一张贴图钉上时才可能转 `Showing`）；
/// - `Showing`：正弹着，`dont_show_again` 是「以后不再提示」那个勾选此刻的状态；
/// - `DoneThisRun`：本进程已经弹过（确认了、或关掉那张图了），此后不再弹。
///
/// 勾了并确认 ⇒ 另写 `config.ui.pin_hint_dismissed = true`（跨启动也不弹）；没勾就确认 ⇒
/// 只到 `DoneThisRun`、不写配置，下次启动再弹一次。esc 关那张图走的是同一条 `Close`，
/// 图关了提示也跟着没（它画在药丸窗里），本进程仍算 `DoneThisRun`——不做两段式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinHint {
    NotShown,
    Showing { dont_show_again: bool },
    DoneThisRun,
}

/// 一张钉在屏幕上的图。
///
/// **状态层只存"它存在、它现在在做什么"**：窗口的建/销、几何、带子开合都在 `system::pin`，
/// 这里存的是视图与派发要读的东西。`document` 为 `None` 时拖选**不产生任何选中**
/// （REQ-005：没长出来就不许假装能选，而不是"选中了一个空字符串"）。
#[derive(Debug, Clone)]
pub struct Pin {
    /// 贴图自己的号（与请求序号分开：窗口关掉之后这个号不再复用）
    pub id: RequestId,
    /// 历史里那张原图
    pub path: PathBuf,
    /// 代序号：每一次**重新派识别**都要 +1。回信带的是派发那一刻的代，
    /// 对不上就丢弃——同一张贴图上"晚到的上一次结果盖掉这一次"是最难查的那种错
    /// （第四条序号互不作废那条纪律的第四处）
    pub recognize_gen: u64,
    /// 这张自己的识别进度。**每张一条**，共用一条就会让第七张的进度盖掉第一张的
    pub stage: RecognitionStage,
    /// 文字层的数据源
    pub document: Option<ice_core::document::OcrDocument>,
    /// 带子开着还是收着（REQ-003：收起 = 窗口缩到图片本身）
    pub band_open: bool,
    /// 这张四周那道窗影开不开（用户 2026-11-12："截图后图片要有阴影…不然不突出"，
    /// 右键可关）。**默认开**：钉上来就要从桌面里凸出来。每张一条，跟着贴图走——
    /// 共用一条就会出"关了第一张的影，第三张也跟着没了"。这是对 CHG-011 Q-002
    /// "with_has_shadow(false) 这一格不动它"的改判：那一格当时说的是药丸的影，
    /// 现在用户要的是图片本体的窗影，两件事，故另立这一格。
    pub shadow: bool,
    /// 这张**该钉在哪儿**：确认那一刻选区的左上角，全局点坐标。
    ///
    /// CHG-009 REQ-001 原话是"选完**原地**钉成一张浮在屏幕上的图"。系统框选那个年代拿不到
    /// 那块矩形（`screencapture -i` 只交回一个文件），所以当时交给了系统默认位；
    /// 自绘遮罩（CHG-010）交得出矩形，这一格就该兑现。
    /// `None` = 这条入口本来就没有位置（「打开上一次」、满了之后 F3 补钉那两张），照旧让系统摆。
    pub at: Option<Pos2>,
    /// 这张的像素（识别回信带回来的那一份）。
    /// 合成要有底图，而底图**必须是屏上正在显示的那一张**——现读文件会读到没换成品之前的旧图。
    pub source: Option<Admitted>,
    /// 这张自己的翻译段与合成段。**每张各持一套**：借别人的在飞标记会出
    /// "这张在翻，那张的按钮因此变灰"这种隔空互相干扰。
    pub translate_stage: TranslateStage,
    pub translate_in_flight: Option<RequestId>,
    pub failed_blocks: Vec<u32>,
    pub compose_in_flight: Option<RequestId>,
    /// 成品。有它就显示它（REQ-003「原地替换」），没有就显示 `source`。
    pub product: Option<Composed>,
    /// 译文落好了、等着合成那一版成品。翻译回信里置真，派发后清。
    ///
    /// 为什么要一格"欠着合成"而不是让回信直接派：`apply_outcome` 只做状态，发任务是帧循环
    /// 的事（REQ-005 那条分层纪律）。
    pub compose_due: bool,
    /// 这一张此刻在文字层上拖选了哪些块（REQ-005）。**每张一条**：选中集合跟着贴图走，
    /// 共用一条就会出"在第一张上选的字，按 C 从第三张复制出去"。
    pub selected: Vec<u32>,
}

impl Pin {
    /// 这份文档里已经带着译文的块数。合成画砸了要把"翻好了几块"如实说回去，
    /// 不能顺手报个 0——那等于抹掉已经花出去的请求。
    pub fn translate_ok_count(&self) -> usize {
        self.document
            .as_ref()
            .map(|document| {
                document
                    .blocks
                    .iter()
                    .filter(|block| block.translation.is_some())
                    .count()
            })
            .unwrap_or(0)
    }

    /// 测试与本模块内部构造用（贴图窗的端到端用例要造一张真贴图）
    #[cfg(test)]
    pub fn for_test(id: RequestId, path: PathBuf) -> Self {
        Self::new(id, path, None)
    }

    fn new(id: RequestId, path: PathBuf, at: Option<Pos2>) -> Self {
        Self {
            id,
            path,
            recognize_gen: 0,
            stage: RecognitionStage::Idle,
            document: None,
            band_open: true,
            shadow: true,
            at,
            source: None,
            translate_stage: TranslateStage::Idle,
            translate_in_flight: None,
            failed_blocks: Vec::new(),
            compose_in_flight: None,
            product: None,
            compose_due: false,
            selected: Vec::new(),
        }
    }

    /// 文字层是不是已经长出来了（REQ-005 的那句"没长出来就不许假装能选"）。
    ///
    /// 判据是"识别完成 **且** 文档真在"，不是"文档非空"——`NoText` 那一档的文档是空的，
    /// 而 `Failed` 那一档根本不该出现可选的暗示。
    pub fn text_ready(&self) -> bool {
        matches!(self.stage, RecognitionStage::Done { .. }) && self.document.is_some()
    }

    /// 按 `C` 会拿到的那段文字。没有选中、或选中的块已经不在文档里 → `None`
    /// （调用方据此退回"复制图像"，**不许交出一个空串**）。
    pub fn selection_text(&self) -> Option<String> {
        let document = self.document.as_ref()?;
        if self.selected.is_empty() {
            return None;
        }
        document.joined_text(&self.selected)
    }
}

/// 发起一次截图时点名的一块屏：`-D` 序号 + 两个坐标系里的位置。
/// 两份位置都要，因为抬层问的是 AppKit（左下原点），认窗问的是系统窗口表（左上原点）；
/// 只存一份就得在用的时候现翻，而翻的时候要主屏高度——那个数不该在遮罩开着期间再去问一次。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenSpec {
    pub display: u8,
    pub appkit: Rect,
    /// 全局左上角（左上原点、y 向下）
    pub origin: Pos2,
}

/// 一块屏的冻结帧。**像素只活在遮罩开着这段时间**：确认时裁完就清，取消时整批清
/// （CHG-010 REQ-102：确认之前盘上不许有东西，那这一份内存就是它唯一的容身之处）。
#[derive(Clone)]
pub struct Freeze {
    /// `-D` 的序号（1 起），与 `system::screen` 那块屏一一对上
    pub display: u8,
    /// 那块屏的**点**尺寸（遮罩窗本地坐标就是 0..这一对）
    pub points: Vec2,
    /// 那块屏在 AppKit 坐标里的矩形（抬层与对位要用，见 `system::cover`）
    pub appkit: Rect,
    /// 那块屏的**全局左上角**（左上原点、y 向下，与系统窗口表同一坐标系）。
    /// 认窗要把表里的全局矩形换回这块屏的本地坐标，靠的就是它；AppKit 那份是左下原点，
    /// 两者差一个主屏高度，混用就会整块偏移（`system::screen::flip` 是唯一换算处）。
    pub origin: Pos2,
    /// 抓回来的物理像素
    pub image: std::sync::Arc<image::RgbaImage>,
}

impl std::fmt::Debug for Freeze {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 一句"这张有多大"就够日志用了；把像素或路径带进日志是本仓明令禁止的
        f.debug_struct("Freeze")
            .field("display", &self.display)
            .field("points", &self.points)
            .finish()
    }
}

/// 一次"冻结后自选"的进行中状态（CHG-010）。**只在遮罩开着这段时间存在**。
#[derive(Debug, Clone)]
pub struct Capture {
    pub frames: Vec<Freeze>,
    /// 选区落在第几块屏（`frames` 的下标）。**选区只在这一块屏内**（Q-004）
    pub screen: usize,
    /// 那块屏本地坐标里的选区状态机
    pub sel: crate::system::selection::Selection,
    /// 抬层那一步这一刻成没成（成=整屏盖住，含菜单栏与 Dock 那两条）。
    /// 不成也要能继续用，但界面 SHALL 说一句（REQ-101：不许静默当成功）。
    pub raised: bool,
    /// 原生那两下（抬层 / 激活）还要不要再重下。**缺陷 D-31~D-34 的病因就在这格**：
    /// 探针里"写完不等于生效"量的是一次性动作，我照它做成每帧重下，于是几张贴图加遮罩
    /// 每帧枚举窗口、重设 frame，把主线程拖死了。改成**头几帧重下、之后停**：
    /// 收敛实测在 0.02 秒内完成（约一两帧），给 12 帧余量足够，之后不再动。
    pub push_frames: u8,
    /// 认窗那一份快照（REQ-108）。`None` = 这一轮还没问过系统；帧循环在遮罩开的第一帧
    /// 问一次就存进来，之后整段不再问（画面已冻结，问第二遍只会认出图里不存在的窗）。
    pub windows: Option<std::sync::Arc<crate::system::windows::Snapshot>>,
    /// 光标此刻在哪块屏（display 序号）。认窗高亮只在那块屏上做，另一块压暗不亮窗
    /// （用户那句「两块屏上的窗都亮了」）。`None` = 问不到光标，退回各遮罩自己的指针判法。
    /// 帧循环每帧问一次窗口服务器，跨屏那一刻变了就叫醒各扇遮罩重画。
    pub lit: Option<u8>,
    /// 这一轮冻结的号（= 回信那个 `request_id`）。**纹理缓存靠它认"这是新一轮的像素"**。
    ///
    /// 缺陷 D-41：缓存原先只按"哪块屏 + 多大"认，而同一块屏每轮抓出来尺寸都一样，
    /// 于是第二轮之后的遮罩把**上一轮**那张定格画面当背景显示（真机读数：遮罩里菜单栏
    /// 那颗钟停在 20:28，而这一刻是 20:51）。裁出去的那张倒是新像素——**所见非所得**，
    /// 比整张都旧更糟，因为用户在框他看不见的东西。
    pub frozen_id: RequestId,
}

/// 原生抬层与激活重下的帧数上限。之后不再下发（见 `Capture::push_frames`）。
pub const PUSH_FRAMES: u8 = 12;

impl Capture {
    /// 这次截图当前那块屏的作业面（给纯函数用的 `Field`）。拿不到就是没有选区可算。
    pub fn field(&self) -> Option<crate::system::selection::Field> {
        let frame = self.frames.get(self.screen)?;
        Some(crate::system::selection::Field::new(
            Rect::from_min_size(Pos2::ZERO, frame.points),
            frame.px(),
        ))
    }
}

impl Freeze {
    /// 那份像素的真实尺寸（点尺寸与像素尺寸不一致时，以像素为准——它是裁的对象）。
    pub fn px(&self) -> Vec2 {
        Vec2::new(self.image.width() as f32, self.image.height() as f32)
    }

    /// 这块屏的作业面。
    pub fn field(&self) -> crate::system::selection::Field {
        crate::system::selection::Field::new(
            Rect::from_min_size(Pos2::ZERO, self.points),
            self.px(),
        )
    }
}

/// 一次截图回信的落点结果，交给帧循环决定要不要开窗与派识别。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureApplied {
    /// 回信过期（被更晚的一次顶掉，或这一轮已经没有在飞标记）：**界面零变化**
    Stale,
    /// 用户取消：不建贴图、其余状态零变化
    Cancelled,
    /// 失败，带那句能行动的话
    Failed(String),
    /// 新建的贴图号。帧循环拿它去开窗（`system::pin::open`）并派"只识别"
    Pinned { pin: RequestId },
    /// 冻结帧已就位、遮罩该开了。`screens` 是拿到的块数（**少于屏数也算成功**：
    /// 少一块就是那一屏没盖住，界面说一句，不假装全屏定格了）
    Frozen { screens: usize },
}

/// 识别段的五态（CHG-005 REQ-006）。贴图每张各持一条（`Pin::stage`）。
///
/// 只覆盖**识别那一段**。翻译段另有一条并列的（[`TranslateStage`]）：
/// 两段各有各的过期序号，晚到的识别结果不许作废翻译段，把它们挤进一个枚举那条界线就没了。
/// 叫 Stage 而不是 Progress——它不承诺百分比。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecognitionStage {
    #[default]
    Idle,
    /// 任务已派发，正在工作线程里跑
    Recognizing,
    Done {
        blocks: usize,
    },
    /// `NoTextFound` 单独一态：它不是失败，把"这张图没字"报成"识别失败"是误导（REQ-002）
    NoText,
    Failed(String),
}

impl RecognitionStage {
    /// 贴图带子那一行的主文案
    pub fn label(&self) -> String {
        match self {
            RecognitionStage::Idle => "空闲".to_string(),
            RecognitionStage::Recognizing => "识别中…".to_string(),
            RecognitionStage::Done { blocks } => format!("完成 {blocks} 块"),
            RecognitionStage::NoText => "未发现文字".to_string(),
            RecognitionStage::Failed(reason) => format!("识别失败：{reason}"),
        }
    }
}

/// 翻译段的五态（CHG-006 REQ-009）。与 [`RecognitionStage`] 并列，两边文案互不挪用。
/// 贴图每张各持一条（`Pin::translate_stage`）。
///
/// 没有"整批失败"这一态：一块失败只作废它自己（REQ-001 第二条），所以段的结局只有
/// "翻完"和"停下"，失败块数作为附注挂在结局上。`Failed` 装的是**发不出去**那一类
/// （定不出方向），不是发出去之后回来的错。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TranslateStage {
    #[default]
    Idle,
    /// 逐块串行进行中：`done` 是已落地的块数，`total` 是这批的块数
    Running {
        done: usize,
        total: usize,
    },
    Done {
        ok: usize,
        failed: usize,
    },
    /// 用户在块边界按了停止。已翻好的块不撤销（Q-006 裁定"留"），所以它不叫 Failed
    Stopped {
        ok: usize,
        failed: usize,
    },
    Failed(String),
}

impl TranslateStage {
    /// 贴图带子那一行的主文案
    pub fn label(&self) -> String {
        match self {
            TranslateStage::Idle => "空闲".to_string(),
            TranslateStage::Running { done, total } => format!("翻译中 {done}/{total} 块"),
            TranslateStage::Done { ok, failed } => tail("完成", *ok, *failed),
            TranslateStage::Stopped { ok, failed } => tail("已停止", *ok, *failed),
            TranslateStage::Failed(reason) => format!("翻译失败：{reason}"),
        }
    }
}

/// "完成 N 块"与"已停止 N 块"共用的一句：有失败块就附在后面，一条都没有就不啰嗦
fn tail(head: &str, ok: usize, failed: usize) -> String {
    if failed == 0 {
        format!("{head} {ok} 块")
    } else {
        format!("{head} {ok} 块，失败 {failed} 块")
    }
}

/// 菜单栏托盘在**这一台机器上**装出来没有（CHG-008：FR-5.4 + FR-5.2 前半句改判）。
///
/// **运行时状态，不落盘**：它是"这一进程这次能不能用"的事实，写进配置就是对着下一台机器
/// 撒谎。三档里 `Unknown` 只代表"还没问过"（第一帧之前），界面按 `Unavailable` 的口径说话，
/// 不许出现"文案说有托盘、实际没有"的混合状态（REQ-006）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayState {
    #[default]
    Unknown,
    /// 状态项已装好，关窗收进菜单栏
    Available,
    /// 装不上（或非 macOS）：关窗退回收进后台，磁贴留在 Dock
    Unavailable,
}

/// 识别失败给人的那句话（REQ-001 的"SHALL NOT 把 OS 错误原文整段甩到界面上"）。
///
/// 分档原则：`import` 与平台降级那两条的 `Display` 本来就是写给用户看的中文，直接给；
/// `Recognition` 里带的是 `code=` / `desc_len=` 这类排障字段（同一句必须留在日志里，
/// 不能进界面），所以换成一句人话。
pub fn image_failure_text(err: &VisionError) -> String {
    match err {
        VisionError::UnsupportedPlatform | VisionError::Decode(_) => err.to_string(),
        VisionError::Recognition(_) => "本机识别失败，请重试或换一张图".to_string(),
        VisionError::TooLarge(_, limit) => format!("图片太大，超出可处理上限 {limit} 像素"),
        VisionError::NoTextFound => "未发现文字".to_string(),
        VisionError::Composite(_) | VisionError::Font(_) | VisionError::NotImplemented(_) => {
            "本机处理失败，请重试".to_string()
        }
    }
}

pub struct AppState {
    pub config: Config,
    pub glossary: Glossary,
    pub tab: Tab,
    /// 当前选中的引擎（工具条 F-04）。与 `config.translation.default_engine`（设置页的默认值）分开：
    /// 随手切换不该改用户的默认配置，但退出时默认值要落盘（FR-1.3）。
    pub engine: EngineKind,
    pub source: String,
    /// `source` 的字符数缓存：只在字节长度变化时重算，避免每帧 O(n)（REQ-002 的 20k 字符要求）
    pub char_count: usize,
    source_bytes: usize,
    pub translation: Option<TranslationResult>,
    pub failure: Option<Failure>,
    pub in_flight: Option<RequestId>,
    pub request_id: RequestId,
    pub toasts: Vec<Toast>,
    /// 后台也看得见的那条浮空提示（用户 2026-11-16）。与 `toasts` 分开存：主窗收着时
    /// `toasts` 画在主窗里没人看，F2"没得翻"那两句改走这一扇独立浮窗（`system::hint`）。
    pub float_hint: Option<Toast>,
    /// 启动时的一次性提示（配置损坏降级、旧术语表导入条数）
    pub startup_notice: Option<String>,
    pub key: KeyState,
    /// 从错误动作跳到设置页后，把焦点定位到 Key 输入
    pub focus_key_input: bool,
    /// 上一帧源文编辑器是否持有焦点。快捷键在绘制之前判定，所以用的是上一帧的事实。
    pub focus_in_source: bool,
    pub history_query: String,
    pub history_rows: Vec<HistoryRecord>,
    pub history_error: Option<String>,
    pub history_count: usize,
    pub term_source: String,
    pub term_target: String,
    pub confirm: Option<Confirm>,
    /// Cmd+Q 走过一次：此后关闭请求不再被拦截（FR-5.2）
    pub quitting: bool,
    /// 窗口当前是否处于隐藏/最小化态
    pub hidden: bool,
    /// 菜单栏托盘这一档在这台机器上生效没有（[`TrayState`]）。帧循环每帧核对后写入，
    /// 关窗去向、唤回入口、设置页文案、心跳四件事都读它——只有一个分叉点，不许各处自己判断。
    pub tray: TrayState,
    /// 窗口当前是不是被"双击标题栏"铺满了，以及**铺的是哪块矩形**（任务 #25）。
    ///
    /// `Some(area)` 就是"正在铺满"，`None` 是没有。存矩形而不是存布尔：铺满的目标尺寸是
    /// 这块屏的可用区算出来的，重发（D-10 护栏）必须拿同一个矩形再算一遍，只存布尔的话
    /// 第二轮就要从配置里现推一个位置，两处推到不同的数就等于护栏判的不是发出去那件事。
    ///
    /// **运行时状态，不落盘**：铺满是这一屏这一次观看的临时姿势，重启后回到 `full_rect`
    /// 才是"记住位置尺寸"（FR-3.6）。它同时是 `mode::capture` 写槽的抑制开关——铺满期间
    /// 实时几何就是整屏，让它写进 `full_rect` 就等于把 D-6/D-10 那一类缺陷换个入口重犯一遍：
    /// 还原时已经没有"铺满之前的尺寸"可回了。
    pub fill: Option<Rect>,
    /// 有配置改动待落盘
    pub config_dirty: bool,
    /// 设置页里正在编辑的 Key 明文缓冲。只在内存，退出前不清空也不落盘、不进日志。
    pub key_input: String,
    // ---- 贴图翻译链的共用序号与哨兵（图片页删除后只剩贴图这一条链在用） ----
    /// 逐块翻译与合成共用的自增序号：派发方写它，回信按它丢弃过期结果。
    /// 贴图自己的在飞标记存在各张 `Pin` 身上，这里只发号。
    pub translate_request_id: RequestId,
    /// 块边界停止的哨兵，随 [`Job::ImageTranslate`] 一起交给工作循环
    pub image_cancel: Arc<AtomicBool>,
    /// ---- 截图与贴图（CHG-009 第四条通道） ----
    pub capture_stage: CaptureStage,
    /// 一次"冻结后自选"的进行中状态（CHG-010）。`None` = 没有遮罩开着。
    pub capture: Option<Capture>,
    /// 这一刻遮罩是不是当前键盘窗（帧循环每帧回读）。**一帧一变，不进 `Capture`**：
    /// 遮罩一关就该忘掉，留着它会让下一次按 esc 被上一轮的判断骗过去
    pub capture_key: bool,
    /// 这次要点名抓哪几块屏（序号 + 两个坐标系里的位置），回信按它配对
    capture_screens: Vec<ScreenSpec>,
    pub capture_request_id: RequestId,
    pub capture_in_flight: Option<RequestId>,
    /// 历史文件名的自增段。**从 1 起**，0 留给"一次都没发过"那个读法
    pub capture_seq: u64,
    /// 屏幕上正钉着的图，按钉上的先后排列（老化与「打开上一次」都读这个次序）
    pub pins: Vec<Pin>,
    /// 下一个贴图号。关掉的不回收——号只往前走，晚到的回信因此不可能落在一张新图上
    pub next_pin_id: RequestId,
    /// F2 队列口径的**回队簿记**（2026-11-16 用户改判 CHG-012 REQ-001/002/003：游标作废）：
    /// 被关掉的贴图路径，队首＝最晚关掉的那张＝下一次 F2 最先补回的。**运行态**：不进
    /// `config.toml`、不持久化（与 [`Self::hotkey_capture`] 同族），重启后为空。
    /// 每条路径至多出现一次（关→开→再关只挪位不叠加）；已被老化/清除抹掉的条目在
    /// [`Self::next_browse`] 选图时顺手丢弃，「清除所有历史」则整本清空。
    pub browse_requeue: Vec<PathBuf>,
    /// 最近一次截图回信的**落点**，由帧循环取走（`take`）后清空：开窗与派"只识别"
    /// 都是碰系统的事，不能发生在状态层里（REQ-005 那条分层纪律）
    pub capture_applied: Option<CaptureApplied>,
    /// **刚建好、还没派过"只识别"的贴图号**。三种来源都往这里放：截图回信、「固定」
    /// 那一格、以及「打开上一次」——帧循环在这一处统一派识别，"钉上去之后做什么"
    /// 就只有一份代码
    pub pins_new: Vec<RequestId>,
    /// 设置页里"正在等你按一个键"的那一格（capture / pin_last / reopen 之一）。
    /// `None` = 没在采集。这是运行态、不进配置：采集态是界面的临时状态，重启后不该还开着
    pub hotkey_capture: Option<&'static str>,
    /// 贴图首用提示这一格（CHG-011 REQ-005）。**运行态、不落盘**：跨启动的"不再提示"由
    /// `config.ui.pin_hint_dismissed` 承载，这一格只管"本次启动弹没弹、勾没勾"。
    pub pin_hint: PinHint,
}

impl AppState {
    pub fn new(
        config: Config,
        glossary: Glossary,
        key: KeyState,
        startup_notice: Option<String>,
    ) -> Self {
        let engine = config.translation.default_engine;
        Self {
            config,
            glossary,
            tab: Tab::Text,
            engine,
            source: String::new(),
            char_count: 0,
            source_bytes: 0,
            translation: None,
            failure: None,
            in_flight: None,
            request_id: 0,
            toasts: Vec::new(),
            float_hint: None,
            startup_notice,
            key,
            focus_key_input: false,
            focus_in_source: false,
            history_query: String::new(),
            history_rows: Vec::new(),
            history_error: None,
            history_count: 0,
            term_source: String::new(),
            term_target: String::new(),
            confirm: None,
            quitting: false,
            hidden: false,
            tray: TrayState::Unknown,
            fill: None,
            config_dirty: false,
            key_input: String::new(),
            translate_request_id: 0,
            image_cancel: Arc::new(AtomicBool::new(false)),
            capture_stage: CaptureStage::Idle,
            capture: None,
            capture_screens: Vec::new(),
            capture_key: false,
            capture_request_id: 0,
            capture_in_flight: None,
            capture_seq: 0,
            pins: Vec::new(),
            next_pin_id: 0,
            browse_requeue: Vec::new(),
            capture_applied: None,
            pins_new: Vec::new(),
            hotkey_capture: None,
            pin_hint: PinHint::NotShown,
        }
    }

    pub fn mode(&self) -> UiMode {
        self.config.ui.mode
    }

    /// 每帧调一次：只在源文真的变了时重算字符数
    pub fn sync_char_count(&mut self) {
        if self.source.len() != self.source_bytes {
            self.source_bytes = self.source.len();
            self.char_count = self.source.chars().count();
        }
    }

    pub fn toast(&mut self, tone: Tone, text: impl Into<String>) {
        self.toasts.push(Toast {
            tone,
            text: text.into(),
            shown_at: std::time::Instant::now(),
        });
        if self.toasts.len() > MAX_TOASTS {
            let extra = self.toasts.len() - MAX_TOASTS;
            self.toasts.drain(0..extra);
        }
    }

    /// 每帧调一次：过期的提示条自动消失。返回是否有变化（决定要不要请求重绘）。
    ///
    /// 这里**没有**"用户点掉一条"那条路（变更 I）：提示条只剩自己会消失这一种结束方式。
    pub fn tick_toasts(&mut self) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|toast| !toast.expired());
        self.toasts.len() != before
    }

    /// 立一条浮空提示（后台可见）。整条替换：同一时刻只浮一句，新的顶掉旧的。
    pub fn set_float_hint(&mut self, tone: Tone, text: impl Into<String>) {
        self.float_hint = Some(Toast {
            tone,
            text: text.into(),
            shown_at: std::time::Instant::now(),
        });
    }

    /// 每帧调一次：到期就收掉。返回是否刚收掉（调用方据此决定要不要再画一帧把窗撤掉）。
    pub fn tick_float_hint(&mut self) -> bool {
        if self.float_hint.as_ref().is_some_and(Toast::expired) {
            self.float_hint = None;
            true
        } else {
            false
        }
    }

    /// 提交一次翻译。返回 `None` 表示"不该发请求"（正在飞行中，或源文为空）。
    pub fn submit(&mut self) -> Option<Job> {
        if self.in_flight.is_some() {
            return None;
        }
        self.sync_char_count();
        if self.source.trim().is_empty() {
            // 空态：不发请求，也不留旧结果误导（REQ-002）
            self.translation = None;
            self.failure = None;
            return None;
        }
        self.reverse_target_when_it_collides();
        self.request_id += 1;
        let id = self.request_id;
        self.in_flight = Some(id);
        self.failure = None;
        Some(Job::Translation {
            id,
            engine: self.engine,
            request: TranslationRequest::new(
                self.source.clone(),
                self.config.translation.lang_pair(),
            ),
        })
    }

    /// REQ-006：源语言留在"自动检测"而检出的语言正好是当前目标语言时，**发请求之前**把目标翻过来。
    ///
    /// 排在 `request_id += 1` 之前，所以 `lang_pair()` 拿到的已经是新语言对，机翻引擎的
    /// 同语言拒绝分支（`machine.rs:160-164`）永远不会被触发，用户也不用再按一次 Enter。
    ///
    /// 显式选定源语言时一个字都不改（REQ-006 第三条）：那是用户自己的选择，
    /// 应用只能拦、不能替他改。检测失败（`None` 或置信度不够）也不改、不拦，按 P4 回退走。
    fn reverse_target_when_it_collides(&mut self) {
        if self.config.translation.source.is_some() {
            return;
        }
        let Some(detection) =
            ice_core::detect(&self.source).filter(|d| d.confidence >= DETECT_CONFIDENCE)
        else {
            return;
        };
        let previous = self.config.translation.target;
        let Some(new_target) = auto_reverse_target(detection.language, previous) else {
            return;
        };
        self.reverse_target(detection.language, new_target);
    }

    /// 检出语言正好是当前目标语言后改目标：设置、最近使用、脏标记、提示四件事一起做。
    ///
    /// 语义页（[`AppState::reverse_target_when_it_collides`]）与图片那一批
    /// （[`AppState::image_direction`]）共用这一条：同一个规则在两处各写一遍，
    /// 迟早会有一处忘了记脏标记或忘了告诉用户。
    fn reverse_target(&mut self, detected: Language, new_target: Language) {
        self.config.translation.target = new_target;
        self.config.translation.remember_target(new_target);
        self.config_dirty = true;
        self.toast(
            Tone::Info,
            format!(
                "检测到{}，已自动改为 {} → {}",
                detected.display_name(),
                detected.display_name(),
                new_target.display_name()
            ),
        );
    }

    /// 应用一个任务结果。返回 `false` 表示这是过期结果，已按各自那一侧的序号丢弃（REQ-002、REQ-006）。
    pub fn apply_outcome(&mut self, outcome: Outcome) -> bool {
        match outcome {
            Outcome::Translation { id, engine, result } => {
                self.apply_translation(id, engine, result)
            }
            // 进度按贴图落地：序号对不上就忽略，晚到的进度不该把已完成的段写回"进行中"
            Outcome::ImageTranslateProgress {
                id,
                pin,
                done,
                total,
            } => {
                if let Some(target) = self.pins.iter_mut().find(|p| p.id == pin) {
                    if target.translate_in_flight == Some(id) {
                        target.translate_stage = TranslateStage::Running { done, total };
                    }
                }
                true
            }
            Outcome::ImageTranslated {
                id,
                pin,
                results,
                cancelled,
                elapsed_ms,
            } => self.apply_pin_translated(pin, id, results, cancelled, elapsed_ms),
            Outcome::ImageComposed {
                id,
                pin,
                result,
                elapsed_ms: _,
            } => self.apply_pin_composed(pin, id, result),
            // 截图与贴图那两条：回信**只改状态**，开窗与派识别发生在帧循环里
            Outcome::Froze { id, frames } => {
                self.capture_applied = Some(self.apply_frozen(id, frames));
                true
            }
            Outcome::ScreenCaptured { id, outcome } => {
                self.capture_applied = Some(self.apply_capture(id, outcome));
                true
            }
            Outcome::PinRecognized {
                pin,
                generation,
                result,
                ..
            } => self.apply_pin_recognized(pin, generation, result),
        }
    }

    fn apply_translation(
        &mut self,
        id: RequestId,
        engine: EngineKind,
        result: Result<TranslationResult, ProviderError>,
    ) -> bool {
        if self.in_flight != Some(id) {
            return false;
        }
        self.in_flight = None;
        match result {
            Ok(result) => {
                self.failure = None;
                self.translation = Some(result);
            }
            Err(err) => {
                self.failure = Some(failure_of(&err, engine));
                // 译文区改为显示原因；源文一个字符都不动（FR-1.7）
                self.translation = None;
            }
        }
        true
    }

    // ---- 区域截图与贴图（CHG-009 REQ-001/002/004/005，第四条通道） ----

    /// 发起一次框选。全局热键命中后调的就是它——入口只留这一个，
    /// 两条路迟早会不一样（图钉那次的教训）。
    ///
    /// 一次框选在飞时**不发起第二次**（REQ-001 末句：后到的直接忽略）。这一条不靠
    /// `capture_gate` 的灰按钮兜住，因为热键那一下不经过按钮。
    pub fn submit_capture(&mut self, screens: Vec<ScreenSpec>) -> Option<Job> {
        if self.capture_gate().is_some() {
            return None;
        }
        if screens.is_empty() {
            // 一块屏都列不出来：这不是"用户取消了"，是这台机器给不出画面（REQ-101 末句）
            self.capture_stage = CaptureStage::Failed(
                capture::Outcome::Failed(capture::Failure::NoPixels)
                    .explain()
                    .unwrap_or("截不出画面")
                    .to_string(),
            );
            return None;
        }
        self.capture_request_id += 1;
        let id = self.capture_request_id;
        self.capture_seq += 1;
        self.capture_in_flight = Some(id);
        self.capture_stage = CaptureStage::Capturing;
        self.capture_screens = screens.clone();
        // 派发口就这一格（切新旧两条链都只改这里）。
        // 2026-11-09 夜里：真机报 D-31~D-34 后曾退回旧路径；本轮按诊断改掉病因
        // （原生重下从"每帧"收成"头几帧"，见 `Capture::push_frames`）再切回新链。
        Some(Job::Freeze {
            id,
            seq: self.capture_seq,
            displays: screens.into_iter().map(|spec| spec.display).collect(),
        })
    }

    /// 冻结帧回来了。**像素只在这里进状态**，遮罩一关就清（确认之前盘上不许有东西）。
    ///
    /// 抓成功的屏按 `submit_capture` 那次给的顺序配回点尺寸；**少几块照样开遮罩**，
    /// 但把少了几块这件事原样交回界面（REQ-101：不许静默当成功）。
    pub fn apply_frozen(&mut self, id: RequestId, grabbed: Vec<Freeze>) -> CaptureApplied {
        if self.capture_in_flight != Some(id) {
            return CaptureApplied::Stale;
        }
        self.capture_in_flight = None;
        if grabbed.is_empty() {
            self.capture = None;
            let text = capture::Outcome::Failed(capture::Failure::NoPixels)
                .explain()
                .unwrap_or("截不出画面")
                .to_string();
            self.capture_stage = CaptureStage::Failed(text.clone());
            return CaptureApplied::Failed(text);
        }
        let specs = self.capture_screens.clone();
        let frames: Vec<Freeze> = grabbed
            .into_iter()
            .filter_map(|frame| {
                let spec = specs.iter().find(|spec| spec.display == frame.display)?;
                Some(Freeze {
                    points: spec.appkit.size(),
                    appkit: spec.appkit,
                    origin: spec.origin,
                    ..frame
                })
            })
            .collect();
        let screens = frames.len();
        self.capture = Some(Capture {
            frozen_id: id,
            frames,
            screen: 0,
            sel: crate::system::selection::Selection::fresh(),
            raised: false,
            push_frames: PUSH_FRAMES,
            windows: None,
            lit: None,
        });
        self.capture_stage = CaptureStage::Frozen;
        CaptureApplied::Frozen { screens }
    }

    /// 记一下这一刻"抬层成没成"（帧循环每帧回读一次）。它只决定界面上那句
    /// "菜单栏与 Dock 这一条我没盖住"要不要说（REQ-101 不许静默当成功）。
    pub fn set_raised(&mut self, raised: bool) {
        if let Some(capture) = self.capture.as_mut() {
            capture.raised = raised;
        }
    }

    /// 遮罩这一刻是不是当前键盘窗。esc 收不收得到就看它，而它**要一两帧才翻成 true**，
    /// 所以每帧回读、不许"发一次激活就当成功"。
    pub fn capture_has_keyboard(&self) -> bool {
        self.capture_key
    }

    /// 遮罩那一帧的选区回写。`screen` 是第几块屏——**按下在哪块屏，选区就归哪块**，
    /// 换屏按下时丢掉旧框（旧框的坐标是另一块屏的本地坐标，留着它会裁到不该裁的地方）
    pub fn apply_selection(&mut self, screen: usize, sel: crate::system::selection::Selection) {
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        if capture.screen != screen {
            capture.screen = screen;
        }
        capture.sel = sel;
    }

    /// 钉住遮罩里确认的那一张（走的是与老流程同一条 `pin_from_history`，不开第二条钉图路）
    pub fn pin_confirmed_capture(&mut self, path: PathBuf) -> RequestId {
        // 「原地」那一格在这里兑现：确认这一刻选区在哪，钉出来就在那儿。
        // 换算只在这一处：那块屏的全局原点 + 选区在那块屏里的左上角。
        // 先算再清 `capture`——清完就没有这份几何了。
        let at = self.capture.as_ref().and_then(|capture| {
            let frame = capture.frames.get(capture.screen)?;
            let sel = capture.sel.rect?;
            Some(Pos2::new(
                frame.origin.x + sel.min.x,
                frame.origin.y + sel.min.y,
            ))
        });
        self.capture = None;
        self.pin_from_history(path, at)
    }

    /// 用掉一帧原生重下的配额。成为键盘窗之后提前收手，不再骚扰窗口服务器。
    pub fn consume_push_frame(&mut self, settled: bool) {
        if let Some(capture) = self.capture.as_mut() {
            if settled {
                capture.push_frames = 0;
            } else {
                capture.push_frames = capture.push_frames.saturating_sub(1);
            }
        }
    }

    /// 正在遮罩里吗。帧循环用它决定要不要每帧重下抬层与激活。
    pub fn capturing(&self) -> bool {
        matches!(self.capture_stage, CaptureStage::Frozen)
    }

    /// 把当前选区裁成一张图（**只在确认之后调**，REQ-102/104）。
    ///
    /// 换算只走 `Field::to_pixels` 那一处；拿不到选区、或那块屏的换算不可信（零尺寸、
    /// 横纵比例分歧），一律 `None` 由调用方说那句"这张没裁出来"——不许裁一张猜测的图。
    pub fn crop_selection(&self) -> Option<image::RgbaImage> {
        let capture = self.capture.as_ref()?;
        let frame = capture.frames.get(capture.screen)?;
        let field = frame.field();
        let sel = capture.sel.rect?;
        let px = field.to_pixels(sel)?;
        let x = px.x.max(0.0) as u32;
        let y = px.y.max(0.0) as u32;
        let w = px.w.max(1.0) as u32;
        let h = px.h.max(1.0) as u32;
        let (width, height) = (frame.image.width(), frame.image.height());
        if x >= width || y >= height {
            return None;
        }
        let cropped = image::imageops::crop_imm(
            frame.image.as_ref(),
            x,
            y,
            w.min(width - x),
            h.min(height - y),
        )
        .to_image();
        Some(cropped)
    }

    /// 遮罩收尾：**两种终点都不留像素在内存里**（那东西一块屏就是几十 MB）。
    /// `pinned=true` 走"已钉住"，false 走"已取消"，`Selected` 那一格由调用方另设。
    pub fn end_capture(&mut self, stage: CaptureStage) {
        self.capture = None;
        self.capture_stage = stage;
    }

    pub fn capture_gate(&self) -> Option<&'static str> {
        if !crate::system::capture::capability().capture {
            return Some("这台机器没有可用的框选截图；可先用系统截图再点「粘贴」");
        }
        if self.capture_in_flight.is_some() || self.capture.is_some() {
            return Some("上一次截图还没完");
        }
        None
    }

    /// 一次框选的回信落地。**这是唯一一处把 `capture` 的三态翻译成状态**：
    /// 取消什么都不做、失败只留一句、成功才建贴图。序号对不上（`Stale`）时
    /// 连状态都不动——晚到的上一次结果盖掉这一次的画面最难查。
    pub fn apply_capture(&mut self, id: RequestId, outcome: capture::Outcome) -> CaptureApplied {
        if self.capture_in_flight != Some(id) {
            return CaptureApplied::Stale;
        }
        self.capture_in_flight = None;
        match outcome {
            capture::Outcome::Cancelled => {
                self.capture_stage = CaptureStage::Cancelled;
                CaptureApplied::Cancelled
            }
            capture::Outcome::Failed(kind) => {
                // 那句能行动的话由 `capture` 算好带回来，这一层不再造第二套文案
                let text = capture::Outcome::Failed(kind)
                    .explain()
                    .unwrap_or("截图失败")
                    .to_string();
                self.capture_stage = CaptureStage::Failed(text.clone());
                CaptureApplied::Failed(text)
            }
            capture::Outcome::Unsupported => {
                let text = capture::Outcome::Unsupported
                    .explain()
                    .unwrap_or("这台机器没有可用的框选截图")
                    .to_string();
                self.capture_stage = CaptureStage::Failed(text.clone());
                CaptureApplied::Failed(text)
            }
            capture::Outcome::Captured(path) => {
                let id = self.push_pin(path, None);
                self.capture_stage = CaptureStage::Pinned;
                self.pins_new.push(id);
                CaptureApplied::Pinned { pin: id }
            }
        }
    }

    /// 记下一张新的贴图，返回它的号。**没有数量上限**（用户 2026-11-16 改口 CHG-009 Q-007：
    /// 6 张那档"找得着"的判断作废，钉多少张由用户自己决定）。
    ///
    /// 号只往前走、不复用：窗口关掉之后那个号不会被下一张顶替，所以任何一条晚到的回信
    /// 都不可能落在一张新贴图上（[`Pin::recognize_gen`] 是同一族纪律的第二层）。
    pub fn push_pin(&mut self, path: PathBuf, at: Option<Pos2>) -> RequestId {
        self.next_pin_id += 1;
        let id = self.next_pin_id;
        self.pins.push(Pin::new(id, path, at));
        // 首用提示（CHG-011 REQ-005）：**第一张**钉上（0→1）、配置里没勾过「不再提示」、
        // 且本进程还没弹过，才弹一次。三个条件少一个都不弹——同一进程钉第二张不再弹。
        if self.pins.len() == 1
            && !self.config.ui.pin_hint_dismissed
            && self.pin_hint == PinHint::NotShown
        {
            self.pin_hint = PinHint::Showing {
                dont_show_again: false,
            };
        }
        id
    }

    /// 「打开上一次截图」那一格：把历史里最新那张钉出来。**取图那一条路不重开**——
    /// 这张已经在盘上，走的还是 `push_pin` + 派识别那一套
    pub fn pin_from_history(&mut self, path: PathBuf, at: Option<Pos2>) -> RequestId {
        let id = self.push_pin(path, at);
        self.pins_new.push(id);
        self.capture_stage = CaptureStage::Pinned;
        id
    }

    /// F2 队列口径的选图（2026-11-16 用户改判 CHG-012 REQ-001/002/003：游标作废）。
    /// `shots` 由帧末读盘递进来（状态层不碰 fs，CHG-009 REQ-005 分层纪律）。两档先后：
    /// 1. **先补关掉的**：回队簿记从队首起，取第一条"还在历史里、且不在屏幕上"的路径；
    ///    被老化/清除抹掉的条目顺手丢弃。最晚关掉的先补回（用户原话"哪个被关闭哪个就
    ///    重新放在最前面"）。
    /// 2. 簿记里没有可补的，才按历史**从新到旧**补第一张不在屏幕上的（[`capture::next_unpinned`]）。
    ///
    /// 两档都落空＝历史里的图全在屏幕上，返回 `None`，调用方提示"没有更多截图了"。
    /// 每张历史图只补一份：已在屏上的路径两档都跳过（单实例）。
    pub fn next_browse(&mut self, shots: &[capture::Shot]) -> Option<PathBuf> {
        while let Some(path) = self.browse_requeue.first().cloned() {
            self.browse_requeue.remove(0);
            let in_history = shots.iter().any(|shot| shot.path == path);
            let on_screen = self.pins.iter().any(|pin| pin.path == path);
            if in_history && !on_screen {
                return Some(path);
            }
        }
        let pinned: Vec<PathBuf> = self.pins.iter().map(|pin| pin.path.clone()).collect();
        capture::next_unpinned(shots, &pinned).map(|shot| shot.path.clone())
    }

    /// 关掉一张：从状态里摘掉，返回是否真摘掉了（`false` = 那个号本来就不在，
    /// 调用方不该为它说"已关闭"）。窗口的关闭命令由帧循环发，这里不碰 egui。
    pub fn close_pin(&mut self, id: RequestId) -> bool {
        let closed = self
            .pins
            .iter()
            .find(|pin| pin.id == id)
            .map(|pin| pin.path.clone());
        let before = self.pins.len();
        self.pins.retain(|pin| pin.id != id);
        let removed = self.pins.len() != before;
        // 首用提示画在药丸窗里：关掉那张图，窗没了提示也跟着没（esc 一下做到两件，REQ-005）。
        // 本进程仍算弹过 ⇒ DoneThisRun，下一张不再弹。只在真摘掉了一张时才算。
        if removed && matches!(self.pin_hint, PinHint::Showing { .. }) {
            self.pin_hint = PinHint::DoneThisRun;
        }
        // 队列口径（改判 CHG-012 REQ-003）：关掉的回队首，F2 先补它。不分来源——
        // F1/F3 钉的也是历史里的截图，关掉同样排到最前。同路径只留一条（去重后插队首）。
        if let Some(path) = closed {
            self.browse_requeue.retain(|p| *p != path);
            self.browse_requeue.insert(0, path);
        }
        removed
    }

    /// 给某张贴图派一次"只识别"。**这一支里一个翻译任务都不许出现**（REQ-001 主干：
    /// 翻译由人按那颗按钮才走）。每次派发都换代，晚到的上一代结果因此作废。
    pub fn submit_pin_recognize(&mut self, pin: RequestId) -> Option<Job> {
        let target = self.pins.iter_mut().find(|p| p.id == pin)?;
        if matches!(target.stage, RecognitionStage::Recognizing) {
            return None;
        }
        target.recognize_gen += 1;
        let generation = target.recognize_gen;
        let path = target.path.clone();
        target.stage = RecognitionStage::Recognizing;
        Some(Job::PinRecognize {
            pin,
            generation,
            path,
        })
    }

    /// 一张贴图的识别回信。`false` = 这一代已经不是当前那一代（贴图换过图或被重派过），
    /// 结果整条丢弃，界面上那一格保持它现在的样子。
    pub fn apply_pin_recognized(
        &mut self,
        pin: RequestId,
        generation: u64,
        result: Result<(Admitted, OcrDocument), VisionError>,
    ) -> bool {
        let Some(pin) = self.pins.iter_mut().find(|p| p.id == pin) else {
            return false;
        };
        if pin.recognize_gen != generation {
            return false;
        }
        match result {
            Ok((admitted, document)) => {
                pin.stage = RecognitionStage::Done {
                    blocks: document.blocks.len(),
                };
                pin.source = Some(admitted);
                pin.document = Some(document);
            }
            // "这张图上真没有可读的字"不是失败：沿用 CHG-005 那一档，不新造（REQ-002）
            Err(VisionError::NoTextFound) => {
                pin.stage = RecognitionStage::NoText;
                pin.document = None;
            }
            Err(err) => {
                pin.stage = RecognitionStage::Failed(image_failure_text(&err));
                pin.document = None;
            }
        }
        true
    }

    /// 带子开合（REQ-003）。返回 `false` = 那个号不在，界面不该装作收起了。
    pub fn set_pin_band(&mut self, pin: RequestId, open: bool) -> bool {
        match self.pins.iter_mut().find(|p| p.id == pin) {
            Some(p) => {
                p.band_open = open;
                true
            }
            None => false,
        }
    }

    /// 右键那颗「图片阴影」勾选翻一下（用户 2026-11-12）。只改状态——真正下到原生窗
    /// 由贴图窗那一帧的 `system::shadow::apply` 幂等重下（与 cover/background 同族）。
    /// 只认真存在的贴图，不存在的号返回 false。
    pub fn toggle_pin_shadow(&mut self, pin: RequestId) -> bool {
        match self.pins.iter_mut().find(|p| p.id == pin) {
            Some(p) => {
                p.shadow = !p.shadow;
                true
            }
            None => false,
        }
    }

    /// 首用提示里那个「以后不再提示」勾选翻一下（CHG-011 REQ-005）。只在正弹着时有效。
    pub fn toggle_pin_hint_dont_show(&mut self) {
        if let PinHint::Showing { dont_show_again } = &mut self.pin_hint {
            *dont_show_again = !*dont_show_again;
        }
    }

    /// 首用提示那颗「知道了」按下（CHG-011 REQ-005）。勾了 ⇒ 写配置 bool（跨启动不再弹）；
    /// 没勾 ⇒ 只到 `DoneThisRun`、**不写配置**，下次启动再弹一次。无论勾没勾，本进程都不再弹。
    /// 返回是否真写了配置（调用方据此决定要不要落盘）。
    pub fn ack_pin_hint(&mut self, dont_show_again: bool) -> bool {
        if !matches!(self.pin_hint, PinHint::Showing { .. }) {
            return false;
        }
        self.pin_hint = PinHint::DoneThisRun;
        if dont_show_again {
            self.config.ui.pin_hint_dismissed = true;
            self.config_dirty = true;
            return true;
        }
        false
    }

    /// 记下一张贴图此刻拖选了哪些块（REQ-005）。**空表也是一次合法写入**——它就是"取消选中"，
    /// 不许把空表当"这一帧没有新信息"忽略掉，否则用户松开一个空拖之后界面还写着有选中。
    pub fn set_pin_selection(&mut self, pin: RequestId, ids: Vec<u32>) -> bool {
        match self.pins.iter_mut().find(|p| p.id == pin) {
            Some(p) => {
                p.selected = ids;
                true
            }
            None => false,
        }
    }

    /// 派发前把语言对落成具体方向（REQ-001 第四条：SHALL NOT 把 `auto` 交给机翻端点）。
    ///
    /// 采样用整批文本拼起来的那一段，而不是第一块：图片里的块常常只有两三个字，
    /// 拿"AI"这一块去判语言会判出一个假的确定性。检出的语言正好撞上当前的目标语言时，
    /// 与语义页同一条规则、同一个处置（[`AppState::reverse_target`]）。
    ///
    /// 返回 `None` = 定不出来（检不出或置信度不够）。这时不发请求：宁可让用户在语言对里
    /// 指定源语言，也不替他猜一个方向然后把二十个块按猜的方向翻完。
    fn image_direction(&mut self, sample: &str) -> Option<LangPair> {
        let pair = self.config.translation.lang_pair();
        if pair.source.is_some() {
            return Some(pair);
        }
        let detected = ice_core::detect(sample)
            .filter(|detection| detection.confidence >= DETECT_CONFIDENCE)?;
        let target = match auto_reverse_target(detected.language, pair.target) {
            Some(new_target) => {
                self.reverse_target(detected.language, new_target);
                new_target
            }
            None => pair.target,
        };
        Some(LangPair::explicit(detected.language, target))
    }

    /// 一张贴图按下「翻译」：识别 → 翻译 → 回填 → 成品原地替换，
    /// 文档、在飞标记与成品都存在这张自己身上。
    ///
    /// 三道前置门（没在飞、有可翻的块、定得出方向）；第三道不过要把话留在
    /// **这张的**翻译段上——他正看着的是屏上那张。
    pub fn submit_pin_translate(&mut self, pin: RequestId) -> Option<Job> {
        // 顺序要紧：先在不_mut_借那一张的情况下把方向与号算出来，再回头写它自己的状态
        let position = self.pins.iter().position(|p| p.id == pin)?;
        if self.pins[position].translate_in_flight.is_some() {
            return None;
        }
        let pending: Vec<(u32, String)> = self.pins[position]
            .document
            .as_ref()?
            .enabled_blocks()
            .map(|block| (block.id, block.text.clone()))
            .collect();
        if pending.is_empty() {
            return None;
        }
        let sample = pending
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let engine = self.engine;
        let cancel = Arc::clone(&self.image_cancel);
        let pair = match self.image_direction(&sample) {
            Some(pair) => pair,
            None => {
                self.pins[position].translate_stage =
                    TranslateStage::Failed(NO_DIRECTION_TEXT.to_string());
                return None;
            }
        };
        let blocks = pending
            .into_iter()
            .map(|(id, text)| (id, TranslationRequest::new(text, pair)))
            .collect::<Vec<_>>();
        self.translate_request_id += 1;
        let id = self.translate_request_id;
        let target = &mut self.pins[position];
        target.translate_in_flight = Some(id);
        target.translate_stage = TranslateStage::Running {
            done: 0,
            total: blocks.len(),
        };
        target.failed_blocks.clear();
        // 重翻一次，上一版成品当场作废：留着它就是"旧图配新译文"
        target.product = None;
        target.compose_due = false;
        Some(Job::ImageTranslate {
            id,
            pin,
            engine,
            blocks,
            cancel,
        })
    }

    /// 落地一张贴图的译文。`false` = 这一批已经不是它了（贴图被关掉或又重派过）。
    fn apply_pin_translated(
        &mut self,
        pin: RequestId,
        id: RequestId,
        results: Vec<(u32, Result<String, ProviderError>)>,
        cancelled: bool,
        // 耗时这一格留给贴图带子的完整版（T-009 那格要说"识别中/翻好 5 块"，
        // 秒数与它同列）；现在不存进 `Pin` 是为了不出现"存了但没人画"的字段
        _elapsed_ms: u64,
    ) -> bool {
        let Some(target) = self.pins.iter_mut().find(|p| p.id == pin) else {
            return false;
        };
        if target.translate_in_flight != Some(id) {
            return false;
        }
        target.translate_in_flight = None;
        let mut ok = 0;
        if let Some(document) = target.document.as_mut() {
            ok = land_translations(document, results, &mut target.failed_blocks);
        }
        let failed = target.failed_blocks.len();
        target.translate_stage = if cancelled {
            TranslateStage::Stopped { ok, failed }
        } else {
            TranslateStage::Done { ok, failed }
        };
        // 翻成了哪怕一块才欠一次合成：全失败时屏上该留着原图，不是去画一张没有字的成品
        target.compose_due = ok > 0;
        true
    }

    /// 帧循环取走"该合成成品的贴图"。译文落好不等于当场画图——发任务是这一层之外的事。
    pub fn take_due_pin_composes(&mut self) -> Vec<Job> {
        let mut jobs = Vec::new();
        let mut serial = self.translate_request_id;
        for target in self.pins.iter_mut() {
            if !target.compose_due {
                continue;
            }
            target.compose_due = false;
            let (Some(source), Some(document)) = (target.source.as_ref(), target.document.as_ref())
            else {
                continue;
            };
            serial += 1;
            target.compose_in_flight = Some(serial);
            jobs.push(Job::ImageCompose {
                id: serial,
                pin: target.id,
                source: source.image.clone(),
                document: document.clone(),
                mode: CompositeMode::default(),
            });
        }
        self.translate_request_id = serial;
        jobs
    }

    /// 成画即换：只换这一张贴图显示的那一个，历史里的原图一个字都不动（Q-008）。
    /// 失败保留原图 + 一句原因（REQ-003 末句）——**贴图不许变没**。
    fn apply_pin_composed(
        &mut self,
        pin: RequestId,
        id: RequestId,
        result: Result<Composed, VisionError>,
    ) -> bool {
        // 耗时不进 `Pin`：贴图那一格的进度只写"到哪一步"，秒数是状态行那一列的事
        let Some(target) = self.pins.iter_mut().find(|p| p.id == pin) else {
            return false;
        };
        if target.compose_in_flight != Some(id) {
            return false;
        }
        target.compose_in_flight = None;
        match result {
            Ok(composed) => target.product = Some(composed),
            // 画砸了：屏上还是原来那张，失败原因挂在这张的翻译段旁边那一格
            Err(err) => {
                target.translate_stage = TranslateStage::Done {
                    ok: target.translate_ok_count(),
                    failed: target.failed_blocks.len(),
                };
                target.product = None;
                tracing::warn!(pin, error = image_failure_text(&err), "贴图成品没画成");
            }
        }
        true
    }

    /// 切换引擎：对当前源文自动重译（FR-1.3）
    pub fn set_engine(&mut self, kind: EngineKind) -> Option<Job> {
        self.engine = kind;
        self.submit()
    }

    /// 切换目标语言：记住最近使用 + 自动重译（FR-1.4、FR-1.3）
    pub fn set_target(&mut self, lang: Language) -> Option<Job> {
        self.config.translation.target = lang;
        self.config.translation.remember_target(lang);
        self.config_dirty = true;
        self.submit()
    }

    pub fn set_source_language(&mut self, lang: Option<Language>) {
        self.config.translation.source = lang;
        self.config_dirty = true;
    }

    /// 另一个形态——**只算不改**。
    ///
    /// 改 mode 这件事归 `mode::apply`：它必须先按**旧**形态把窗口实时几何记回源槽，再把 mode
    /// 翻过去。反过来做（先翻再记）会让 `capture` 把切换前的尺寸写进**目标形态**的槽，随后
    /// `target_size` 读到的正是这个实时尺寸，`InnerSize` 退化成"请求当前尺寸"的空操作——
    /// 缺陷 D-13（用户 2026-10-28："不去碰一下边框，他就无法变为悬浮前的大小"）。
    pub fn next_mode(&self) -> UiMode {
        match self.config.ui.mode {
            UiMode::Full => UiMode::Compact,
            UiMode::Compact => UiMode::Full,
        }
    }

    pub fn set_mode(&mut self, mode: UiMode) {
        self.config.ui.mode = mode;
        self.config_dirty = true;
    }

    pub fn toggle_topmost(&mut self) -> bool {
        self.config.ui.always_on_top = !self.config.ui.always_on_top;
        self.config_dirty = true;
        self.config.ui.always_on_top
    }

    pub fn mark_config_dirty(&mut self) {
        self.config_dirty = true;
    }

    /// 错误动作：跳到设置页并把焦点定位到 Key 输入
    pub fn go_to_key_settings(&mut self) {
        self.tab = Tab::Settings;
        self.focus_key_input = true;
    }

    /// 错误动作：改用机翻并立即重译（FR-1.7）
    pub fn switch_to_machine(&mut self) -> Option<Job> {
        self.set_engine(EngineKind::Machine)
    }

    /// Esc：清空输入行以及它对应的结果（REQ-005）。
    ///
    /// 不发请求、不退出、不收窗。在飞的任务一并作废：`in_flight` 置空之后，
    /// 那次结果回来时会被 `apply_outcome` 当成过期结果丢掉——否则用户清空了输入行，
    /// 半秒后译文又自己冒出来。
    pub fn clear_input(&mut self) {
        self.source.clear();
        self.translation = None;
        self.failure = None;
        self.in_flight = None;
        self.sync_char_count();
    }

    pub fn visible_languages(&self) -> Vec<Language> {
        self.config.translation.visible_languages()
    }

    // ---- 历史（FR-4.4） ----

    pub fn reload_history(&mut self, store: &HistoryStore) {
        let query = self.history_query.trim();
        let loaded = if query.is_empty() {
            store.recent(HISTORY_CAPACITY)
        } else {
            store.search(query, HISTORY_CAPACITY)
        };
        match loaded {
            Ok(rows) => {
                self.history_count = rows.len();
                self.history_rows = rows;
                self.history_error = None;
            }
            Err(err) => {
                self.history_rows.clear();
                self.history_error = Some(format!("历史不可用：{err}"));
            }
        }
    }

    /// 翻译成功后追加一条历史（追加写，不重写整文件）
    pub fn record_history(&mut self, store: &HistoryStore, result: &TranslationResult) {
        let record = HistoryRecord {
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or_default(),
            engine: result.engine,
            detected: result.detected,
            target: self.config.translation.target,
            source_text: self.source.clone(),
            result_text: result.text.clone(),
            elapsed_ms: result.elapsed_ms,
        };
        if let Err(err) = store.append(&record) {
            self.history_error = Some(format!("历史写入失败：{err}"));
            return;
        }
        self.reload_history(store);
    }

    pub fn clear_history(&mut self, store: &HistoryStore) {
        match store.clear() {
            Ok(()) => {
                self.history_rows.clear();
                self.history_count = 0;
                self.history_error = None;
                self.toast(Tone::Info, "历史已清空");
            }
            Err(err) => self.history_error = Some(format!("清空历史失败：{err}")),
        }
    }

    /// 点一条历史：只回填内容，不自动发请求（REQ-006）
    pub fn load_history_row(&mut self, index: usize) -> bool {
        // 先克隆出来：下面要改 `self.source`，不能同时借着 `self.history_rows`
        let Some(row) = self.history_rows.get(index).cloned() else {
            return false;
        };
        self.source = row.source_text.clone();
        self.source_bytes = 0;
        self.sync_char_count();
        self.translation = Some(TranslationResult {
            text: row.result_text.clone(),
            engine: row.engine,
            detected: row.detected,
            elapsed_ms: row.elapsed_ms,
            cache_hit: false,
            segments: 1,
        });
        self.failure = None;
        self.tab = Tab::Text;
        true
    }

    // ---- 术语（FR-4.5） ----

    pub fn add_term(&mut self) -> Option<Term> {
        let source = self.term_source.trim();
        let target = self.term_target.trim();
        if source.is_empty() || target.is_empty() {
            return None;
        }
        let term = Term::new(source, target);
        self.glossary.upsert(term.clone());
        self.glossary.write_to_config(&mut self.config);
        self.term_source.clear();
        self.term_target.clear();
        self.config_dirty = true;
        Some(term)
    }

    pub fn remove_term(&mut self, source: &str) -> bool {
        let removed = self.glossary.remove(source);
        if removed {
            self.glossary.write_to_config(&mut self.config);
            self.config_dirty = true;
        }
        removed
    }

    /// 状态条（F-11）：就绪 / 错误 / 未配置三态
    pub fn status_text(&self, ai_available: bool) -> String {
        let engine_state = if ai_available {
            "机翻可用 · AI 可用"
        } else {
            "机翻可用 · AI 未配置"
        };
        let head = if let Some(failure) = &self.failure {
            format!("错误：{}", failure.message)
        } else if self.in_flight.is_some() {
            "翻译中…".to_string()
        } else {
            "就绪".to_string()
        };
        format!(
            "{head} · {engine_state} · 历史 {} 条 · Cmd+Enter 翻译",
            self.history_count
        )
    }
}

/// 把一批"块号 → 译文"落到某一份文档上，顺手销账失败台账。
///
/// 丢的那一支尤其容易写歪：块不在了 = 用户在这批请求还在网上跑
/// 的时候把它删了，结果无处落，只能丢，**不许**因此说"已翻好"。
fn land_translations(
    document: &mut OcrDocument,
    results: Vec<(u32, Result<String, ProviderError>)>,
    failed: &mut Vec<u32>,
) -> usize {
    let mut ok = 0;
    for (block_id, result) in results {
        failed.retain(|kept| *kept != block_id);
        match result {
            Ok(text) => {
                if let Some(block) = document.blocks.iter_mut().find(|b| b.id == block_id) {
                    block.translation = Some(text);
                    ok += 1;
                }
            }
            Err(_) => failed.push(block_id),
        }
    }

    ok
}

#[cfg(test)]
mod tests {
    use ice_core::document::{PixelRect, TextBlock};

    /// 截图这十条用例共用的一个干净状态（构造与 `app_with_source` 同一份，只是不塞源文）。
    fn capture_state() -> AppState {
        AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        )
    }

    /// 贴图那一支的识别回信同时带底图与文档（合成用的底图必须是入场那一份）
    /// 一次截图能点名的屏：给两块（主屏 1920×1080 + 左置 2240×1400，本机实测形状）。
    /// 测试里不写 `Vec::new()`：那样测的是"一块屏"，而负坐标那块屏才是这包容易错的地方。
    fn two_screens() -> Vec<ScreenSpec> {
        vec![
            ScreenSpec {
                display: 1,
                appkit: Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0)),
                origin: Pos2::ZERO,
            },
            ScreenSpec {
                display: 2,
                appkit: Rect::from_min_size(Pos2::new(-2240.0, -320.0), Vec2::new(2240.0, 1400.0)),
                // AppKit 左下原点翻成全局左上原点：主屏高 1080，这块屏高 1400、底边在 -320
                origin: Pos2::new(-2240.0, 0.0),
            },
        ]
    }

    fn pin_doc() -> OcrDocument {
        let mut document = OcrDocument::empty(600, 400);
        document.blocks = vec![TextBlock::new(
            0,
            "hello world",
            PixelRect::new(10.0, 10.0, 200.0, 18.0),
            0.9,
        )];
        document
    }

    /// 让一张贴图走到"识别已回来"的状态
    fn recognized_pin(st: &mut AppState) -> u64 {
        st.push_pin(PathBuf::from("/tmp/history/pin.png"), None)
    }

    /// CHG-012 改判队列口径（2026-11-16 用户拍板，游标作废），纯状态层（不碰盘，
    /// shots 由调用方读盘递进来）：
    /// - 从新到旧补**不在屏幕上**的，每张只补一份（单实例）；全在屏上＝`None`；
    /// - 关掉哪张哪张回队首，下一次 F2 先补它；连关几张时**最晚关的先补**；
    /// - 补回即销账；同路径反复关只留一条；被老化/清除抹掉的条目选图时顺手丢弃；
    /// - F1 那条路（push_pin）钉的图关掉同样回队首；关一个不存在的号不动簿记。
    #[test]
    fn f2_queue_restores_closed_first_and_never_duplicates_a_pin() {
        let mut st = capture_state();
        assert!(st.browse_requeue.is_empty(), "新建状态簿记为空");
        let shots = vec![
            capture::Shot {
                path: PathBuf::from("/shots/c.png"),
                mtime: 30,
            },
            capture::Shot {
                path: PathBuf::from("/shots/b.png"),
                mtime: 20,
            },
            capture::Shot {
                path: PathBuf::from("/shots/a.png"),
                mtime: 10,
            },
        ];
        let (a_path, b_path, c_path) = (
            PathBuf::from("/shots/a.png"),
            PathBuf::from("/shots/b.png"),
            PathBuf::from("/shots/c.png"),
        );

        // 从新到旧依次补：c、b、a；全在屏上再按＝没有更多
        let path = st.next_browse(&shots).unwrap();
        let c = st.pin_from_history(path, None);
        assert_eq!(st.pins.iter().find(|p| p.id == c).unwrap().path, c_path);
        let path = st.next_browse(&shots).unwrap();
        let b = st.pin_from_history(path, None);
        assert_eq!(st.pins.iter().find(|p| p.id == b).unwrap().path, b_path);
        let path = st.next_browse(&shots).unwrap();
        let a = st.pin_from_history(path, None);
        assert_eq!(st.pins.iter().find(|p| p.id == a).unwrap().path, a_path);
        assert!(
            st.next_browse(&shots).is_none(),
            "历史全在屏上＝没有更多截图了，不许钉出第二份"
        );
        assert_eq!(st.pins.len(), 3);

        // 关掉 b：回队首，下一次 F2 补回 b（用户真机报的那一档）
        assert!(st.close_pin(b));
        assert_eq!(st.browse_requeue, vec![b_path.clone()]);
        let again = st.next_browse(&shots).unwrap();
        assert_eq!(again, b_path, "关掉的先补回，而不是说没有更多");
        let b2 = st.pin_from_history(again, None);
        assert!(b2 != b, "补回是新号，不复用旧号");
        assert!(st.browse_requeue.is_empty(), "补回即销账");
        assert!(st.next_browse(&shots).is_none(), "补回后又全在屏上");

        // 同路径反复关只留一条：关 b2 → 再钉回 → 再关，簿记仍只有一条 b
        st.close_pin(b2);
        let b3 = st.pin_from_history(b_path.clone(), None);
        st.close_pin(b3);
        assert_eq!(st.browse_requeue, vec![b_path.clone()], "去重后插队首");
        let path = st.next_browse(&shots).unwrap();
        st.pin_from_history(path, None);
        assert!(st.browse_requeue.is_empty(), "把 b 补回屏上，簿记清空");

        // 连关 c、a：最晚关的 a 排队首，补回序 a → c
        st.close_pin(c);
        st.close_pin(a);
        assert_eq!(st.browse_requeue, vec![a_path.clone(), c_path.clone()]);
        let path = st.next_browse(&shots).unwrap();
        st.pin_from_history(path, None);
        assert_eq!(st.browse_requeue, vec![c_path.clone()], "先补最晚关的 a");
        let path = st.next_browse(&shots).unwrap();
        st.pin_from_history(path, None);
        assert!(st.browse_requeue.is_empty());
        assert!(st.next_browse(&shots).is_none());

        // 关一个不存在的号：不动簿记
        let snapshot = st.browse_requeue.clone();
        assert!(!st.close_pin(999));
        assert_eq!(st.browse_requeue, snapshot);

        // F1 那条路钉的图关掉同样回队首
        let f1 = st.push_pin(PathBuf::from("/shots/f1.png"), None);
        st.close_pin(f1);
        assert_eq!(
            st.browse_requeue.first().unwrap(),
            &PathBuf::from("/shots/f1.png")
        );

        // 簿记里的路径被老化/清除抹掉（不在 shots 里）：选图时丢弃，不报错不卡死
        st.browse_requeue = vec![a_path.clone()];
        st.close_pin(st.pins.iter().find(|p| p.path == a_path).unwrap().id);
        let aged: Vec<capture::Shot> = shots
            .iter()
            .filter(|s| s.path != a_path)
            .map(|s| capture::Shot {
                path: s.path.clone(),
                mtime: s.mtime,
            })
            .collect();
        assert!(
            st.next_browse(&aged).is_none(),
            "a 已被抹掉、其余全在屏上＝没有更多；失效条目顺手清掉"
        );
        assert!(st.browse_requeue.is_empty(), "抹掉的条目不许留在簿记里");
    }

    /// 每张贴图各持一套链上状态：**一张在翻，另一张的批照样派得出**；
    /// 发给第一张的回信也不许改第二张的段状态。这是"各窗自持状态"最直接的后果。
    #[test]
    fn two_pins_translate_independently_and_replies_land_on_their_own() {
        let mut st = capture_state();
        let first = recognized_pin(&mut st);
        let second = st.push_pin(PathBuf::from("/tmp/history/pin2.png"), None);
        let (admitted, _) = pixels_and_doc(600, 400);
        for pin in &mut st.pins {
            pin.source = Some(admitted.clone());
            pin.document = Some(pin_doc());
        }

        let first_job = st.submit_pin_translate(first).expect("第一张该能派一批");
        let second_job = st
            .submit_pin_translate(second)
            .expect("第一张在飞，第二张该照派得出");
        match (&first_job, &second_job) {
            (
                Job::ImageTranslate {
                    id: first_id,
                    pin: first_pin,
                    ..
                },
                Job::ImageTranslate {
                    id: second_id,
                    pin: second_pin,
                    ..
                },
            ) => {
                assert_eq!(*first_pin, first);
                assert_eq!(*second_pin, second);
                assert_ne!(first_id, second_id, "两批共用一个号，回信就分不开是谁的");
            }
            other => panic!("派发的必须是逐块翻译：{other:?}"),
        }

        // 第一批的回信落地：只有第一张的段状态变
        let first_id = match first_job {
            Job::ImageTranslate { id, .. } => id,
            _ => unreachable!(),
        };
        assert!(st.apply_outcome(Outcome::ImageTranslated {
            id: first_id,
            pin: first,
            results: vec![(0, Ok("你好世界".to_string()))],
            cancelled: false,
            elapsed_ms: 3,
        }));
        assert_eq!(
            st.pins[0].translate_stage,
            TranslateStage::Done { ok: 1, failed: 0 }
        );
        assert_eq!(
            st.pins[1].translate_stage,
            TranslateStage::Running { done: 0, total: 1 },
            "第一张的回信把第二张的段状态改了"
        );
        assert!(st.pins[1].translate_in_flight.is_some());
    }

    /// 整条链走完：译文落进这张自己的文档 → 欠一次合成 → 成品**原地替换**。
    /// 历史目录里那张原图一个字节都不动（Q-008：原图是唯一真相源）。
    #[test]
    fn the_pin_chain_replaces_the_picture_in_place_and_owes_one_compose() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("1-1-1.png");
        image::RgbImage::from_pixel(60, 40, image::Rgb([1, 2, 3]))
            .save(&file)
            .unwrap();
        let before = std::fs::read(&file).unwrap();

        let mut st = capture_state();
        let pin = st.push_pin(file.clone(), None);
        let (admitted, document) = pixels_and_doc(60, 40);
        let mut document = document;
        document.blocks = pin_doc().blocks;
        st.pins[0].source = Some(admitted);
        st.pins[0].document = Some(document);

        let job = st.submit_pin_translate(pin).expect("该派出一批翻译");
        let batch = match job {
            Job::ImageTranslate { id, .. } => id,
            other => panic!("{other:?}"),
        };
        assert!(st.apply_outcome(Outcome::ImageTranslated {
            id: batch,
            pin,
            results: vec![(0, Ok("你好世界".to_string()))],
            cancelled: false,
            elapsed_ms: 4,
        }));
        assert_eq!(
            st.pins[0].translate_stage,
            TranslateStage::Done { ok: 1, failed: 0 }
        );
        assert_eq!(
            st.pins[0].document.as_ref().unwrap().blocks[0]
                .translation
                .as_deref(),
            Some("你好世界"),
            "译文该落在这张贴图自己的文档上"
        );

        // 译文落好 → 欠一次合成，且带的是这张的底图与文档
        let due = st.take_due_pin_composes();
        assert_eq!(due.len(), 1, "该派一次成品合成");
        let compose_id = match &due[0] {
            Job::ImageCompose { id, pin: owner, .. } => {
                assert_eq!(*owner, pin);
                *id
            }
            other => panic!("{other:?}"),
        };
        assert!(!st.pins[0].compose_due, "派过就该销账，不许每帧重复派");

        let product = a_composed(1);
        assert!(st.apply_outcome(Outcome::ImageComposed {
            id: compose_id,
            pin,
            result: Ok(product),
            elapsed_ms: 5,
        }));
        assert!(st.pins[0].product.is_some(), "成品没换上");
        assert!(
            matches!(
                crate::system::pin::displayed(&st.pins[0]),
                Some(crate::system::pin::Display::Owned(_))
            ),
            "屏上该是成品"
        );
        assert_eq!(std::fs::read(&file).unwrap(), before, "原图被动过了");
    }

    /// 成品画砸了：**屏上还是原来那张**，加一句原因——不许把图变没（REQ-003 末句）。
    #[test]
    fn a_failed_product_keeps_the_picture_instead_of_blanking_it() {
        let mut st = capture_state();
        let pin = recognized_pin(&mut st);
        let (admitted, document) = pixels_and_doc(60, 40);
        st.pins[0].source = Some(admitted);
        st.pins[0].document = Some(document);
        st.pins[0].compose_due = true;
        let compose_id = match st.take_due_pin_composes().remove(0) {
            Job::ImageCompose { id, .. } => id,
            other => panic!("{other:?}"),
        };
        assert!(st.apply_outcome(Outcome::ImageComposed {
            id: compose_id,
            pin,
            result: Err(ice_vision::error::VisionError::Composite(
                "测试：画不出来".into()
            )),
            elapsed_ms: 2,
        }));
        assert!(st.pins[0].product.is_none());
        // 底图还在，屏上就还有东西
        assert!(matches!(
            crate::system::pin::displayed(&st.pins[0]),
            Some(crate::system::pin::Display::Borrowed(_))
        ));
    }

    /// 贴图关掉之后，它那些还在飞的回信一律丢弃——尤其不许落到"后来那张"上
    /// （号只往前走不复用，这一条是它的对偶：拿旧号来找也不会找到新图）。
    #[test]
    fn replies_for_a_closed_pin_are_dropped() {
        let mut st = capture_state();
        let pin = recognized_pin(&mut st);
        let (admitted, _) = pixels_and_doc(60, 40);
        st.pins[0].source = Some(admitted);
        // 得给一份**有字**的文档：空文档本来就该派不出翻译，那样这条用例什么也没锁
        st.pins[0].document = Some(pin_doc());
        let batch = match st.submit_pin_translate(pin).unwrap() {
            Job::ImageTranslate { id, .. } => id,
            other => panic!("{other:?}"),
        };
        assert!(st.close_pin(pin));
        assert!(!st.apply_outcome(Outcome::ImageTranslated {
            id: batch,
            pin,
            results: vec![(0, Ok("没了对象".to_string()))],
            cancelled: false,
            elapsed_ms: 1,
        }));
        // 派一次翻译，不会凭空造出一张贴图
        assert!(st.pins.is_empty());
        assert!(st.submit_pin_translate(pin).is_none());
    }

    /// CHG-013：截图任务**不再带"收起哪些窗"那份账**（改口 CHG-009 REQ-010）。发起框选
    /// 不收起自家窗，任务里只剩号与屏序号；"不再发 Visible(false)"由 app.rs 帧末队列断言守。
    #[test]
    fn the_capture_job_no_longer_carries_a_hide_list() {
        fn id_of(job: &Job) -> RequestId {
            match job {
                Job::ScreenCapture { id, .. } | Job::Freeze { id, .. } => *id,
                other => panic!("派出的必须是截图任务：{other:?}"),
            }
        }

        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/a.png"), None);
        st.push_pin(PathBuf::from("/tmp/history/b.png"), None);
        let job = st.submit_capture(two_screens()).expect("空闲时该发得出");
        // 派发口只发自绘冻结这一支，且结构上已经没有 hidden 那一格，只带号与屏序号
        let Job::Freeze { displays, .. } = &job else {
            panic!("派发口只发自绘冻结这一支：{job:?}");
        };
        assert_eq!(displays.len(), 2, "两块屏都要点名");

        // 主窗本来就收着（托盘那一档）也照样发得出：唤醒不分这一格（CHG-013 保留唤醒）。
        // 先让上一次取消落地——在飞时本来就该拒
        let first = st.apply_capture(id_of(&job), capture::Outcome::Cancelled);
        assert!(matches!(first, CaptureApplied::Cancelled));
        st.hidden = true;
        let job = st
            .submit_capture(two_screens())
            .expect("上一次落地之后该再发得出");
        assert!(matches!(job, Job::Freeze { .. }));
    }

    /// **每一轮冻结都带自己的号**，纹理缓存认的就是这一格（缺陷 D-41 的形状核查）。
    ///
    /// 真机读数：第二次之后的遮罩把**上一轮**那张定格画面当背景显示（菜单栏那颗钟停在
    /// 20:28，而这一刻是 20:51），裁出去的那张却是新像素——用户在框他看不见的东西，
    /// 比整张都旧更难查。病因是缓存拿"哪块屏 + 多大"当内容的凭证，而同一块屏每轮尺寸都一样。
    #[test]
    fn each_capture_carries_its_own_generation_for_the_texture_cache() {
        let frame = |display: u8| Freeze {
            display,
            points: Vec2::new(1920.0, 1080.0),
            appkit: Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0)),
            origin: Pos2::ZERO,
            image: Arc::new(image::RgbaImage::from_pixel(
                16,
                16,
                image::Rgba([7, 7, 7, 255]),
            )),
        };
        let mut st = capture_state();
        let first = st.submit_capture(two_screens()).expect("第一次该发得出");
        // 号从派发出的那条任务里取（两条路都认，切换派发口时这条测试不许只有一支能过）
        let (Job::ScreenCapture { id: id1, .. } | Job::Freeze { id: id1, .. }) = &first else {
            unreachable!()
        };
        let id1 = *id1;
        st.apply_frozen(id1, vec![frame(1)]);
        assert_eq!(
            st.capture.as_ref().expect("冻结已落地").frozen_id,
            id1,
            "这一轮的号就是回信那个 request_id"
        );

        // 上一轮取消落地之后再来一次：号必须换，缓存才认得出"这是新一轮的像素"
        st.end_capture(CaptureStage::Cancelled);
        let second = st
            .submit_capture(two_screens())
            .expect("上一次落地之后该再发得出");
        let (Job::ScreenCapture { id: id2, .. } | Job::Freeze { id: id2, .. }) = &second else {
            unreachable!()
        };
        let id2 = *id2;
        st.apply_frozen(id2, vec![frame(1)]);
        let second_gen = st.capture.as_ref().expect("第二轮也冻结了").frozen_id;
        assert_eq!(second_gen, id2);
        assert_ne!(second_gen, id1, "两轮同号 = 缓存会把上一轮的画面留下来");

        // 代码形状：纹理那一处不许再拿尺寸当内容的凭证。
        // 这一格只读 `freeze_texture` 自己那一段——`app.rs` 里 `#[cfg(test)]` 也出现在结构体字段
        // 与语句上，拿"截到第一个 cfg(test) 之前"那种整文件读法会把它后面的真代码一起切掉。
        let source = include_str!("app.rs");
        let from = source
            .find("fn freeze_texture")
            .expect("纹理那一处得有个出处");
        let body: String = source[from..].chars().take(1600).collect();
        assert!(
            body.contains("old_gen == frozen_id"),
            "缓存必须认这一轮的号，而不是认像素尺寸"
        );
        assert!(
            !body.contains("size[0] == w"),
            "尺寸一样不等于内容一样：同一块屏每轮抓出来尺寸都一样"
        );
    }

    /// **确认之后钉回原地**（CHG-009 REQ-001 那句"选完**原地**钉成一张浮在屏幕上的图"）。
    ///
    /// 系统框选那个年代拿不到那块矩形（`screencapture -i` 只交回一个文件），只能交给默认位；
    /// 自绘遮罩（CHG-010）交得出矩形，这一格就该兑现。故意用一块**原点不是 (0,0)** 的屏来量：
    /// 只算本地坐标会得出 `(100,80)`，而用户框住的那一块在 `(-2140,80)`——副屏在左那一档
    /// 就是这么算的（CHG-009 事实 34 同一课）。
    /// 另一半：从历史里取的那一张本来就没有"原地"可给，`at` 必须是 `None`。
    #[test]
    fn a_confirmed_pin_is_placed_where_the_box_was() {
        let mut st = capture_state();
        st.submit_capture(two_screens()).expect("该发得出");
        let frames = vec![Freeze {
            display: 2,
            points: Vec2::new(1920.0, 1080.0),
            appkit: Rect::from_min_size(Pos2::new(-1920.0, 0.0), Vec2::new(1920.0, 1080.0)),
            origin: Pos2::new(-1920.0, 0.0),
            image: Arc::new(image::RgbaImage::from_pixel(
                1920,
                1080,
                image::Rgba([1, 1, 1, 255]),
            )),
        }];
        assert!(matches!(
            st.apply_frozen(1, frames),
            CaptureApplied::Frozen { screens: 1 }
        ));
        let field = st.capture.as_ref().unwrap().field().unwrap();
        st.apply_selection(
            0,
            crate::system::selection::Selection::from_rect(
                &field,
                Rect::from_min_max(Pos2::new(100.0, 80.0), Pos2::new(300.0, 200.0)),
            ),
        );

        let id = st.pin_confirmed_capture(PathBuf::from("/tmp/x.png"));
        let pin = st.pins.iter().find(|p| p.id == id).expect("钉完就在集合里");
        assert_eq!(
            pin.at,
            Some(Pos2::new(-2140.0, 80.0)),
            "那块屏的全局原点 (-2240,0) + 选区在那块屏里的左上角 (100,80)，两处都得算"
        );

        // 「打开上一次」那一张：没有原地可给，交给系统摆，不许拿上一张的位置顶替
        let other = st.pin_from_history(PathBuf::from("/tmp/y.png"), None);
        let other = st.pins.iter().find(|p| p.id == other).expect("同上");
        assert_eq!(other.at, None, "这条入口本来就没有矩形");
    }

    /// 遮罩里的三个动作各自落到哪一格（REQ-102/105/106）。
    /// **取消不许留下选区**：留着它，下一次按 F1 会看见上一个框的残影。
    #[test]
    fn cancelling_a_capture_leaves_no_selection_and_no_path() {
        let mut st = capture_state();
        st.submit_capture(two_screens()).expect("该发得出");
        let frames = vec![Freeze {
            display: 1,
            points: Vec2::new(1920.0, 1080.0),
            appkit: Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0)),
            origin: Pos2::ZERO,
            image: Arc::new(image::RgbaImage::from_pixel(
                1920,
                1080,
                image::Rgba([9, 9, 9, 255]),
            )),
        }];
        assert!(matches!(
            st.apply_frozen(1, frames.clone()),
            CaptureApplied::Frozen { screens: 1 }
        ));
        assert!(st.capturing());
        assert!(st.crop_selection().is_none(), "还没选就不许裁得出东西");

        st.end_capture(CaptureStage::Cancelled);
        assert!(st.capture.is_none());
        assert_eq!(st.capture_stage, CaptureStage::Cancelled);

        // 同一批像素不能既被取消又被钉住
        st.pin_confirmed_capture(PathBuf::from("/tmp/x.png"));
        assert_eq!(st.pins.len(), 1);
    }

    /// 原生重下的配额**真的会减**。D-31~D-34 的病因就是"每帧重下、永不收手"：
    /// 帧循环读了 `push_frames > 0` 却一直没人减它，于是遮罩开着就一直枚举窗口、
    /// 重设 frame，把主线程拖死。收敛那一帧提前归零，没收敛就一帧减一格，
    /// 减到 0 之后再问也不许下溢回绕成 255（那等于永远收不了手）。
    #[test]
    fn the_native_repush_quota_is_spent_and_then_stops() {
        let mut st = capture_state();
        st.submit_capture(two_screens()).expect("该发得出");
        st.apply_frozen(
            1,
            vec![Freeze {
                display: 1,
                points: Vec2::new(40.0, 30.0),
                appkit: Rect::from_min_size(Pos2::ZERO, Vec2::new(40.0, 30.0)),
                origin: Pos2::ZERO,
                image: Arc::new(image::RgbaImage::from_pixel(
                    40,
                    30,
                    image::Rgba([1, 1, 1, 255]),
                )),
            }],
        );
        assert_eq!(st.capture.as_ref().unwrap().push_frames, PUSH_FRAMES);
        for left in (0..PUSH_FRAMES).rev() {
            st.consume_push_frame(false);
            assert_eq!(st.capture.as_ref().unwrap().push_frames, left);
        }
        st.consume_push_frame(false);
        assert_eq!(
            st.capture.as_ref().unwrap().push_frames,
            0,
            "用尽之后不许回绕成 255"
        );

        // 成为键盘窗那一刻提前收手，剩下的配额不再用
        st.capture.as_mut().unwrap().push_frames = PUSH_FRAMES;
        st.consume_push_frame(true);
        assert_eq!(st.capture.as_ref().unwrap().push_frames, 0);
    }

    /// 选区归"按下那一帧所在的那块屏"。换屏按下要丢掉旧框——旧框的坐标是
    /// 另一块屏的本地坐标，留着它就裁到不该裁的地方（副屏还带负偏移）。
    #[test]
    fn a_selection_belongs_to_the_screen_it_was_started_on() {
        let mut st = capture_state();
        st.submit_capture(two_screens()).expect("该发得出");
        let frames = (1..=2u8)
            .map(|display| Freeze {
                display,
                points: Vec2::new(1000.0, 800.0),
                appkit: Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0)),
                origin: Pos2::ZERO,
                image: Arc::new(image::RgbaImage::from_pixel(
                    1000,
                    800,
                    image::Rgba([1, 1, 1, 255]),
                )),
            })
            .collect();
        st.apply_frozen(1, frames);
        let field = st.capture.as_ref().unwrap().field().unwrap();
        let mut sel = crate::system::selection::Selection::fresh();
        sel.press(&field, Pos2::new(10.0, 10.0), 10.0);
        sel.drag(&field, Pos2::new(90.0, 70.0));
        sel.release(&field, Pos2::new(90.0, 70.0));
        st.apply_selection(0, sel);
        assert_eq!(st.capture.as_ref().unwrap().screen, 0);
        assert_eq!(
            st.capture.as_ref().unwrap().sel.rect,
            Some(Rect::from_min_max(
                Pos2::new(10.0, 10.0),
                Pos2::new(90.0, 70.0)
            ))
        );

        // 换到第二块屏按下：屏序号跟着换
        let mut fresh = crate::system::selection::Selection::fresh();
        let other = Freeze {
            display: 2,
            points: Vec2::new(1000.0, 800.0),
            appkit: Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0)),
            origin: Pos2::ZERO,
            image: Arc::new(image::RgbaImage::from_pixel(
                1000,
                800,
                image::Rgba([1, 1, 1, 255]),
            )),
        }
        .field();
        fresh.press(&other, Pos2::new(5.0, 5.0), 10.0);
        st.apply_selection(1, fresh);
        assert_eq!(st.capture.as_ref().unwrap().screen, 1);
    }

    /// 确认之后才有像素可裁，而且**裁出来的尺寸与尺寸读数同源**。
    #[test]
    fn a_confirmed_selection_crops_to_the_size_the_label_promised() {
        let mut st = capture_state();
        st.submit_capture(two_screens()).expect("该发得出");
        // 2 倍屏：主屏 1920×1080 点 = 3840×2160 像素。
        // 注意 `points` 不是这张图说了算——`apply_frozen` 会按**屏的规格**配对后覆盖它，
        // 于是"屏给尺寸、图给像素"这一条分工在这里被顺手钉住：喂进来的 1000×800 会被换成 1920×1080
        st.apply_frozen(
            1,
            vec![Freeze {
                display: 1,
                points: Vec2::new(1000.0, 800.0),
                appkit: Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0)),
                origin: Pos2::ZERO,
                image: Arc::new(image::RgbaImage::from_pixel(
                    3840,
                    2160,
                    image::Rgba([5, 6, 7, 255]),
                )),
            }],
        );
        let field = st.capture.as_ref().unwrap().field().unwrap();
        assert_eq!(
            field.scale(),
            Some(2.0),
            "屏的规格要盖过图自己的尺寸，否则换算没有唯一来源"
        );
        let mut sel = crate::system::selection::Selection::fresh();
        sel.press(&field, Pos2::new(100.0, 100.0), 10.0);
        sel.drag(&field, Pos2::new(300.0, 250.0));
        sel.release(&field, Pos2::new(300.0, 250.0));
        st.apply_selection(0, sel);
        let label = st.capture.as_ref().unwrap().sel.size_label(&field);
        let cut = st.crop_selection().expect("该裁得出来");
        assert_eq!(label.as_deref(), Some("400 × 300"), "读数与实际裁剪对不上");
        assert_eq!((cut.width(), cut.height()), (400, 300));
    }

    /// 一块屏都没冻上：那是失败，**不是用户取消**（静默抓屏没有"取消"这一档）。
    #[test]
    fn an_empty_freeze_is_a_failure_never_a_cancellation() {
        let mut st = capture_state();
        st.submit_capture(two_screens()).expect("该发得出");
        let applied = st.apply_frozen(1, Vec::new());
        assert!(matches!(applied, CaptureApplied::Failed(_)), "{applied:?}");
        assert!(!matches!(applied, CaptureApplied::Cancelled));
        assert!(!st.capturing());
        assert!(st.capture.is_none(), "没画面就不该留一扇空遮罩的账");
    }

    fn pixels_and_doc(width: u32, height: u32) -> (ice_vision::import::Admitted, OcrDocument) {
        (
            ice_vision::import::admit_rgba(
                image::RgbaImage::from_pixel(width, height, image::Rgba([1, 2, 3, 255])),
                "测试",
            ),
            OcrDocument::empty(width, height),
        )
    }

    // ---- CHG-009 第四条通道（T-004/T-005 的无头判据） ----

    /// 回信只改状态：成功那一支建一张贴图、派一次"只识别"，**一条翻译任务都不许出现**。
    /// 断言写成"不发 `Job::ImageTranslate` / `Job::Translation`"，而不是"译文是 None"——
    /// 后者在"根本没派过"与"派了但失败"两种情况下长得一模一样。
    #[test]
    fn a_capture_reply_builds_a_pin_and_dispatches_recognize_not_translation() {
        let mut st = capture_state();
        let path = PathBuf::from("/tmp/history/1-1-100.png");
        // 先真派发一次：回信带的是那一刻的序号，凭空喂一个 1 会被判过期
        let job = st
            .submit_capture(two_screens())
            .expect("空闲时该发得出截图任务");
        assert!(
            matches!(
                job,
                Job::ScreenCapture { id: 1, seq: 1, .. } | Job::Freeze { id: 1, seq: 1, .. }
            ),
            "{job:?}"
        );
        let applied = st.apply_capture(1, capture::Outcome::Captured(path.clone()));
        assert_eq!(applied, CaptureApplied::Pinned { pin: 1 });
        assert_eq!(st.pins.len(), 1);
        assert_eq!(st.pins[0].path, path);
        assert_eq!(st.capture_stage, CaptureStage::Pinned);
        // 翻译段与合成段一个都没被这条回信碰过
        assert!(st.pins[0].translate_in_flight.is_none());
        assert!(st.pins[0].product.is_none());

        // 识别派发走的是贴图自己那一支，回来的是"块"，仍然不是翻译
        let job = st
            .submit_pin_recognize(1)
            .expect("刚钉上就该能派一次只识别");
        assert!(
            matches!(
                job,
                Job::PinRecognize {
                    pin: 1,
                    generation: 1,
                    ..
                }
            ),
            "派出来的必须是贴图识别：{job:?}"
        );
        assert!(st.pins[0].stage == RecognitionStage::Recognizing);
    }

    /// 取消那一支：**贴图不建、别的状态一个都不动**（REQ-002 与面板 `Cancelled` 同规）。
    #[test]
    fn a_cancelled_reply_touches_nothing_but_its_own_stage() {
        let mut st = capture_state();
        st.capture_in_flight = Some(1);
        st.capture_stage = CaptureStage::Capturing;

        let applied = st.apply_capture(1, capture::Outcome::Cancelled);
        assert_eq!(applied, CaptureApplied::Cancelled);
        assert!(st.pins.is_empty(), "取消了就不该有贴图");
        assert_eq!(st.capture_stage, CaptureStage::Cancelled);
        assert!(st.capture_in_flight.is_none());
    }

    /// 一次框选在飞时**不发起第二次**：后到的直接忽略。这条不靠按钮变灰兜住——
    /// 热键那一下不经过按钮。
    #[test]
    fn a_second_capture_while_one_is_in_flight_is_ignored() {
        let mut st = capture_state();
        let first = st.submit_capture(two_screens());
        assert!(first.is_some());
        assert!(
            st.submit_capture(two_screens()).is_none(),
            "在飞时第二次派发必须被拒"
        );
        assert_eq!(
            st.capture_gate(),
            Some("上一次截图还没完"),
            "按钮的话与状态的事实得是同一件事"
        );
        // 序号只走了一次
        assert_eq!(st.capture_request_id, 1);
        assert_eq!(st.capture_seq, 1);
    }

    /// 过期回信（被更晚一次顶掉、或本来就没在飞）：**连状态都不动**，返回 `Stale`。
    #[test]
    fn a_stale_capture_reply_leaves_the_screen_alone() {
        let mut st = capture_state();
        st.capture_in_flight = Some(7);
        st.capture_stage = CaptureStage::Capturing;
        let applied = st.apply_capture(
            9,
            capture::Outcome::Captured(PathBuf::from("/tmp/history/9-9-9.png")),
        );
        assert_eq!(applied, CaptureApplied::Stale);
        assert!(st.pins.is_empty());
        assert_eq!(
            st.capture_stage,
            CaptureStage::Capturing,
            "过期回信不许改阶段"
        );
        assert_eq!(st.capture_in_flight, Some(7));
    }

    /// **没有数量上限**（用户 2026-11-16 改口 CHG-009 Q-007）：第七张、第八张照样钉得上，
    /// 已经钉着的那些一张不少、次序不变。门也不因数量变灰。
    #[test]
    fn pins_have_no_count_cap_and_the_gate_never_grays_on_count() {
        let mut st = capture_state();
        let before: Vec<PathBuf> = (1..=6)
            .map(|i| PathBuf::from(format!("/tmp/history/{i}.png")))
            .collect();
        for path in &before {
            st.push_pin(path.clone(), None);
        }
        assert_eq!(st.pins.len(), 6);
        assert_eq!(st.capture_gate(), None, "钉了六张门也不该灰");

        st.capture_in_flight = Some(1);
        let applied = st.apply_capture(
            1,
            capture::Outcome::Captured(PathBuf::from("/tmp/history/7.png")),
        );
        assert!(
            matches!(applied, CaptureApplied::Pinned { .. }),
            "第七张照样钉得上"
        );
        // 已经钉着的六张：一张不少、次序不变、路径没被换
        assert_eq!(st.pins.len(), 7);
        assert_eq!(
            st.pins
                .iter()
                .take(6)
                .map(|p| p.path.clone())
                .collect::<Vec<_>>(),
            before
        );
    }

    /// 贴图的识别按**代**丢弃：重派之后上一代的回信整条作废，
    /// 界面那一格保持它现在的样子（第四条序号互不作废那条纪律的第四处）。
    #[test]
    fn a_late_pin_reply_cannot_overwrite_the_newer_generation() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/a.png"), None);
        // 第一代：派发 → 落地，文档是那一份 8×8
        let gen1 = match st.submit_pin_recognize(1) {
            Some(Job::PinRecognize { generation, .. }) => generation,
            other => panic!("刚钉上的贴图该能派一次只识别：{other:?}"),
        };
        assert!(st.apply_pin_recognized(1, gen1, Ok(pixels_and_doc(8, 8))));
        assert_eq!(st.pins[0].document.as_ref().unwrap().width, 8);

        // 第二代：同一张再派一次，换代
        let gen2 = match st.submit_pin_recognize(1) {
            Some(Job::PinRecognize { generation, .. }) => generation,
            other => panic!("落地之后同一张该能再派一次：{other:?}"),
        };
        assert_eq!(gen1, 1);
        assert_eq!(gen2, 2);
        assert_eq!(st.pins[0].stage, RecognitionStage::Recognizing);

        // 上一代那条晚到的回信（20×20）：整条丢弃，界面那一格还是第一代那份 8×8
        assert!(
            !st.apply_pin_recognized(1, gen1, Ok(pixels_and_doc(20, 20))),
            "上一代的回信不许盖掉这一代"
        );
        assert_eq!(st.pins[0].document.as_ref().unwrap().width, 8);
        assert_eq!(st.pins[0].stage, RecognitionStage::Recognizing);

        // 这一代的回信回来才落地
        assert!(st.apply_pin_recognized(1, gen2, Ok(pixels_and_doc(20, 20))));
        assert_eq!(st.pins[0].document.as_ref().unwrap().width, 20);
        assert_eq!(st.pins[0].stage, RecognitionStage::Done { blocks: 0 });

        // 贴图关掉之后，任何一代的回信都落不下去（号不复用，落不到新贴图上）
        assert!(st.close_pin(1));
        assert!(!st.apply_pin_recognized(1, gen2, Ok(pixels_and_doc(30, 30))));
    }

    /// "这张图上真没有可读的字"沿用 CHG-005 那一档，**不当失败**（REQ-002 末句）。
    /// 同时 `document` 必须是 `None`——REQ-005 的"没长出来就不许假装能选"靠这一格成立。
    #[test]
    fn a_pin_with_no_text_is_not_a_failure_and_has_nothing_to_select() {
        use ice_vision::error::VisionError;
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/b.png"), None);
        st.submit_pin_recognize(1);
        let applied = st.apply_pin_recognized(1, 1, Err(VisionError::NoTextFound));
        assert!(applied);
        assert_eq!(st.pins[0].stage, RecognitionStage::NoText);
        assert!(
            st.pins[0].document.is_none(),
            "文字层没长出来时数据源必须是 None"
        );

        // 数据源是 None 时，拖选的命中测试拿不到任何块：这里直接断言那份数据是空的
        let blocks = st.pins[0]
            .document
            .as_ref()
            .map(|d| d.blocks.len())
            .unwrap_or_default();
        assert_eq!(blocks, 0);
    }

    /// 关掉一张：不存在的号返回 `false`，调用方因此不能凭空说"已关闭"。
    /// 关完之后剩下的那些不受影响（号不复用，晚到的回信落不到新图上）。
    #[test]
    fn closing_a_pin_reports_whether_it_was_actually_there() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        st.push_pin(PathBuf::from("/tmp/history/2.png"), None);
        assert!(st.close_pin(1));
        assert!(!st.close_pin(1), "再关一次要说没关掉（它本来就不在）");
        assert!(!st.close_pin(99), "不存在的号不许装作成功");
        assert_eq!(st.pins.len(), 1);
        assert_eq!(st.pins[0].id, 2);
        // 关过的号不会被下一张顶替
        assert_eq!(st.push_pin(PathBuf::from("/tmp/history/3.png"), None), 3);
    }

    /// 带子开合只认真存在的贴图（REQ-003 那条"收起=窗口缩到图片本身"的状态面）。
    #[test]
    fn the_band_toggle_refuses_a_pin_that_is_not_there() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        assert!(st.pins[0].band_open, "刚钉上时带子是开着的");
        assert!(st.set_pin_band(1, false));
        assert!(!st.pins[0].band_open);
        assert!(!st.set_pin_band(77, true));
    }

    /// 阴影默认开、右键翻一下只动这一张、不存在的号拒绝（用户 2026-11-12 那条改判的状态面）。
    #[test]
    fn the_shadow_defaults_on_and_toggles_per_pin() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        st.push_pin(PathBuf::from("/tmp/history/2.png"), None);
        assert!(
            st.pins[0].shadow,
            "刚钉上时阴影是开着的（要从桌面里凸出来）"
        );
        assert!(st.toggle_pin_shadow(1));
        assert!(!st.pins[0].shadow);
        assert!(st.pins[1].shadow, "关第一张的影不许波及第二张");
        assert!(st.toggle_pin_shadow(1));
        assert!(st.pins[0].shadow, "再翻一下又开回来");
        assert!(!st.toggle_pin_shadow(77));
    }

    /// 首用提示（CHG-011 REQ-005）：第一张钉上才弹、同一进程第二张不再弹、勾了确认写配置。
    #[test]
    fn the_pin_hint_shows_once_on_the_first_pin_and_checked_ack_persists() {
        let mut st = capture_state();
        assert_eq!(st.pin_hint, PinHint::NotShown);

        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        assert_eq!(
            st.pin_hint,
            PinHint::Showing {
                dont_show_again: false
            },
            "第一张钉上且配置没勾过 ⇒ 弹一次"
        );

        // 同一进程钉第二张：不重置、不再弹（仍是那个 Showing）
        st.push_pin(PathBuf::from("/tmp/history/2.png"), None);
        assert_eq!(
            st.pin_hint,
            PinHint::Showing {
                dont_show_again: false
            }
        );

        st.toggle_pin_hint_dont_show();
        assert_eq!(
            st.pin_hint,
            PinHint::Showing {
                dont_show_again: true
            }
        );

        // 勾了确认 ⇒ 写配置 bool + 本进程收掉
        assert!(st.ack_pin_hint(true));
        assert_eq!(st.pin_hint, PinHint::DoneThisRun);
        assert!(st.config.ui.pin_hint_dismissed, "勾了就要跨启动不再弹");
        assert!(st.config_dirty, "写了配置就得落盘");

        st.push_pin(PathBuf::from("/tmp/history/3.png"), None);
        assert_eq!(st.pin_hint, PinHint::DoneThisRun, "此后再钉也不弹");
    }

    /// 没勾就确认：不写持久化标记，本进程不再弹，但**下次启动**第一张又弹。
    #[test]
    fn acking_without_the_check_does_not_persist_and_next_launch_shows_again() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        assert!(!st.ack_pin_hint(false), "没勾不该报告写了配置");
        assert_eq!(st.pin_hint, PinHint::DoneThisRun);
        assert!(!st.config.ui.pin_hint_dismissed, "没勾不许写持久化标记");
        assert!(!st.config_dirty);

        // 下一次启动（配置仍 false）：新进程第一张又弹
        let mut next = capture_state();
        next.config.ui.pin_hint_dismissed = st.config.ui.pin_hint_dismissed;
        next.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        assert_eq!(
            next.pin_hint,
            PinHint::Showing {
                dont_show_again: false
            }
        );
    }

    /// 提示开着时 esc 关那张图：提示随之收（画在药丸窗里，窗没了就没了），本进程算弹过、不写配置。
    #[test]
    fn closing_the_pin_while_the_hint_is_up_dismisses_it_for_this_run() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        assert_eq!(
            st.pin_hint,
            PinHint::Showing {
                dont_show_again: false
            }
        );
        assert!(st.close_pin(1));
        assert_eq!(
            st.pin_hint,
            PinHint::DoneThisRun,
            "关图一下做到两件，不做两段式"
        );
        assert!(!st.config.ui.pin_hint_dismissed, "关图不该写持久化标记");
        st.push_pin(PathBuf::from("/tmp/history/2.png"), None);
        assert_eq!(st.pin_hint, PinHint::DoneThisRun, "再钉一张不再弹");
    }

    /// 勾过「不再提示」的机器：第一张也不弹（配置 bool 为 true 那一支）。
    #[test]
    fn a_dismissed_config_never_shows_the_hint() {
        let mut st = capture_state();
        st.config.ui.pin_hint_dismissed = true;
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        assert_eq!(st.pin_hint, PinHint::NotShown);
    }

    /// 文字层是不是"长出来了"：判据是**识别完成且文档真在**，两张都要。
    /// 单独一条"文档非空"不够——`Failed`/`NoText` 那两档此刻什么都选不了，
    /// 而 `Recognizing` 那一档文档是上一轮的残留时更危险（会拿旧图的字去选新图）。
    #[test]
    fn the_text_layer_only_counts_when_recognition_finished() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        let pin = &mut st.pins[0];
        assert!(!pin.text_ready(), "刚钉上时还没有文字层");
        pin.stage = RecognitionStage::Recognizing;
        pin.document = Some(pin_doc());
        assert!(!pin.text_ready(), "识别在飞时旧文档不算已经能选");
        pin.stage = RecognitionStage::Done { blocks: 1 };
        assert!(pin.text_ready());
        pin.stage = RecognitionStage::NoText;
        assert!(!pin.text_ready(), "未发现文字那一档不该给出可选的暗示");
    }

    /// 选中的块拼出去的是**那段文字**；没选中、或选中的块已经不在这一轮的文档里，
    /// 交出来的是 `None`——调用方据此退回"复制图像"（不许交出一个空串当选中）。
    #[test]
    fn a_selection_yields_text_or_nothing_not_an_empty_string() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        let pin = &mut st.pins[0];
        pin.stage = RecognitionStage::Done { blocks: 1 };
        pin.document = Some(pin_doc());
        assert_eq!(pin.selection_text(), None, "还没选就是没选");
        pin.selected = vec![0];
        assert_eq!(pin.selection_text().as_deref(), Some("hello world"));
        pin.selected = vec![7];
        assert_eq!(
            pin.selection_text(),
            None,
            "不在文档里的 id 不许交出别处的文字"
        );
        pin.selected = vec![];
        assert_eq!(pin.selection_text(), None);
    }

    /// **空表是一次合法写入**（它就是"取消选中"）。当成"这帧没新信息"忽略掉的话，
    /// 用户松开一个空拖之后带子还写着"已选中 N 段"。
    #[test]
    fn clearing_a_selection_is_a_write_not_a_no_op() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"), None);
        st.pins[0].selected = vec![0];
        assert!(st.set_pin_selection(1, vec![0, 1]));
        assert_eq!(st.pins[0].selected, vec![0, 1]);
        assert!(st.set_pin_selection(1, Vec::new()));
        assert!(st.pins[0].selected.is_empty(), "取消选中必须真的落下去");
        // 不存在的贴图：返回 false，调用方不许凭空说"选好了"
        assert!(!st.set_pin_selection(99, vec![1]));
    }

    /// 那颗按钮的话与状态行读**同一个来源**：两道门从上往下——这台机器没这件事、
    /// 上一次还在画。这里锁后一道（第一道要换平台才看得到）。数量不设门（Q-007 已改口）。
    #[test]
    fn the_button_gate_and_the_status_line_read_one_source() {
        let mut st = capture_state();
        if !capture::capability().capture {
            // 非 mac：第一道门就该亮
            assert_eq!(
                st.capture_gate(),
                Some("这台机器没有可用的框选截图；可先用系统截图再点「粘贴」")
            );
            return;
        }
        assert_eq!(st.capture_gate(), None, "空闲时按钮不该灰");
        st.submit_capture(two_screens());
        assert_eq!(st.capture_gate(), Some("上一次截图还没完"));
        st.capture_in_flight = None;
        for i in 0..8 {
            st.push_pin(PathBuf::from(format!("/tmp/history/{i}.png")), None);
        }
        assert_eq!(st.capture_gate(), None, "钉再多张门也不灰");
    }

    use super::*;

    fn ok(text: &str, engine: EngineKind) -> TranslationResult {
        TranslationResult {
            text: text.into(),
            engine,
            detected: Some(Language::En),
            elapsed_ms: 320,
            cache_hit: false,
            segments: 1,
        }
    }

    fn app_with_source(text: &str) -> AppState {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = text.into();
        state
    }

    /// 各支里只有文本那一支带请求，这几条测试断的全是它——取不出来就说明用错了分支。
    fn request_of(job: &Job) -> &TranslationRequest {
        match job {
            Job::Translation { request, .. } => request,
            Job::ImageTranslate { .. }
            | Job::ImageCompose { .. }
            | Job::ScreenCapture { .. }
            | Job::Freeze { .. }
            | Job::PinRecognize { .. } => {
                panic!("这条断言只适用于文本任务")
            }
        }
    }

    fn engine_of(job: &Job) -> EngineKind {
        match job {
            Job::Translation { engine, .. } => *engine,
            Job::ImageTranslate { .. }
            | Job::ImageCompose { .. }
            | Job::ScreenCapture { .. }
            | Job::Freeze { .. }
            | Job::PinRecognize { .. } => {
                panic!("这条断言只适用于文本任务")
            }
        }
    }

    /// 带 `id` 的各支，测试里取它就在这儿展开一次。
    /// 不给 `Job` 加 `fn id()`：派发方本来就是按支 match 的，那个访问器只有测试在用。
    fn job_id(job: &Job) -> RequestId {
        match job {
            Job::Translation { id, .. }
            | Job::ImageTranslate { id, .. }
            | Job::ImageCompose { id, .. }
            | Job::ScreenCapture { id, .. }
            | Job::Freeze { id, .. } => *id,
            // 贴图那条带的是 `pin` + `generation`，**不是** 请求序号：
            // 给它编一个 `id` 就等于让测试可以拿错东西去断言
            Job::PinRecognize { .. } => panic!("贴图识别那条没有请求序号"),
        }
    }

    fn translation_outcome(
        id: RequestId,
        engine: EngineKind,
        result: Result<TranslationResult, ProviderError>,
    ) -> Outcome {
        Outcome::Translation { id, engine, result }
    }

    #[test]
    fn empty_source_does_not_submit_and_clears_stale_result() {
        let mut state = app_with_source("  \n ");
        state.translation = Some(ok("旧译文", EngineKind::Machine));
        assert!(state.submit().is_none());
        assert!(state.translation.is_none());
        assert_eq!(state.request_id, 0);
    }

    #[test]
    fn submit_while_in_flight_is_ignored() {
        let mut state = app_with_source("hello");
        let first = state.submit().expect("首次提交");
        assert_eq!(job_id(&first), 1);
        assert!(state.submit().is_none(), "请求进行中不得叠加第二个请求");
        assert_eq!(state.request_id, 1);
    }

    /// REQ-006 的纯函数：同语言才翻，中文（简/繁）↔ 英语，其余一律回简体中文。
    /// 不同语言返回 `None`——这一条同时守住"一次提交最多翻转一次"：翻完的目标与检出语言必然不同。
    #[test]
    fn a_colliding_target_flips_to_the_other_side() {
        assert_eq!(
            auto_reverse_target(Language::ZhHans, Language::ZhHans),
            Some(Language::En)
        );
        assert_eq!(
            auto_reverse_target(Language::ZhHant, Language::ZhHant),
            Some(Language::En)
        );
        assert_eq!(
            auto_reverse_target(Language::En, Language::En),
            Some(Language::ZhHans)
        );
        assert_eq!(
            auto_reverse_target(Language::Ja, Language::Ja),
            Some(Language::ZhHans)
        );
        assert_eq!(
            auto_reverse_target(Language::Fr, Language::Fr),
            Some(Language::ZhHans)
        );
        assert_eq!(auto_reverse_target(Language::En, Language::ZhHans), None);
        assert_eq!(auto_reverse_target(Language::ZhHans, Language::En), None);
    }

    /// 英文源文 + 目标也是英语：发请求之前就翻成简体中文，用户不需要再按一次 Enter
    #[test]
    fn an_english_source_against_an_english_target_is_reversed_before_the_request() {
        let mut state = app_with_source("hello world, this is a small test");
        state.config.translation.target = Language::En;

        let job = state.submit().expect("检测失败不该拦住提交");

        assert_eq!(
            request_of(&job).pair.target,
            Language::ZhHans,
            "Job 用的必须是翻转后的语言对"
        );
        assert_eq!(request_of(&job).pair.source, None, "源语言永远留在自动检测");
        assert_eq!(state.config.translation.target, Language::ZhHans);
        assert_eq!(
            state.config.translation.recent_targets.first(),
            Some(&Language::ZhHans),
            "新目标语言要走 remember_target 进最近使用"
        );
        assert!(state.config_dirty, "改了语言就得落盘");
        let toast = state
            .toasts
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join("|");
        assert!(
            toast.contains("检测到英语") && toast.contains("英语 → 简体中文"),
            "提示条要说明改成了什么：{toast}"
        );
    }

    /// 中文源文撞中文目标是更常见的一侧：换成英语
    #[test]
    fn a_chinese_source_against_a_chinese_target_goes_to_english() {
        let mut state = app_with_source("今天天气很好，我们去公园散步");
        let job = state.submit().expect("要发请求");
        assert_eq!(request_of(&job).pair.target, Language::En);
        assert_eq!(state.config.translation.target, Language::En);
    }

    /// 用户**显式**选了源语言时一个字都不改（REQ-006 第三条）：照 P4 让引擎拦下
    #[test]
    fn an_explicit_source_language_is_never_overridden() {
        let mut state = app_with_source("hello world");
        state.config.translation.source = Some(Language::En);
        state.config.translation.target = Language::En;

        let job = state.submit().expect("照样发请求，由引擎同语言拒绝");

        assert_eq!(request_of(&job).pair.source, Some(Language::En));
        assert_eq!(
            request_of(&job).pair.target,
            Language::En,
            "显式设定不许应用替用户改"
        );
        assert!(state.toasts.is_empty(), "没改语言就不该有提示条");
    }

    /// 变更 I（2026-10-28）：提示条整条 lifetime 就是一次线性淡出。
    /// 判的是"多老有多淡"这条式子本身——画它的那一侧只乘这一个数，别处不再另算一套。
    /// 名字里不写时长：2026-10-29 用户把那 500ms 抬成了 1s，改判只改常量。
    #[test]
    fn a_toast_fades_linearly_over_its_whole_lifetime() {
        let mut state = app_with_source("hello");
        state.toasts.clear();
        state.toast(Tone::Info, "已复制");
        let toast = &state.toasts[0];
        assert!(
            (toast.opacity() - 1.0).abs() < 0.05,
            "刚出生的提示条几乎该是实心的，实得 {}",
            toast.opacity()
        );

        // 把时间戳拨到半程：不要求精确到 0.5，只要求"确实在往下走且没走过头"
        state.toasts[0].shown_at -= TOAST_LIFETIME / 2;
        let middle = state.toasts[0].opacity();
        assert!(
            (0.4..=0.65).contains(&middle),
            "半程该淡到一半上下，实得 {middle}"
        );

        state.toasts[0].shown_at -= TOAST_LIFETIME;
        assert_eq!(
            state.toasts[0].opacity(),
            0.0,
            "过了点就该完全不画，不许留一点残影"
        );
        assert!(state.tick_toasts(), "到点要收掉，并让调用方补一次重绘");
        assert!(state.toasts.is_empty());
    }

    /// 浮空提示（用户 2026-11-16）：同一时刻只浮一句，新的整条顶掉旧的；寿命与 toasts
    /// 同一条式子，到点 tick 收掉。它不进 `toasts`——两条路各画各的，不许叠在一起。
    #[test]
    fn a_float_hint_is_replaced_wholesale_and_expires_on_its_own_clock() {
        let mut state = app_with_source("hello");
        state.set_float_hint(Tone::Info, "还没有截图记录");
        assert!(state.toasts.is_empty(), "浮空提示不走主窗那条 toasts");
        let first = state.float_hint.as_ref().unwrap();
        assert!((first.opacity() - 1.0).abs() < 0.05, "刚出生几乎实心");

        state.set_float_hint(Tone::Info, "没有更多截图了");
        assert_eq!(
            state.float_hint.as_ref().unwrap().text,
            "没有更多截图了",
            "新的整条顶掉旧的，不攒第二条"
        );

        assert!(!state.tick_float_hint(), "没到点不收");
        state.float_hint.as_mut().unwrap().shown_at -= TOAST_LIFETIME;
        assert!(state.tick_float_hint(), "到点要收掉");
        assert!(state.float_hint.is_none());
        assert!(!state.tick_float_hint(), "已经没了再 tick 不报变化");
    }

    /// 检不出语言（纯数字）：不翻、不拦，请求按 P4 的回退照常发
    #[test]
    fn undetectable_text_does_not_trigger_a_reversal() {
        let mut state = app_with_source("12345 67890");
        state.config.translation.target = Language::En;

        let job = state.submit().expect("检测失败不阻塞提交");

        assert_eq!(request_of(&job).pair.target, Language::En);
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn stale_outcome_is_dropped_without_touching_visible_text() {
        let mut state = app_with_source("hello");
        let first = state.submit().unwrap();
        // 用户改了源文并再次提交（第一次的结果此时才回来）
        state.in_flight = None;
        state.source = "world".into();
        let second = state.submit().unwrap();
        assert_ne!(job_id(&first), job_id(&second));

        let stale = translation_outcome(
            job_id(&first),
            EngineKind::Machine,
            Ok(ok("旧结果", EngineKind::Machine)),
        );
        assert!(!state.apply_outcome(stale));
        assert!(state.translation.is_none(), "过期结果不得回写");
        assert_eq!(state.in_flight, Some(job_id(&second)), "新请求仍在飞行中");

        let fresh = translation_outcome(
            job_id(&second),
            EngineKind::Machine,
            Ok(ok("新结果", EngineKind::Machine)),
        );
        assert!(state.apply_outcome(fresh));
        assert_eq!(state.translation.unwrap().text, "新结果");
        assert_eq!(state.in_flight, None);
    }

    #[test]
    fn failure_keeps_source_and_maps_action() {
        let mut state = app_with_source("hello");
        let job = state.submit().unwrap();
        state.apply_outcome(translation_outcome(
            job_id(&job),
            EngineKind::Ai,
            Err(ProviderError::Unauthorized),
        ));
        assert_eq!(state.source, "hello", "失败不得丢源文");
        let failure = state.failure.as_ref().expect("要有可读错误");
        assert_eq!(failure.message, ProviderError::Unauthorized.to_string());
        assert_eq!(failure.action, Some(FailureAction::GoSettings));
        assert!(state.translation.is_none());

        state.go_to_key_settings();
        assert_eq!(state.tab, Tab::Settings);
        assert!(state.focus_key_input);
    }

    #[test]
    fn timeout_on_ai_offers_machine_fallback_but_not_on_machine() {
        let err = ProviderError::Timeout(15_000, 2);
        let ai = failure_of(&err, EngineKind::Ai);
        assert_eq!(ai.action, Some(FailureAction::UseMachine));
        assert!(ai.message.contains("已重试 2 次"), "{}", ai.message);
        assert_eq!(failure_of(&err, EngineKind::Machine).action, None);
    }

    #[test]
    fn rate_limit_keeps_retry_possible_and_has_no_action() {
        let failure = failure_of(&ProviderError::RateLimited, EngineKind::Ai);
        assert_eq!(failure.action, None);
        assert_eq!(failure.message, ProviderError::RateLimited.to_string());
    }

    #[test]
    fn switching_engine_or_target_resubmits_current_source() {
        let mut state = app_with_source("hello");
        let id = job_id(&state.submit().unwrap());
        state.apply_outcome(translation_outcome(
            id,
            EngineKind::Machine,
            Ok(ok("你好", EngineKind::Machine)),
        ));

        let job = state.set_engine(EngineKind::Ai).expect("切引擎要重译");
        assert_eq!(engine_of(&job), EngineKind::Ai);
        assert_eq!(request_of(&job).text, "hello");

        state.in_flight = None;
        let job = state.set_target(Language::Ja).expect("改目标语言要重译");
        assert_eq!(request_of(&job).pair.target, Language::Ja);
        assert_eq!(state.config.translation.recent_targets[0], Language::Ja);
        assert!(state.config_dirty);
    }

    #[test]
    fn mode_toggle_preserves_content_and_does_not_request() {
        let mut state = app_with_source("hello");
        let id = job_id(&state.submit().unwrap());
        state.apply_outcome(translation_outcome(
            id,
            EngineKind::Machine,
            Ok(ok("你好", EngineKind::Machine)),
        ));
        let id_before = state.request_id;

        for _ in 0..20 {
            let next = state.next_mode();
            state.set_mode(next);
        }

        assert_eq!(state.mode(), UiMode::Full);
        assert_eq!(state.source, "hello");
        assert_eq!(state.translation.unwrap().text, "你好");
        assert_eq!(state.request_id, id_before, "切换模式不得重复发请求");
    }

    #[test]
    fn char_count_is_cached_until_source_changes() {
        let mut state = app_with_source("你好世界");
        state.sync_char_count();
        assert_eq!(state.char_count, 4);
        state.sync_char_count();
        assert_eq!(state.char_count, 4);
        state.source.push_str("abc");
        state.sync_char_count();
        assert_eq!(state.char_count, 7);
    }

    #[test]
    fn term_edits_flow_into_config_and_mark_dirty() {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.term_source = "pod".into();
        state.term_target = "Pod".into();
        let term = state.add_term().expect("术语要能新增");
        assert_eq!(term.source, "pod");
        assert_eq!(state.config.terms.len(), 1);
        assert!(state.config_dirty);
        assert!(state.term_source.is_empty(), "输入缓冲要清空");

        state.term_target = String::new();
        assert!(state.add_term().is_none(), "译名为空不新增");

        assert!(state.remove_term("POD"));
        assert!(state.config.terms.is_empty());
    }

    #[test]
    fn history_row_loads_without_submitting() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(dir.path().join("history.jsonl"));
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.source = "hello".into();
        let result = ok("你好", EngineKind::Machine);
        state.submit();
        state.record_history(&store, &result);
        assert_eq!(state.history_count, 1);

        state.source = String::new();
        state.translation = None;
        assert!(state.load_history_row(0));
        assert_eq!(state.source, "hello");
        assert_eq!(state.translation.unwrap().text, "你好");
        assert_eq!(state.tab, Tab::Text);
        assert_eq!(state.request_id, 1, "回显历史不得自动发请求");
    }

    #[test]
    fn corrupt_history_shows_reason_and_keeps_app_usable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        std::fs::write(&path, "{ 这不是 jsonl\n").unwrap();
        let store = HistoryStore::at(&path);
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        state.reload_history(&store);
        // 损坏行被跳过而不是致命错误（core 的既有行为），因此这里不该有 error
        assert!(state.history_error.is_none(), "{:?}", state.history_error);
        state.source = "hello".into();
        assert!(state.submit().is_some());
    }

    #[test]
    fn status_text_reflects_state_and_ai_availability() {
        let mut state = AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        );
        assert!(state.status_text(false).starts_with("就绪"));
        assert!(state.status_text(false).contains("AI 未配置"));
        assert!(state.status_text(true).contains("AI 可用"));

        state.failure = Some(failure_of(&ProviderError::RateLimited, EngineKind::Ai));
        assert!(state.status_text(true).starts_with("错误：被限流"));
    }

    /// 派一次贴图识别、把错误落地，返回那张贴图最终停在哪个态。
    fn land_pin_failure(err: VisionError) -> RecognitionStage {
        let mut st = capture_state();
        let pin = recognized_pin(&mut st);
        st.submit_pin_recognize(pin).expect("刚钉上该派得出识别");
        assert!(
            st.apply_pin_recognized(pin, 1, Err(err)),
            "代对得上，结果该被接住"
        );
        st.pins[0].stage.clone()
    }

    /// REQ-007 / T-008：没有识别能力的两条来路——非 mac 编译面的 `UnsupportedPlatform`、
    /// macOS 上 Vision 真的失败的 `Recognition`——必须汇成同一个界面态。
    ///
    /// 判在 state 层的原因：置灰入口和那句话是画出来的，无头测不到像素；但"错误最后落地成什么"
    /// 是数据。`Recognition` 的字符串里带着 `code=` / `desc_len=`：它们必须留在日志里，
    /// 一处都不许进界面。
    #[test]
    fn losing_ocr_lands_in_the_same_failure_shape_whatever_the_platform() {
        let no_capability = land_pin_failure(VisionError::UnsupportedPlatform);
        let engine_failed = land_pin_failure(VisionError::Recognition(
            "perform_failed_code=1234 desc_len=42".into(),
        ));

        for stage in [&no_capability, &engine_failed] {
            let RecognitionStage::Failed(reason) = stage else {
                panic!("缺少识别能力要落在 Failed，实际是 {stage:?}");
            };
            assert!(
                stage.label().starts_with("识别失败："),
                "带子那一行要说清是识别这一步出的事：{}",
                stage.label()
            );
            assert!(
                !reason.contains("code=") && !reason.contains("desc_len"),
                "OS 排障字段漏进了界面文案：{reason}"
            );
        }

        // 降级的那句话：点名这台机器，同时说清文本那条路没断
        let RecognitionStage::Failed(reason) = &no_capability else {
            unreachable!()
        };
        assert!(
            reason.contains("当前平台") && reason.contains("不支持"),
            "{reason}"
        );
        assert!(reason.contains("文本"), "{reason}");
    }

    /// 给贴图摆一份"识别已回来"的文档：每串文字一个块。
    fn pin_with_texts(st: &mut AppState, texts: &[&str]) -> u64 {
        let pin = recognized_pin(st);
        let (admitted, mut document) = pixels_and_doc(600, 400);
        document.blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                TextBlock::new(
                    index as u32,
                    *text,
                    PixelRect::new(0.0, index as f32 * 20.0, 200.0, 18.0),
                    0.9,
                )
            })
            .collect();
        st.pins[0].source = Some(admitted);
        st.pins[0].document = Some(document);
        pin
    }

    /// REQ-001 第四条：`auto` 永远不许原样交给机翻端点。定得出方向时给的是**具体那一对**，
    /// 定不出时不派发，并且把话说到"该去改哪一个设置"。
    #[test]
    fn the_translate_batch_always_carries_a_concrete_language_pair() {
        // 图上的文字是英文：检出 En，目标照默认走简中
        let mut st = capture_state();
        let pin = pin_with_texts(&mut st, &["hello world"]);
        let job = st.submit_pin_translate(pin).expect("英文该定得出方向");
        let Job::ImageTranslate { blocks, .. } = &job else {
            panic!("要的是逐块翻译那一支");
        };
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].1.text, "hello world");
        assert_eq!(
            blocks[0].1.pair.source,
            Some(Language::En),
            "自动检测的结论必须落成具体源语言，端点不认识 auto"
        );

        // 图上的文字就是目标语言：换的是**目标**（回到英语），源语言照检出的那个
        let mut same = capture_state();
        let pin = pin_with_texts(&mut same, &["你好，今天怎么样"]);
        let flipped = same
            .submit_pin_translate(pin)
            .expect("中文撞中文目标该换成英语而不是失败");
        let Job::ImageTranslate { blocks, .. } = &flipped else {
            panic!("要的是逐块翻译那一支");
        };
        assert_eq!(blocks[0].1.pair.source, Some(Language::ZhHans));
        assert_eq!(
            blocks[0].1.pair.target,
            Language::En,
            "自己译自己的那条改判"
        );

        // 定不出方向（图上只有标点和数字）：不派发，且那句话指得懂下一步
        let mut blank = capture_state();
        let pin = pin_with_texts(&mut blank, &["—— 2026 ——"]);
        assert!(
            blank.submit_pin_translate(pin).is_none(),
            "认不出语言时不该把 auto 交出去"
        );
        assert_eq!(
            blank.pins[0].translate_stage.label(),
            format!("翻译失败：{NO_DIRECTION_TEXT}"),
            "那句话要说清缺的是源语言"
        );
        assert!(
            blank.pins[0].translate_in_flight.is_none(),
            "定不出方向时不能留下一个在飞的批次"
        );
    }

    /// 一批翻译被停止后回来（`cancelled: true`）：落地成 Stopped 而不是 Done，
    /// 已翻好的块照样保留（Q-006 裁定"留"）。
    #[test]
    fn a_cancelled_batch_lands_as_stopped_keeping_finished_blocks() {
        let mut st = capture_state();
        let pin = pin_with_texts(&mut st, &["第一块", "第二块"]);
        let job = st.submit_pin_translate(pin).unwrap();
        let Job::ImageTranslate { id, cancel, .. } = &job else {
            unreachable!()
        };
        assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(
            matches!(st.pins[0].translate_stage, TranslateStage::Running { .. }),
            "结果还没回来，段不能说停"
        );

        assert!(st.apply_outcome(Outcome::ImageTranslated {
            id: *id,
            pin,
            results: vec![(0, Ok("你好".to_string()))],
            cancelled: true,
            elapsed_ms: 30,
        }));
        assert_eq!(
            st.pins[0].translate_stage,
            TranslateStage::Stopped { ok: 1, failed: 0 }
        );
    }

    /// 一张成品，块数由 `replaced` 决定。测试只要形状，不要像素。
    fn a_composed(blocks: usize) -> Composed {
        Composed {
            image: image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])),
            per_block: (0..blocks as u32)
                .map(|id| (id, ice_vision::typeset::PlanKind::InPlace))
                .collect(),
            replaced: blocks,
            overlaid: 0,
            skipped: 0,
            degraded_blocks: Vec::new(),
            missing_glyphs: Vec::new(),
        }
    }

    /// REQ-006 第四条：文本侧与贴图侧各走各的序号，谁也不许让对方的在飞结果作废。
    /// 两支都从 0 起计，所以号子**会**撞成同一个数——正因如此过期判定必须分通道看，
    /// 这条断言把那个巧合直接写死。
    #[test]
    fn the_text_and_pin_channels_invalidate_only_their_own_results() {
        let mut state = app_with_source("hello");
        let pin = pin_with_texts(&mut state, &["图上第一块"]);
        let text = state.submit().expect("有源文就该派发");
        let image = state
            .submit_pin_translate(pin)
            .expect("同一时刻另一支也该派发得出");
        assert_eq!(
            job_id(&text),
            job_id(&image),
            "两支独立计数，号子撞上才证明判过期靠的不是数字本身"
        );

        assert!(state.apply_outcome(Outcome::ImageTranslated {
            id: job_id(&image),
            pin,
            results: vec![(0, Ok("图上译文".to_string()))],
            cancelled: false,
            elapsed_ms: 4,
        }));
        assert_eq!(
            state.in_flight,
            Some(job_id(&text)),
            "贴图译文落地不该把在飞的文本翻译算作过期"
        );
        assert!(state.translation.is_none(), "贴图译文不该写进译文栏");

        assert!(state.apply_outcome(Outcome::Translation {
            id: job_id(&text),
            engine: EngineKind::Machine,
            result: Ok(ok("译文", EngineKind::Machine)),
        }));
        assert_eq!(state.translation.as_ref().unwrap().text, "译文");
        assert_eq!(
            state.pins[0].document.as_ref().unwrap().blocks[0]
                .translation
                .as_deref(),
            Some("图上译文"),
            "文本翻译落地不该动贴图的文档"
        );
    }
}
