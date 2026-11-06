//! 界面状态与状态转移。
//!
//! 这里只放纯逻辑，不碰 egui 绘制，因此 REQ-001~REQ-004、REQ-007 的行为都能无头断言。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::Rect;
use ice_core::config::{Config, UiMode};
use ice_core::document::{OcrDocument, PixelRect, TextBlock};
use ice_core::glossary::{Glossary, Term};
use ice_core::history::{HISTORY_CAPACITY, HistoryRecord, HistoryStore};
use ice_core::language::{EngineKind, LangPair, Language};
use ice_providers::error::ProviderError;
use ice_providers::request::{TranslationRequest, TranslationResult};
use ice_vision::composite::Composed;
use ice_vision::error::VisionError;
use ice_vision::import::Admitted;
use ice_vision::layout::{RowObservation, flip_rect};
use ice_vision::typeset::CompositeMode;
use image::RgbaImage;

use crate::system::capture;

pub type RequestId = u64;

/// 全量模式的四个页签（`03-ui-design.md` §2 F-01）。术语表是设置页内的一个分区（F-12），不占页签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Text,
    Image,
    History,
    Settings,
}

impl Tab {
    pub const fn all() -> [Tab; 4] {
        [Tab::Text, Tab::Image, Tab::History, Tab::Settings]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Tab::Text => "文本",
            Tab::Image => "图片",
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

/// 那颗「停止」对着识别段时的 hover（Q-008）。
///
/// 三条文案分开写是这条入口唯一容易讲错的地方：只有翻译段是**真停**（后面的块不再发请求），
/// 另两段的计算已经在跑，按下去只是"这次结果我不收了"。写成一句"已停止"就撒了两次谎。
const RECOGNIZE_STOP_HINT: &str = "底层那次识别撤不回来；这一颗只是说结果回来时不要填进列表";

/// 对着翻译段：块边界真停，已翻好的留下（Q-006）
const TRANSLATE_STOP_HINT: &str = "停在块边界：已翻好的块留下，后面的块不再请求";

/// 对着合成段：整张图画完才换代，按下去丢的是这一次的结果
const COMPOSE_STOP_HINT: &str = "这一张撤不回来；这一颗只是说画完的结果不要换上来";

/// 两条出路（REQ-008）都要手上有成品。点下去到帧末执行之间那张成品被换掉了
/// （换了图、换了一批译文）时说的就是这一句——那一刻**不许把"已保存 / 已复制"说出口**，
/// 用户会以为手上那个文件就是屏幕上那张图。
pub const PRODUCT_GONE_TEXT: &str = "那一张成品已经不在了（换了图或换了译文）；请重新生成一次";

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// 一次识别任务的原料从哪来（CHG-005 REQ-001 的四条入口到此汇成两支）。
///
/// 分两支的唯一理由是**线程**：文件那三条入口把读盘与解码整个交给工作线程（spec 的并发那句
/// "解码与缩放同段跑在 `spawn_blocking` 里"），UI 线程只交出路径；剪贴板那支的字节已由
/// arboard 在主线程取出（它拿的是系统剪贴板句柄，挪到别的线程属于另一套线程约束），
/// 剩下的只有"拼成缓冲 + 超上限缩放"，那一段同样进工作线程。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageInput {
    Path(PathBuf),
    Clipboard {
        /// RGBA8 裸像素，长度必须是 `width × height × 4`（`system::clipboard::read_image` 已校验）
        bytes: Vec<u8>,
        width: usize,
        height: usize,
    },
}

/// 一次待派发的任务：文本翻译，或图片链上的三段之一（识别 / 逐块翻译 / 合成）。
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
    ImageOcr {
        id: RequestId,
        input: ImageInput,
    },
    /// 逐块翻译（REQ-001）。每一块带的就是**将要发给引擎的那一份请求**——定方向是派发方
    /// （[`AppState::submit_image_translate`]）的事，机翻端点从不认识 `auto`，而工作循环
    /// 拿到的是已经填好具体语言对的请求，它没有第二次改方向的机会。
    ///
    /// `blocks` 是 `(块 id, 该块请求)`，已按块 id 升序：同一份文档必须每次派发出同一个次序，
    /// 否则"重试这一块"的块号会和界面上那行对不上。
    ImageTranslate {
        id: RequestId,
        /// 这批译文落在哪一份文档上（图片页 vs 某张贴图）
        owner: ChainOwner,
        engine: EngineKind,
        blocks: Vec<(u32, TranslationRequest)>,
        /// 块边界停止的哨兵：界面置真，工作循环在**下一块请求之前**看到它就收手。
        /// 在飞的那一次请求不撤回（撤不回），所以停止最少要等一块的时间。
        cancel: Arc<AtomicBool>,
    },
    /// 发起一次区域截图（CHG-009 REQ-001）。**只带自增号，不带路径**：历史目录在哪
    /// 只有 `system::capture` 知道，状态层不许知道文件系统的事（REQ-005 那条分层纪律）。
    /// 跑在 `spawn_blocking` 里，因为它会**阻塞到用户画完为止**。
    ScreenCapture {
        id: RequestId,
        /// 历史文件名的第三段（同 pid 同秒的两次连点靠它分开）
        seq: u64,
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
        /// 成品画进哪一份；图片页那条落 `state.composed`，贴图那条落 `Pin::product`
        owner: ChainOwner,
        source: RgbaImage,
        document: OcrDocument,
        mode: CompositeMode,
    },
}

/// 识别段回来的一整份产物：入场后的像素、行级观察、合并好的文档。
///
/// `rows` 是 FR-2.3"拆分回原行"唯一的原料来源（`OcrDocument` 里只有合并后的块），
/// 而 `admitted` 携带的宽高就是文档宽高——REQ-003 的换算基准由这一份锁住。
#[derive(Debug)]
pub struct ImageRecognition {
    pub admitted: Admitted,
    pub rows: Vec<RowObservation>,
    pub document: OcrDocument,
    pub elapsed_ms: u64,
}

/// 一段图片链（翻译 / 合成）这次是为谁跑的。
///
/// **为什么不是给贴图另开一对 Job/Outcome**：那样就有两份逐块翻译的循环、两份"块号→译文"
/// 的落地代码，两份迟早不一样（图钉那次就是这个教训）。加一个归属标记，工作循环与落地函数
/// 各只有一份，贴图与图片页差的是"结果落在哪个文档上"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChainOwner {
    /// 图片页那一份工作区（`state.image` / `state.ocr` / `state.composed`）
    #[default]
    Page,
    /// 某张贴图自己的那一份（`Pin::document` / `Pin::product`）
    Pin(u64),
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
    Image {
        id: RequestId,
        result: Result<ImageRecognition, VisionError>,
    },
    /// 逐块翻译的一段推进（REQ-009 的"N/M 块"）：20 个块要等十几秒，用户得看见它在走。
    ImageTranslateProgress {
        id: RequestId,
        owner: ChainOwner,
        done: usize,
        total: usize,
    },
    /// 逐块翻译收工。`results` 是 `(块 id, 该块结果)`：一块 `Err` 只作废它自己，
    /// 其余块的 `Ok` 照样落地（REQ-001 第二条）。`cancelled` 表示这是在块边界停下来的，
    /// 已翻好的那些照样有效、不是半成品（Q-006 裁定"留"）。
    ImageTranslated {
        id: RequestId,
        owner: ChainOwner,
        results: Vec<(u32, Result<String, ProviderError>)>,
        cancelled: bool,
        elapsed_ms: u64,
    },
    ImageComposed {
        id: RequestId,
        owner: ChainOwner,
        result: Result<Composed, VisionError>,
        elapsed_ms: u64,
    },
    /// 一次框选回来了。三态（成功/取消/失败）原样带，**不在工作线程里判该干什么**
    ScreenCaptured {
        id: RequestId,
        outcome: crate::system::capture::Outcome,
    },
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
    /// 交互框选已经发起，正在等用户画完（可能两秒，也可能两分钟）
    Capturing,
    /// 拿到图并且钉上了。贴图的细节在 [`AppState::pins`] 里，这一态只说"这一段成功了"
    Pinned,
    /// 工具报错、权限被拒、图坏了——装的是一句能行动的话（`capture` 那侧算好的）
    Failed(String),
    /// 用户 esc 取消：**什么都不做**的那一条，状态行不许说"失败"
    Cancelled,
}

impl CaptureStage {
    /// 状态行那一格。空串=这一句不该出现（`Idle`/`Capturing` 由界面上那颗按钮自己表达）。
    pub fn label(&self) -> Option<&'static str> {
        match self {
            CaptureStage::Idle => None,
            CaptureStage::Capturing => Some("在框选里…"),
            CaptureStage::Pinned => Some("已钉住"),
            // 失败那句已经在 `Failed` 里带着了，这一层不再造第二句
            CaptureStage::Failed(_) => Some("截不出来"),
            CaptureStage::Cancelled => Some("已取消"),
        }
    }

    /// 状态行那一格的颜色档（与另外三段同口径：取消不算坏，只是这一次没往下走）
    pub const fn phase(&self) -> Phase {
        match self {
            CaptureStage::Capturing => Phase::Busy,
            CaptureStage::Pinned => Phase::Done,
            CaptureStage::Failed(_) => Phase::Bad,
            CaptureStage::Idle | CaptureStage::Cancelled => Phase::Idle,
        }
    }

    /// I 区那一格的完整说法：`Failed` 装的是一句能行动的话，要原样露出来
    pub fn line(&self) -> String {
        match self {
            CaptureStage::Failed(reason) => reason.clone(),
            stage => stage.label().unwrap_or("空闲").to_string(),
        }
    }
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
    /// 图片本体上有没有正被按住拖动（REQ-004，走查要看的那一件）
    pub dragging: bool,
    /// 这张的像素（识别回信带回来的那一份，与图片页的 `AppState::image` 同一种东西）。
    /// 合成要有底图，而底图**必须是屏上正在显示的那一张**——现读文件会读到没换成品之前的旧图。
    pub source: Option<Admitted>,
    /// 这张自己的翻译段与合成段。**与图片页那两条分开**：借图片页的在飞标记会出
    /// "贴图在翻，图片页的按钮因此变灰"这种隔空互相干扰。
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

    /// 这张贴图此刻该显示哪一份：有成品用成品，否则用原图。
    pub fn displayed(&self) -> Option<&Admitted> {
        self.source.as_ref()
    }

    /// 测试与本模块内部构造用（贴图窗的端到端用例要造一张真贴图）
    pub fn for_test(id: RequestId, path: PathBuf) -> Self {
        Self::new(id, path)
    }

    fn new(id: RequestId, path: PathBuf) -> Self {
        Self {
            id,
            path,
            recognize_gen: 0,
            stage: RecognitionStage::Idle,
            document: None,
            band_open: true,
            dragging: false,
            source: None,
            translate_stage: TranslateStage::Idle,
            translate_in_flight: None,
            failed_blocks: Vec::new(),
            compose_in_flight: None,
            product: None,
            compose_due: false,
        }
    }
}

/// 一次截图回信的落点结果，交给帧循环决定要不要开窗与派识别。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureApplied {
    /// 回信过期（被更晚的一次顶掉，或这一轮已经没有在飞标记）：**界面零变化**
    Stale,
    /// 用户取消：不建贴图、不改图片页任何东西
    Cancelled,
    /// 失败，带那句能行动的话。**"屏幕上已经钉满"也走这一档**（Q-007）：
    /// 那句话同样是一句能行动的提示（"先 esc 关掉一张"），而不是"截图失败了"
    Failed(String),
    /// 新建的贴图号。帧循环拿它去开窗（`system::pin::open`）并派"只识别"
    Pinned { pin: RequestId },
}

/// 同时能钉几张。**6 不是能力上限**（本机实测同时钉 8 张各自出图，事实 33），
/// 它是"屏幕上摆六张还找得着"那个判断（Q-007）。
///
/// 满了的行为是**拒绝新的那一张**，不是撤掉最老的那一张：用户正对着的那张凭空消失，
/// 比"这张没钉上，先关一张"糟糕得多——后者他一眼就看见、一句话就知道下一步。
pub const PIN_MAX: usize = 6;

/// 图片页识别段的五态（CHG-005 REQ-006）。
///
/// 只覆盖**识别那一段**。翻译段与合成段各有一条并列的（[`TranslateStage`]、[`ComposeStage`]）：
/// 三段各有各的过期序号，晚到的识别结果不许作废翻译段，把它们挤进一个枚举那条界线就没了。
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
    /// I 区那一行的主文案（耗时由界面按 `image_elapsed_ms` 另拼）
    pub fn label(&self) -> String {
        match self {
            RecognitionStage::Idle => "空闲".to_string(),
            RecognitionStage::Recognizing => "识别中…".to_string(),
            RecognitionStage::Done { blocks } => format!("完成 {blocks} 块"),
            RecognitionStage::NoText => "未发现文字".to_string(),
            RecognitionStage::Failed(reason) => format!("识别失败：{reason}"),
        }
    }

    /// 界面着色用那一档。"能不能停"不由这里判——那颗「停止」按钮三段共用，
    /// 看的是各自的在飞标记（[`AppState::stop_hint`]）。
    pub const fn phase(&self) -> Phase {
        match self {
            RecognitionStage::Recognizing => Phase::Busy,
            RecognitionStage::Done { .. } => Phase::Done,
            RecognitionStage::Failed(_) => Phase::Bad,
            // 「这张图没字」既不是坏也没结果，只是这一次什么也没认出来
            RecognitionStage::Idle | RecognitionStage::NoText => Phase::Idle,
        }
    }
}

/// 图片页翻译段的五态（CHG-006 REQ-009）。与 [`RecognitionStage`] 并列，两边文案互不挪用。
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
    /// I 区那一行的主文案（耗时由界面按 `translate_elapsed_ms` 另拼，与识别段同一拼法）
    pub fn label(&self) -> String {
        match self {
            TranslateStage::Idle => "空闲".to_string(),
            TranslateStage::Running { done, total } => format!("翻译中 {done}/{total} 块"),
            TranslateStage::Done { ok, failed } => tail("完成", *ok, *failed),
            TranslateStage::Stopped { ok, failed } => tail("已停止", *ok, *failed),
            TranslateStage::Failed(reason) => format!("翻译失败：{reason}"),
        }
    }

    pub const fn phase(&self) -> Phase {
        match self {
            TranslateStage::Running { .. } => Phase::Busy,
            // 停在块边界也算"有结果"：已翻好的那些块还在文档里（Q-006 裁定"留"）
            TranslateStage::Done { .. } | TranslateStage::Stopped { .. } => Phase::Done,
            TranslateStage::Failed(_) => Phase::Bad,
            TranslateStage::Idle => Phase::Idle,
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

/// 图片页合成段的四态（CHG-006 REQ-009）。
///
/// `Done` 不带任何计数：那四个数（回填/覆盖层/跳过/降级）与缺字形都在 [`Composed`] 里，
/// 抄一份进状态枚举就出现了"数字与成品各说各话"的可能。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ComposeStage {
    #[default]
    Idle,
    /// 一次同步计算，撤不回
    Composing,
    Done,
    Failed(String),
}

impl ComposeStage {
    pub fn label(&self) -> String {
        match self {
            ComposeStage::Idle => "空闲".to_string(),
            ComposeStage::Composing => "合成中…".to_string(),
            // 具体怎么画的（几块回填、几块覆盖层）由界面从 `composed` 上读，这里不重复
            ComposeStage::Done => "完成".to_string(),
            ComposeStage::Failed(reason) => format!("合成失败：{reason}"),
        }
    }

    pub const fn phase(&self) -> Phase {
        match self {
            ComposeStage::Composing => Phase::Busy,
            ComposeStage::Done => Phase::Done,
            ComposeStage::Failed(_) => Phase::Bad,
            ComposeStage::Idle => Phase::Idle,
        }
    }
}

/// 三段共用的"现在处在哪一档"（CHG-006 REQ-009）。
///
/// 三套状态枚举的变体名各说各话，界面要的颜色却只有四种：没开始、进行中、有结果、坏了。
/// 映射写在状态侧而不是界面侧，为的是加一态时不会只改一半——`TranslateStage::Stopped`
/// 是"有结果"（译文还在），把它当"坏了"红给用户看是假话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Busy,
    Done,
    Bad,
}

/// 那条共用「停止」入口按当前段给出的结论（REQ-009 的边界：一条入口，两种含义）。
///
/// 文案由这里给出而不是让界面自己拼，为的是"真停"和"丢弃结果"不许被写成同一句话——
/// 后者那次计算还在跑，说"已停止"是撒谎。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// 停在翻译段：块边界真停，后面的块不再发请求
    Translate,
    /// 停在识别段或合成段：只丢弃这一次的结果
    Ignore,
    /// 三段都不在飞，什么都没停
    Nothing,
}

impl Stop {
    /// 给用户的那句话；`Nothing` 时是 `None`——没有可报的停止就别报
    pub fn label(self) -> Option<&'static str> {
        match self {
            Stop::Translate => Some("已停止这次处理，译文还在"),
            Stop::Ignore => Some("已忽略这次结果，那次计算撤不回"),
            Stop::Nothing => None,
        }
    }
}

/// 一次"拆分块"的结果（Q-004 的裁定分成三档，界面按档给不同文案）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Split {
    /// 文字没改过：按行回溯，还原成 N 块
    Restored(usize),
    /// 改过：整段留在第一块，其余 N−1 块给空串，界面要提示核对
    KeptInFirst { pieces: usize },
    /// 拆不动：来源行未知，或本来就只有来源一行
    Nothing,
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

/// 拆分时从 `image_rows` 里捞出来的一行：行号 + 原文 + 本帧尺寸下的像素框 + 该行置信度。
/// 只在 [`AppState::split_block`] 内部当中间变量用。
struct RowPiece {
    row: u32,
    text: String,
    bbox: PixelRect,
    confidence: f32,
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
    // ---- 图片工作区（CHG-005：REQ-001 / REQ-005 / REQ-006） ----
    /// 当前这张入场图（S0 的产物）。**运行态**：重开软件就没了，不进配置也不落盘。
    pub image: Option<Admitted>,
    /// 上一次识别的行级观察，「拆分回原行」唯一的原料（FR-2.3）。随新结果整体替换。
    pub image_rows: Vec<RowObservation>,
    /// 编辑中的识别结果。五种编辑动作改的就是这一份——它是要交给 P7 的那一份（REQ-005）。
    pub ocr: Option<OcrDocument>,
    pub image_stage: RecognitionStage,
    /// 图片侧自己的序号与在飞标记：与文本翻译的 `request_id`/`in_flight` 互不相干（REQ-006）。
    pub image_request_id: RequestId,
    pub image_in_flight: Option<RequestId>,
    /// 上一次识别的耗时，I 区显示用（REQ-006 的"耗时可见"）
    pub image_elapsed_ms: u64,
    /// 手输路径那条入口的输入缓冲
    pub image_path_input: String,
    /// F 区刚被点中的那一块：H 区把那一行描一圈并滚进视野（REQ-007 的"点框定位到那一块"）。
    /// **运行态**，换一张图就清——块号只对当前这份文档有意义，留着会把不相干的一行描上圈
    pub selected_block: Option<u32>,
    // ---- 图片工作区：翻译段与合成段（CHG-006 REQ-001 / REQ-006 / REQ-009） ----
    /// 翻译段状态，与识别段并列不合并
    pub translate_stage: TranslateStage,
    /// 翻译段与合成段各自的序号与在飞标记，与识别段那对完全独立（REQ-009 边界那条）
    pub translate_request_id: RequestId,
    pub translate_in_flight: Option<RequestId>,
    /// 上一批逐块翻译的耗时，I 区显示用
    pub translate_elapsed_ms: u64,
    /// 这一批失败的块号。**运行态**：一次尝试里失败过不是那份文档的性质，
    /// 所以既不写进 `OcrDocument` 也不进配置，换一批就重来
    pub failed_blocks: Vec<u32>,
    /// 翻译段的块边界停止哨兵，随 [`Job::ImageTranslate`] 一起交给工作循环
    pub image_cancel: Arc<AtomicBool>,
    pub compose_stage: ComposeStage,
    pub compose_request_id: RequestId,
    pub compose_in_flight: Option<RequestId>,
    pub compose_elapsed_ms: u64,
    /// 上一次合成的成品。只在整张图画完之后一次性换代，界面上因此永远不会出现半张图（REQ-009）
    pub composed: Option<Composed>,
    /// 下一张成品用什么画法（REQ-006 的三档单选）。运行态、默认"自动"、不进配置：
    /// 它是对**这一张图**的一次性决定，不是用户对整个软件的偏好
    pub compose_mode: CompositeMode,
    /// ---- 截图与贴图（CHG-009 第四条通道） ----
    pub capture_stage: CaptureStage,
    pub capture_request_id: RequestId,
    pub capture_in_flight: Option<RequestId>,
    /// 历史文件名的自增段。**从 1 起**，0 留给"一次都没发过"那个读法
    pub capture_seq: u64,
    /// 屏幕上正钉着的图，按钉上的先后排列（老化与「打开上一次」都读这个次序）
    pub pins: Vec<Pin>,
    /// 下一个贴图号。关掉的不回收——号只往前走，晚到的回信因此不可能落在一张新图上
    pub next_pin_id: RequestId,
    /// 最近一次截图回信的**落点**，由帧循环取走（`take`）后清空：开窗与派"只识别"
    /// 都是碰系统的事，不能发生在状态层里（REQ-005 那条分层纪律）
    pub capture_applied: Option<CaptureApplied>,
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
            image: None,
            image_rows: Vec::new(),
            ocr: None,
            image_stage: RecognitionStage::Idle,
            image_request_id: 0,
            image_in_flight: None,
            image_elapsed_ms: 0,
            image_path_input: String::new(),
            selected_block: None,
            translate_stage: TranslateStage::Idle,
            translate_request_id: 0,
            translate_in_flight: None,
            translate_elapsed_ms: 0,
            failed_blocks: Vec::new(),
            image_cancel: Arc::new(AtomicBool::new(false)),
            compose_stage: ComposeStage::Idle,
            compose_request_id: 0,
            compose_in_flight: None,
            compose_elapsed_ms: 0,
            composed: None,
            compose_mode: CompositeMode::default(),
            capture_stage: CaptureStage::Idle,
            capture_request_id: 0,
            capture_in_flight: None,
            capture_seq: 0,
            pins: Vec::new(),
            next_pin_id: 0,
            capture_applied: None,
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
            Outcome::Image { id, result } => self.apply_image(id, result),
            // 进度也按归属落地：贴图在翻的时候，图片页那格进度条不该跟着跳
            Outcome::ImageTranslateProgress {
                id,
                owner,
                done,
                total,
            } => match owner {
                ChainOwner::Page => self.apply_translate_progress(id, done, total),
                ChainOwner::Pin(pin) => {
                    if let Some(target) = self.pins.iter_mut().find(|p| p.id == pin) {
                        if target.translate_in_flight == Some(id) {
                            target.translate_stage = TranslateStage::Running { done, total };
                        }
                    }
                    true
                }
            },
            // 同一份回信按归属落地：图片页那一份工作区，还是某张贴图自己那一份。
            // 这是"一条实现、两个落点"的另一半——派发出去的是同一批 Job 变体。
            Outcome::ImageTranslated {
                id,
                owner,
                results,
                cancelled,
                elapsed_ms,
            } => match owner {
                ChainOwner::Page => self.apply_image_translated(id, results, cancelled, elapsed_ms),
                ChainOwner::Pin(pin) => {
                    self.apply_pin_translated(pin, id, results, cancelled, elapsed_ms)
                }
            },
            Outcome::ImageComposed {
                id,
                owner,
                result,
                elapsed_ms,
            } => match owner {
                ChainOwner::Page => self.apply_image_composed(id, result, elapsed_ms),
                ChainOwner::Pin(pin) => self.apply_pin_composed(pin, id, result),
            },
            // 截图与贴图那两条：回信**只改状态**，开窗与派识别发生在帧循环里
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

    // ---- 图片工作区（CHG-005 REQ-001/005/006） ----

    /// 派发一次识别。图片侧的序号与文本侧各走各的：一次翻译不能让晚到的识别结果作废，
    /// 反过来也一样（REQ-006 第四条）。
    ///
    /// 同一时刻图片侧只允许一次识别在飞；再次导入等于换图，先按 `restart` 把上一次记为过期。
    pub fn submit_image(&mut self, input: ImageInput) -> Option<Job> {
        if matches!(self.image_stage, RecognitionStage::Recognizing) {
            return None;
        }
        self.image_request_id += 1;
        let id = self.image_request_id;
        self.image_in_flight = Some(id);
        self.image_stage = RecognitionStage::Recognizing;
        // 换图即作废：上一张图的行、块、耗时全部清掉，界面上不留"新图配旧结果"的画面
        self.image_rows.clear();
        self.ocr = None;
        self.image_elapsed_ms = 0;
        self.selected_block = None;
        // 下游两段跟着一起作废。它们的结果都是**这张**文档的译文与成品；在飞的那一批
        // 只把在飞标记摘掉，晚到的结果因此按序号丢弃（那批请求撤不回，但落不到新图上）。
        self.expire_translate();
        self.expire_compose();
        self.composed = None;
        Some(Job::ImageOcr { id, input })
    }

    /// 点「忽略这次识别」：立刻回空闲，晚到的结果按序号丢弃（Q-008——底层那次调用跑不完也不回收）。
    ///
    /// 返回 `false` 表示当前没有在飞的识别，调用方不该给出"已忽略"这种凭空的话。
    /// 这里刻意**不**清空 `image` / `image_rows` / `ocr`：忽略的只是这一次结果，
    /// 上一张图和它的结果（若有）还在原位。
    pub fn ignore_image_result(&mut self) -> bool {
        if self.image_in_flight.take().is_none() {
            return false;
        }
        self.image_stage = RecognitionStage::Idle;
        true
    }

    /// 落地一次识别结果。`false` = 序号对不上（已被忽略或被更新的导入顶掉），界面零变化。
    fn apply_image(
        &mut self,
        id: RequestId,
        result: Result<ImageRecognition, VisionError>,
    ) -> bool {
        if self.image_in_flight != Some(id) {
            return false;
        }
        self.image_in_flight = None;
        match result {
            Ok(recognition) => {
                self.image_elapsed_ms = recognition.elapsed_ms;
                self.image = Some(recognition.admitted);
                self.image_rows = recognition.rows;
                self.ocr = Some(recognition.document);
                self.image_stage = RecognitionStage::Done {
                    blocks: self.ocr.as_ref().map(|d| d.blocks.len()).unwrap_or(0),
                };
            }
            // `NoTextFound` 走单独一态：图没错、机器没坏，是这张图上真没有可读的字
            Err(VisionError::NoTextFound) => {
                self.image_stage = RecognitionStage::NoText;
                self.image_rows.clear();
                self.ocr = None;
            }
            Err(err) => {
                self.image_stage = RecognitionStage::Failed(image_failure_text(&err));
                self.image_rows.clear();
                self.ocr = None;
            }
        }
        true
    }

    // ---- 图片工作区：翻译段与合成段（CHG-006 REQ-001 / REQ-006 / REQ-009） ----

    /// 派发一批逐块翻译（REQ-001）。`ids` 为空 = 所有 `enabled` 块，也就是「翻译全部」；
    /// 给了块号 = 只翻这些块，「重试这一块」（单个）与「重试失败块」（一批）都走这一条。
    ///
    /// 三道前置门：这一批没在飞、有可翻的块、定得出具体方向。前两道不过就什么都不动
    /// （界面那两颗按钮本来就该是灰的）；第三道不过要在段上留一句话，因为用户从界面上
    /// 看不出"自动检测"为什么翻不动。
    ///
    /// 同一时刻只允许一批在飞（REQ-001 第三条）。再次派发等于换一批译文，上一张成品当场
    /// 作废——留着它就是"旧图配新译文"，比空着一格更坏。
    pub fn submit_image_translate(&mut self, ids: &[u32]) -> Option<Job> {
        if self.translate_in_flight.is_some() {
            return None;
        }
        let whole_batch = ids.is_empty();
        let pending: Vec<(u32, String)> = self
            .ocr
            .as_ref()?
            .enabled_blocks()
            .filter(|block| whole_batch || ids.contains(&block.id))
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
        let Some(pair) = self.image_direction(&sample) else {
            self.translate_stage = TranslateStage::Failed(NO_DIRECTION_TEXT.to_string());
            return None;
        };
        // 方向定下来之后才把每块拼成请求：交给工作循环的已经是"能直接发出去的那一份"
        let blocks = pending
            .into_iter()
            .map(|(id, text)| (id, TranslationRequest::new(text, pair)))
            .collect::<Vec<_>>();
        self.translate_request_id += 1;
        let id = self.translate_request_id;
        self.translate_in_flight = Some(id);
        self.translate_stage = TranslateStage::Running {
            done: 0,
            total: blocks.len(),
        };
        self.translate_elapsed_ms = 0;
        // 整批重翻时失败台账清零；只翻几块时留着不动——那些块这次没被试过，
        // 说它"已经翻出来了"是假话（台账按块号在 `apply_image_translated` 里逐块销账）
        if whole_batch {
            self.failed_blocks.clear();
        }
        // 哨兵先落回"没停"：上一批的停止不该让这一批在第一块就收手
        self.image_cancel.store(false, Ordering::Relaxed);
        self.expire_compose();
        self.composed = None;
        Some(Job::ImageTranslate {
            id,
            owner: ChainOwner::Page,
            engine: self.engine,
            blocks,
            cancel: Arc::clone(&self.image_cancel),
        })
    }

    /// 重试这一批里所有失败的块（一颗按钮的给法：逐块一颗太密，H 区用的是这一条 + 单块那条）。
    /// 派发不出去就把台账还回去——失败块列表不该因为"点了但没发出去"而变短。
    ///
    /// 台账为空时**必须**直接返回：空块号列表在 [`AppState::submit_image_translate`] 里的意思是
    /// "整批重翻"，少了这道门这颗按钮就变成"再翻一遍全部"。
    pub fn submit_image_retry_failed(&mut self) -> Option<Job> {
        if self.failed_blocks.is_empty() {
            return None;
        }
        let ids = std::mem::take(&mut self.failed_blocks);
        let job = self.submit_image_translate(&ids);
        if job.is_none() {
            self.failed_blocks = ids;
        }
        job
    }

    /// 派发一次合成（REQ-003~REQ-006）。要有入场图、有文档、没有在飞的合成，
    /// 还要**至少一块有译文**：一条译文都没有就擦字重画，出来的是一张被擦花的原图。
    ///
    /// 已有的成品此刻不动（REQ-009 第四条）：新成品要等整张图画完才一次性换代，
    /// 所以界面上从不会出现画了一半的那一帧。
    pub fn submit_image_compose(&mut self) -> Option<Job> {
        if self.compose_in_flight.is_some() {
            return None;
        }
        let image = self.image.as_ref()?;
        let document = self.ocr.as_ref()?;
        let anything_translated = document
            .enabled_blocks()
            .any(|block| block.translated_text().is_some());
        if !anything_translated {
            return None;
        }
        self.compose_request_id += 1;
        let id = self.compose_request_id;
        self.compose_in_flight = Some(id);
        self.compose_stage = ComposeStage::Composing;
        self.compose_elapsed_ms = 0;
        Some(Job::ImageCompose {
            id,
            owner: ChainOwner::Page,
            source: image.image.clone(),
            document: document.clone(),
            mode: self.compose_mode,
        })
    }

    // ---- 区域截图与贴图（CHG-009 REQ-001/002/004/005，第四条通道） ----

    /// 发起一次框选。图片页那颗「框选贴图」与全局热键**共用这一个入口**——
    /// 两条路迟早会不一样（图钉那次的教训），所以热键命中后也只是调它。
    ///
    /// 一次框选在飞时**不发起第二次**（REQ-001 末句：后到的直接忽略）。这一条不靠
    /// `capture_gate` 的灰按钮兜住，因为热键那一下不经过按钮。
    pub fn submit_capture(&mut self) -> Option<Job> {
        if self.capture_in_flight.is_some() {
            return None;
        }
        self.capture_request_id += 1;
        let id = self.capture_request_id;
        self.capture_seq += 1;
        self.capture_in_flight = Some(id);
        self.capture_stage = CaptureStage::Capturing;
        Some(Job::ScreenCapture {
            id,
            seq: self.capture_seq,
        })
    }

    /// 那颗按钮灰不灰与状态行说哪句话，**读同一个来源**（与 `translate_gate` /
    /// `compose_gate` 同形）。三道门从上往下依次是：这台机器没这件事、上一次还在画、
    /// 屏幕上钉满了。
    pub fn capture_gate(&self) -> Option<&'static str> {
        if !crate::system::capture::capability().capture {
            return Some("这台机器没有可用的框选截图；可先用系统截图再点「粘贴」");
        }
        if self.capture_in_flight.is_some() {
            return Some("上一次框选还没画完");
        }
        if self.pins.len() >= PIN_MAX {
            return Some("屏幕上已经钉满 6 张，先 esc 关掉一张");
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
                // 满了：**图仍然留在历史里**（老化照常），只是不再往屏幕上钉。
                // 撤掉用户已经钉着的那一张来腾位置是不可接受的（Q-007）。
                if self.pins.len() >= PIN_MAX {
                    let text = "屏幕上已经钉满 6 张，先 esc 关掉一张".to_string();
                    self.capture_stage = CaptureStage::Failed(text.clone());
                    return CaptureApplied::Failed(text);
                }
                let pin = self.push_pin(path);
                self.capture_stage = CaptureStage::Pinned;
                match pin {
                    Some(id) => CaptureApplied::Pinned { pin: id },
                    None => {
                        // 只有并发才可能到这里（同一帧内两次 push），保底说"钉不上"
                        let text = "这张没钉上，先 esc 关掉一张".to_string();
                        self.capture_stage = CaptureStage::Failed(text.clone());
                        CaptureApplied::Failed(text)
                    }
                }
            }
        }
    }

    /// 记下一张新的贴图，返回它的号。满了返回 `None`（调用方负责给那句话）。
    ///
    /// 号只往前走、不复用：窗口关掉之后那个号不会被下一张顶替，所以任何一条晚到的回信
    /// 都不可能落在一张新贴图上（[`Pin::recognize_gen`] 是同一族纪律的第二层）。
    pub fn push_pin(&mut self, path: PathBuf) -> Option<RequestId> {
        if self.pins.len() >= PIN_MAX {
            return None;
        }
        self.next_pin_id += 1;
        let id = self.next_pin_id;
        self.pins.push(Pin::new(id, path));
        Some(id)
    }

    /// 「打开上一次截图」（F2 那一格）要的那张：**与老化共用同一个排序**，
    /// 所以"最近"在这条链上只有一个定义。没有历史时返回 `None`，界面给"还没有截图记录"。
    pub fn newest_pin_source(&self) -> Option<&Pin> {
        self.pins.last()
    }

    /// 关掉一张：从状态里摘掉，返回是否真摘掉了（`false` = 那个号本来就不在，
    /// 调用方不该为它说"已关闭"）。窗口的关闭命令由帧循环发，这里不碰 egui。
    pub fn close_pin(&mut self, id: RequestId) -> bool {
        let before = self.pins.len();
        self.pins.retain(|pin| pin.id != id);
        self.pins.len() != before
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

    /// 那条共用的「停止」（REQ-009 的边界：一条入口，两种含义）。
    ///
    /// 返回的是"这次停的是哪一段"，界面据此给文案：翻译段是真停（译文留在原处），
    /// 识别段与合成段只是丢弃结果——那两次计算还在跑，说"已停止"是撒谎。
    /// 三段各自都在飞时才走到前面的那一支，而换图会把翻译段记过期，所以实际不会同时有两支在飞。
    pub fn stop_image_stage(&mut self) -> Stop {
        if self.image_in_flight.is_some() {
            self.ignore_image_result();
            return Stop::Ignore;
        }
        if self.translate_in_flight.is_some() {
            // 只立哨兵，不改段的状态：在飞的那一次请求撤不回，停下与结局由回来的那条结果判定
            self.image_cancel.store(true, Ordering::Relaxed);
            return Stop::Translate;
        }
        if self.compose_in_flight.is_some() {
            self.expire_compose();
            return Stop::Ignore;
        }
        Stop::Nothing
    }

    /// E 区「翻译全部」缺的那一步（REQ-006 后半句：缺哪一步就在状态行说哪一步）。
    ///
    /// 返回的是**给用户看的那句话**，`None` = 这一颗该是能点的。置灰和状态行读同一个来源，
    /// 不允许出现"按钮灰着、状态行却说一切正常"。定不出方向那一类只有真去派发才知道，
    /// 它落在 [`TranslateStage::Failed`] 上，不在这里预支。
    pub fn translate_gate(&self) -> Option<&'static str> {
        if self.translate_in_flight.is_some() {
            Some("这一批还在翻，稍等")
        } else if self.enabled_block_count() == 0 {
            Some("先识别出至少一块文字")
        } else {
            None
        }
    }

    /// E 区「生成图片」缺的那一步，三道门与 [`AppState::submit_image_compose`] 一一对应。
    pub fn compose_gate(&self) -> Option<&'static str> {
        if self.compose_in_flight.is_some() {
            Some("这一张还在画，稍等")
        } else if self.image.is_none() {
            Some("先导入一张图")
        } else if self.translated_block_count() == 0 {
            Some("先翻出至少一块译文")
        } else {
            None
        }
    }

    /// E 区「保存」「复制图片」缺的那一步（REQ-008 的两条出路）。
    ///
    /// 只有一道门：**手上得有一张成品**。合成在飞时这道门照样开着——那一刻 G 区显示的是
    /// 上一张成品，存出去的就是它；"所见即所存"比"再等一秒"诚实，那半句写在那两颗的 hover 里。
    pub fn export_gate(&self) -> Option<&'static str> {
        if self.composed.is_none() {
            Some("先生成一张成品图")
        } else {
            None
        }
    }

    /// 那颗「停止」此刻对着哪一段，以及该给它哪句 hover（三段共用一颗按钮，含义不同）。
    ///
    /// `None` = 三段都不在飞，这颗按钮本来就不该画出来。判定顺序与
    /// [`AppState::stop_image_stage`] 必须一致，所以两边共用这一条：界面上说"停的是谁"
    /// 和按下去真停的是谁不能各说各话。
    pub fn stop_hint(&self) -> Option<&'static str> {
        if self.image_in_flight.is_some() {
            Some(RECOGNIZE_STOP_HINT)
        } else if self.translate_in_flight.is_some() {
            Some(TRANSLATE_STOP_HINT)
        } else if self.compose_in_flight.is_some() {
            Some(COMPOSE_STOP_HINT)
        } else {
            None
        }
    }

    /// 当前这份文档里参与翻译的块数：勾掉的不算，空文也不算（`enabled_blocks` 那一口径）
    pub fn enabled_block_count(&self) -> usize {
        self.ocr
            .as_ref()
            .map(|document| document.enabled_blocks().count())
            .unwrap_or(0)
    }

    /// 其中已经有**可画**译文的块数
    pub fn translated_block_count(&self) -> usize {
        self.ocr
            .as_ref()
            .map(|document| {
                document
                    .enabled_blocks()
                    .filter(|block| block.translated_text().is_some())
                    .count()
            })
            .unwrap_or(0)
    }

    /// 翻译段记为过期并回空闲：晚到的结果落不到当前文档上，失败块号也不再有意义
    fn expire_translate(&mut self) {
        self.translate_in_flight = None;
        self.translate_stage = TranslateStage::Idle;
        self.failed_blocks.clear();
    }

    /// 合成段记为过期并回空闲。成品动不动由调用方决定：停止不动（用户手上那张还是他要的），
    /// 换图与换一批译文要动（它对应的是上一张图）。
    fn expire_compose(&mut self) {
        self.compose_in_flight = None;
        self.compose_stage = ComposeStage::Idle;
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

    /// 逐块翻译的一段推进。序号对不上就直接忽略：晚到的进度不该把已经停下或已经完成的段
    /// 重新写成"进行中"。
    fn apply_translate_progress(&mut self, id: RequestId, done: usize, total: usize) -> bool {
        if self.translate_in_flight != Some(id) {
            return false;
        }
        self.translate_stage = TranslateStage::Running { done, total };
        true
    }

    /// 落地一批逐块翻译的结果。`false` = 序号对不上（已过期或被更新的一批顶掉）。
    ///
    /// `Ok` 的写进对应块的 `translation`，`Err` 的只进 [`AppState::failed_blocks`]：
    /// "这一块这次没翻出来"是这次尝试的性质，不是那份文档的性质。失败台账按块号销账而不是
    /// 整表清空——只重试两块时，另外三块的失败记录还得上着，它们这次没被试过。
    fn apply_image_translated(
        &mut self,
        id: RequestId,
        results: Vec<(u32, Result<String, ProviderError>)>,
        cancelled: bool,
        elapsed_ms: u64,
    ) -> bool {
        if self.translate_in_flight != Some(id) {
            return false;
        }
        self.translate_in_flight = None;
        self.translate_elapsed_ms = elapsed_ms;
        let mut ok = 0;
        if let Some(document) = self.ocr.as_mut() {
            ok = land_translations(document, results, &mut self.failed_blocks);
        }
        let failed = self.failed_blocks.len();
        self.translate_stage = if cancelled {
            TranslateStage::Stopped { ok, failed }
        } else {
            TranslateStage::Done { ok, failed }
        };
        true
    }

    /// 一张贴图按下「翻译」：走的是**同一条链**（识别 → 翻译 → 回填 → 成品原地替换），
    /// 只是文档、在飞标记与成品都存在这张自己身上。
    ///
    /// 前置门与图片页那条一样三道（没在飞、有可翻的块、定得出方向）；第三道不过要把话留在
    /// **这张的**翻译段上，不是图片页的状态行——他正看着的是屏上那张。
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
            owner: ChainOwner::Pin(pin),
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
                owner: ChainOwner::Pin(target.id),
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

    /// 落地一次合成的结果。`false` = 序号对不上，界面零变化。
    fn apply_image_composed(
        &mut self,
        id: RequestId,
        result: Result<Composed, VisionError>,
        elapsed_ms: u64,
    ) -> bool {
        if self.compose_in_flight != Some(id) {
            return false;
        }
        self.compose_in_flight = None;
        self.compose_elapsed_ms = elapsed_ms;
        match result {
            Ok(composed) => {
                self.composed = Some(composed);
                self.compose_stage = ComposeStage::Done;
            }
            // 失败不动上一张成品：用户没要求销毁他手上已经有的那张（REQ-009 第四条）
            Err(err) => self.compose_stage = ComposeStage::Failed(image_failure_text(&err)),
        }
        true
    }

    /// 手输路径那条入口：交出路径这个**动作**由界面发起，读盘在工作线程，所以这里只造任务。
    /// 空串什么都不做（点了确定但框里没字，不该报"找不到文件"）。
    pub fn submit_image_path(&mut self) -> Option<Job> {
        let path = self.image_path_input.trim();
        if path.is_empty() {
            return None;
        }
        self.submit_image(ImageInput::Path(PathBuf::from(path.to_string())))
    }

    /// 当前要交给 P7 的那份文本（REQ-005 的"改动确实进了数据"就断言这一条）。
    /// 没有识别结果时是空串——与 `OcrDocument::merged_text` 同口径。
    pub fn image_merged_text(&self) -> String {
        self.ocr
            .as_ref()
            .map(|document| document.merged_text())
            .unwrap_or_default()
    }

    /// ① 改字（FR-2.3 的"修正错字"）。返回 `false` = 这一块已经不在列表里了。
    pub fn edit_block_text(&mut self, id: u32, text: String) -> bool {
        let Some(doc) = self.ocr.as_mut() else {
            return false;
        };
        match doc.blocks.iter_mut().find(|block| block.id == id) {
            Some(block) => {
                block.text = text;
                true
            }
            None => false,
        }
    }

    /// ② 勾选（FR-2.3 的"删掉不要的块"之一，另一条是③真删）。
    /// 只动 `enabled`：块还在列表里，用户可以再勾回来，但 `enabled_blocks`/`merged_text` 立刻不算它。
    ///
    /// 给的是"设成这个值"而不是"翻一下"：界面那头的复选框本来就知道用户要的是哪一头，
    /// 由它说出口，状态层就不必猜上一次它是什么。
    pub fn set_block_enabled(&mut self, id: u32, enabled: bool) -> bool {
        let Some(doc) = self.ocr.as_mut() else {
            return false;
        };
        match doc.blocks.iter_mut().find(|block| block.id == id) {
            Some(block) => {
                block.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// ③ 删除整块（FR-2.3 的"删除误识别块"）。删完重排块号，界面上的序号不会跳出一个空洞。
    pub fn remove_block(&mut self, id: u32) -> bool {
        let Some(doc) = self.ocr.as_mut() else {
            return false;
        };
        let Some(position) = doc.blocks.iter().position(|block| block.id == id) else {
            return false;
        };
        doc.blocks.remove(position);
        Self::renumber_blocks(doc);
        true
    }

    /// ④ 合并这一块与它的**下一块**（FR-2.3）。
    ///
    /// 判据与 REQ-004 同形但不受它约束：用户手动点就是要越过五条判据，所以这里不看间距、
    /// 不看对齐，只按"文本按行序拼接、框取 `union`、置信度按字数加权、行回溯取并集"合成。
    /// 勾选状态跟随第一块——合并发生在第一块的位置上，用户看到的也是那一行。
    pub fn merge_block(&mut self, id: u32) -> bool {
        let Some(doc) = self.ocr.as_mut() else {
            return false;
        };
        let Some(position) = doc.blocks.iter().position(|block| block.id == id) else {
            return false;
        };
        if position + 1 >= doc.blocks.len() {
            return false;
        }
        let right = doc.blocks.remove(position + 1);
        let left = &mut doc.blocks[position];
        let left_chars = left.text.chars().count() as f32;
        let right_chars = right.text.chars().count() as f32;
        let total = left_chars + right_chars;
        left.confidence = if total > 0.0 {
            (left.confidence * left_chars + right.confidence * right_chars) / total
        } else {
            left.confidence
        };
        left.text = format!("{}\n{}", left.text, right.text);
        left.bbox = left.bbox.union(right.bbox);
        // 并集：两块各自退回原行时不该出现同一行被拆成两个块
        for row in right.source_rows {
            if !left.source_rows.contains(&row) {
                left.source_rows.push(row);
            }
        }
        left.source_rows.sort_unstable();
        Self::renumber_blocks(doc);
        true
    }

    /// ⑤ 拆分回来源行（FR-2.3，判据见 Q-004 的裁定）。
    ///
    /// 判"改没改过"用的是**内容比对**而不是脏标记：合并时文本就是各行按 `\n` 拼接，
    /// 所以拼回去还相等 ⟺ 用户一个字符都没动。改过就不猜字符比例——整段留在第一块，
    /// 其余块给空串（`enabled_blocks` 天然跳过空串，不会把空块送进 P7），并由调用方提示核对。
    pub fn split_block(&mut self, id: u32) -> Split {
        let Some(document) = self.ocr.as_ref() else {
            return Split::Nothing;
        };
        let Some(position) = document.blocks.iter().position(|block| block.id == id) else {
            return Split::Nothing;
        };
        let parent = document.blocks[position].clone();
        let (width, height) = (document.width, document.height);
        if parent.source_rows.len() < 2 {
            return Split::Nothing;
        }
        // 来源行对不上（例如换过图却没重认）：不硬拆，交回界面说明"拆不回"。
        // `collect::<Option<Vec<_>>>()` 的语义正是"缺任何一行就整体放弃"
        let Some(pieces) = parent
            .source_rows
            .iter()
            .map(|&row| {
                let observation = self.image_rows.get(row as usize)?;
                Some(RowPiece {
                    row,
                    text: observation.text.clone(),
                    bbox: flip_rect(observation.normalized_box, width, height),
                    confidence: observation.confidence,
                })
            })
            .collect::<Option<Vec<_>>>()
        else {
            return Split::Nothing;
        };

        let faithful = parent.text
            == pieces
                .iter()
                .map(|piece| piece.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
        let mut blocks: Vec<TextBlock> = if faithful {
            pieces
                .into_iter()
                .map(|piece| {
                    TextBlock::with_rows(
                        0,
                        piece.text,
                        piece.bbox,
                        piece.confidence,
                        vec![piece.row],
                    )
                })
                .collect()
        } else {
            pieces
                .iter()
                .enumerate()
                .map(|(index, piece)| {
                    if index == 0 {
                        // 整段留在第一块，框沿用父块：这一段的字确实铺满父块的位置
                        TextBlock::with_rows(
                            0,
                            parent.text.clone(),
                            parent.bbox,
                            parent.confidence,
                            parent.source_rows.clone(),
                        )
                    } else {
                        TextBlock::with_rows(0, "", piece.bbox, piece.confidence, vec![piece.row])
                    }
                })
                .collect()
        };
        let count = blocks.len();
        for block in &mut blocks {
            block.enabled = parent.enabled;
        }
        if let Some(document) = self.ocr.as_mut() {
            document.blocks.splice(position..position + 1, blocks);
            Self::renumber_blocks(document);
        }
        if faithful {
            Split::Restored(count)
        } else {
            Split::KeptInFirst { pieces: count }
        }
    }

    /// 结构变更后重排块号：`id` 与列表下标保持一致，界面按 id 找块才不会找错行。
    fn renumber_blocks(doc: &mut OcrDocument) {
        for (index, block) in doc.blocks.iter_mut().enumerate() {
            block.id = index as u32;
        }
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

#[cfg(test)]
mod tests {
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
        st.push_pin(PathBuf::from("/tmp/history/pin.png"))
            .expect("能钉一张")
    }

    /// 贴图与图片页各有一套链上状态：**贴图在翻，图片页的按钮不许因此变灰**；
    /// 反过来图片页在飞也不该让贴图这批派不出去。这是"借一份工作区"最直接的后果。
    #[test]
    fn the_pin_chain_and_the_page_chain_do_not_block_each_other() {
        let mut st = capture_state();
        let pin = recognized_pin(&mut st);
        st.pins[0].document = Some(pin_doc());
        let (admitted, _) = pixels_and_doc(600, 400);
        st.pins[0].source = Some(admitted);

        // 图片页先占住它自己那一段
        st.ocr = Some(pin_doc());
        let page_job = st.submit_image_translate(&[]).expect("图片页该能派一批");
        assert!(st.translate_in_flight.is_some());

        let pin_job = st.submit_pin_translate(pin).expect("贴图那一份该照派得出");
        match (&page_job, &pin_job) {
            (
                Job::ImageTranslate {
                    id: page_id,
                    owner: page_owner,
                    ..
                },
                Job::ImageTranslate {
                    id: pin_id,
                    owner: pin_owner,
                    ..
                },
            ) => {
                assert_eq!(*page_owner, ChainOwner::Page);
                assert_eq!(*pin_owner, ChainOwner::Pin(pin));
                assert_ne!(page_id, pin_id, "两条链共用一个号，回信就分不开是谁的");
            }
            other => panic!("派发的必须是逐块翻译：{other:?}"),
        }

        // 图片页那一批没在飞了（贴图这批仍在），互不影响
        assert!(st.pins[0].translate_in_flight.is_some());
        let page_id = match page_job {
            Job::ImageTranslate { id, .. } => id,
            _ => unreachable!(),
        };
        assert!(st.apply_outcome(Outcome::ImageTranslated {
            id: page_id,
            owner: ChainOwner::Page,
            results: vec![(0, Ok("你好世界".to_string()))],
            cancelled: false,
            elapsed_ms: 3,
        }));
        assert_eq!(
            st.pins[0].translate_stage,
            TranslateStage::Running { done: 0, total: 1 },
            "图片页的回信把贴图的段状态改了"
        );
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
        let pin = st.push_pin(file.clone()).unwrap();
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
            owner: ChainOwner::Pin(pin),
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
            Job::ImageCompose { id, owner, .. } => {
                assert_eq!(*owner, ChainOwner::Pin(pin));
                *id
            }
            other => panic!("{other:?}"),
        };
        assert!(!st.pins[0].compose_due, "派过就该销账，不许每帧重复派");

        let product = a_composed(1);
        assert!(st.apply_outcome(Outcome::ImageComposed {
            id: compose_id,
            owner: ChainOwner::Pin(pin),
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
            owner: ChainOwner::Pin(pin),
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
            owner: ChainOwner::Pin(pin),
            results: vec![(0, Ok("没了对象".to_string()))],
            cancelled: false,
            elapsed_ms: 1,
        }));
        // 派一次翻译，不会凭空造出一张贴图
        assert!(st.pins.is_empty());
        assert!(st.submit_pin_translate(pin).is_none());
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
        let job = st.submit_capture().expect("空闲时该发得出框选任务");
        assert!(matches!(job, Job::ScreenCapture { id: 1, seq: 1 }));
        let applied = st.apply_capture(1, capture::Outcome::Captured(path.clone()));
        assert_eq!(applied, CaptureApplied::Pinned { pin: 1 });
        assert_eq!(st.pins.len(), 1);
        assert_eq!(st.pins[0].path, path);
        assert_eq!(st.capture_stage, CaptureStage::Pinned);
        // 翻译段与合成段一个都没被这条回信碰过
        assert!(st.translate_in_flight.is_none());
        assert_eq!(st.compose_stage, ComposeStage::Idle);
        assert!(st.composed.is_none());

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

    /// 取消那一支：**贴图不建、图片页现有内容一个都不动**（REQ-002 与面板 `Cancelled` 同规）。
    /// 所以这里先故意摆一张已有的图与文档，回信之后必须还在原处。
    #[test]
    fn a_cancelled_reply_touches_nothing_but_its_own_stage() {
        let mut st = capture_state();
        st.ocr = Some(OcrDocument::empty(64, 48));
        st.capture_in_flight = Some(1);
        st.capture_stage = CaptureStage::Capturing;

        let applied = st.apply_capture(1, capture::Outcome::Cancelled);
        assert_eq!(applied, CaptureApplied::Cancelled);
        assert!(st.pins.is_empty(), "取消了就不该有贴图");
        assert_eq!(st.capture_stage, CaptureStage::Cancelled);
        assert!(st.ocr.is_some(), "取消不许把图片页已有的东西清掉");
        assert!(st.capture_in_flight.is_none());
    }

    /// 一次框选在飞时**不发起第二次**：后到的直接忽略。这条不靠按钮变灰兜住——
    /// 热键那一下不经过按钮。
    #[test]
    fn a_second_capture_while_one_is_in_flight_is_ignored() {
        let mut st = capture_state();
        let first = st.submit_capture();
        assert!(first.is_some());
        assert!(st.submit_capture().is_none(), "在飞时第二次派发必须被拒");
        assert_eq!(
            st.capture_gate(),
            Some("上一次框选还没画完"),
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

    /// 满 6 张时**拒绝新的那一张**，而且不动已经钉着的任何一张（Q-007）。
    /// 撤最老那张来腾位置是不可接受的：用户正对着的那张会凭空消失。
    #[test]
    fn the_seventh_pin_is_refused_without_disturbing_the_six_already_there() {
        let mut st = capture_state();
        let before: Vec<PathBuf> = (1..=PIN_MAX as u64)
            .map(|i| PathBuf::from(format!("/tmp/history/{i}.png")))
            .collect();
        for path in &before {
            st.push_pin(path.clone()).expect("前六张该钉得上");
        }
        assert_eq!(st.pins.len(), PIN_MAX);
        assert_eq!(
            st.capture_gate(),
            Some("屏幕上已经钉满 6 张，先 esc 关掉一张")
        );

        st.capture_in_flight = Some(1);
        let applied = st.apply_capture(
            1,
            capture::Outcome::Captured(PathBuf::from("/tmp/history/7.png")),
        );
        assert!(
            matches!(applied, CaptureApplied::Failed(_)),
            "第七张要被拒绝"
        );
        // 已经钉着的六张：一张不少、次序不变、路径没被换
        assert_eq!(st.pins.len(), PIN_MAX);
        assert_eq!(
            st.pins.iter().map(|p| p.path.clone()).collect::<Vec<_>>(),
            before
        );
        assert!(st.pins.iter().all(|p| p.id <= PIN_MAX as RequestId));
    }

    /// 贴图的识别按**代**丢弃：重派之后上一代的回信整条作废，
    /// 界面那一格保持它现在的样子（第四条序号互不作废那条纪律的第四处）。
    #[test]
    fn a_late_pin_reply_cannot_overwrite_the_newer_generation() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/a.png"));
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
        st.push_pin(PathBuf::from("/tmp/history/b.png"));
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
        st.push_pin(PathBuf::from("/tmp/history/1.png"));
        st.push_pin(PathBuf::from("/tmp/history/2.png"));
        assert!(st.close_pin(1));
        assert!(!st.close_pin(1), "再关一次要说没关掉（它本来就不在）");
        assert!(!st.close_pin(99), "不存在的号不许装作成功");
        assert_eq!(st.pins.len(), 1);
        assert_eq!(st.pins[0].id, 2);
        // 关过的号不会被下一张顶替
        assert_eq!(st.push_pin(PathBuf::from("/tmp/history/3.png")), Some(3));
    }

    /// 带子开合只认真存在的贴图（REQ-003 那条"收起=窗口缩到图片本身"的状态面）。
    #[test]
    fn the_band_toggle_refuses_a_pin_that_is_not_there() {
        let mut st = capture_state();
        st.push_pin(PathBuf::from("/tmp/history/1.png"));
        assert!(st.pins[0].band_open, "刚钉上时带子是开着的");
        assert!(st.set_pin_band(1, false));
        assert!(!st.pins[0].band_open);
        assert!(!st.set_pin_band(77, true));
    }

    /// 那颗按钮的话与状态行读**同一个来源**：三道门从上往下——这台机器没这件事、
    /// 上一次还在画、屏幕上钉满了。这里锁后两道（第一道要换平台才看得到）。
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
        st.submit_capture();
        assert_eq!(st.capture_gate(), Some("上一次框选还没画完"));
        st.capture_in_flight = None;
        for i in 0..PIN_MAX {
            st.push_pin(PathBuf::from(format!("/tmp/history/{i}.png")));
        }
        assert_eq!(
            st.capture_gate(),
            Some("屏幕上已经钉满 6 张，先 esc 关掉一张")
        );
        // 关掉一张，门就开了（拒绝的前提是"满"，不是"截图这件事坏了"）
        st.close_pin(1);
        assert_eq!(st.capture_gate(), None);
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

    /// 四支里只有文本那一支带请求，这几条测试断的全是它——取不出来就说明用错了分支。
    fn request_of(job: &Job) -> &TranslationRequest {
        match job {
            Job::Translation { request, .. } => request,
            Job::ImageOcr { .. }
            | Job::ImageTranslate { .. }
            | Job::ImageCompose { .. }
            | Job::ScreenCapture { .. }
            | Job::PinRecognize { .. } => {
                panic!("这条断言只适用于文本任务")
            }
        }
    }

    fn engine_of(job: &Job) -> EngineKind {
        match job {
            Job::Translation { engine, .. } => *engine,
            Job::ImageOcr { .. }
            | Job::ImageTranslate { .. }
            | Job::ImageCompose { .. }
            | Job::ScreenCapture { .. }
            | Job::PinRecognize { .. } => {
                panic!("这条断言只适用于文本任务")
            }
        }
    }

    /// 四支都带 `id`，测试里取它就在这儿展开一次。
    /// 不给 `Job` 加 `fn id()`：派发方本来就是按支 match 的，那个访问器只有测试在用。
    fn job_id(job: &Job) -> RequestId {
        match job {
            Job::Translation { id, .. }
            | Job::ImageOcr { id, .. }
            | Job::ImageTranslate { id, .. }
            | Job::ImageCompose { id, .. }
            | Job::ScreenCapture { id, .. } => *id,
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

    /// 派发一次识别、把错误落地，返回界面最终停在哪个态。
    fn land_image_failure(err: VisionError) -> RecognitionStage {
        let mut state = idle_state();
        let Some(Job::ImageOcr { id, .. }) =
            state.submit_image(ImageInput::Path(PathBuf::from("/tmp/不存在的图.png")))
        else {
            panic!("空闲时导入图片必须造出一个识别任务");
        };
        assert!(
            state.apply_image(id, Err(err)),
            "序号 {id} 对得上，结果该被接住"
        );
        state.image_stage
    }

    /// REQ-007 / T-008：没有识别能力的两条来路——非 mac 编译面的 `UnsupportedPlatform`、
    /// macOS 上 Vision 真的失败的 `Recognition`——必须汇成同一个界面态。
    ///
    /// 判在 state 层的原因：置灰入口和那句话是画出来的，无头测不到像素；但"错误最后落地成什么"
    /// 是数据。这一条成立，界面上就只剩怎么摆的问题（摆放由 `views/image.rs` 那组测试盯）。
    /// `Recognition` 的字符串里带着 `code=` / `desc_len=`：它们必须留在日志里，一处都不许进界面。
    #[test]
    fn losing_ocr_lands_in_the_same_failure_shape_whatever_the_platform() {
        let no_capability = land_image_failure(VisionError::UnsupportedPlatform);
        let engine_failed = land_image_failure(VisionError::Recognition(
            "perform_failed_code=1234 desc_len=42".into(),
        ));

        for stage in [&no_capability, &engine_failed] {
            let RecognitionStage::Failed(reason) = stage else {
                panic!("缺少识别能力要落在 Failed，实际是 {stage:?}");
            };
            assert!(
                stage.label().starts_with("识别失败："),
                "I 区要说清是识别这一步出的事：{}",
                stage.label()
            );
            assert!(
                stage.phase() != Phase::Busy,
                "结果已经落地，没有在飞的识别，那颗共用的「停止」不该出现"
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

    fn idle_state() -> AppState {
        AppState::new(
            Config::default(),
            Glossary::default(),
            KeyState::Missing,
            None,
        )
    }

    /// 一份"识别成功"的结果：每串一个块，图是张 4×4 的白图。
    /// 这里要的不是像素，是**行、块、耗时、入场尺寸四样一起落地**这个形状。
    fn recognition(labels: &[&str]) -> ImageRecognition {
        let mut document = OcrDocument::empty(4, 4);
        document.blocks = labels
            .iter()
            .enumerate()
            .map(|(index, text)| {
                TextBlock::with_rows(
                    index as u32,
                    *text,
                    PixelRect::new(0.0, index as f32, 4.0, 1.0),
                    0.9,
                    vec![index as u32],
                )
            })
            .collect();
        ImageRecognition {
            admitted: Admitted {
                image: image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
                scaled: false,
                width: 4,
                height: 4,
                label: "一张图.png".to_string(),
            },
            rows: labels
                .iter()
                .enumerate()
                .map(|(index, text)| RowObservation {
                    text: (*text).to_string(),
                    normalized_box: ice_vision::NormalizedRect {
                        x: 0.0,
                        y: 0.0,
                        w: 1.0,
                        h: 0.5,
                    },
                    confidence: 0.9,
                    row_index: index,
                })
                .collect(),
            document,
            elapsed_ms: 40,
        }
    }

    /// REQ-006 与 REQ-005 交界那条：换图（或重识别）在**派发那一刻**就把旧结果作废。
    /// 清在派发而不是等新一份落地——中间那几百毫秒界面也得是空的，否则用户看到的是"新图配旧列表"，
    /// 而他会照着那份旧列表改字。
    #[test]
    fn a_new_image_voids_the_previous_rows_and_blocks_at_dispatch() {
        let mut state = idle_state();
        let first = state
            .submit_image(ImageInput::Path(PathBuf::from("/tmp/第一张.png")))
            .expect("空闲时该派发得出识别");
        assert!(state.apply_outcome(Outcome::Image {
            id: job_id(&first),
            result: Ok(recognition(&["上一张的第一块"])),
        }));
        assert_eq!(state.image_merged_text(), "上一张的第一块");
        assert!(!state.image_rows.is_empty(), "行也要落地，拆分靠它");

        let second = state
            .submit_image(ImageInput::Path(PathBuf::from("/tmp/第二张.png")))
            .expect("上一份已落地，可以换图");
        assert!(
            state.ocr.is_none() && state.image_rows.is_empty() && state.image_elapsed_ms == 0,
            "派发第二次时旧结果还挂在界面上"
        );
        assert_eq!(state.image_merged_text(), "");

        assert!(state.apply_outcome(Outcome::Image {
            id: job_id(&second),
            result: Ok(recognition(&["新图的第一块"])),
        }));
        assert_eq!(state.image_merged_text(), "新图的第一块");
    }

    /// REQ-006：忽略之后晚到的结果按序号丢弃，且**什么都不碰**——包括"这一次还在飞"那个事实。
    #[test]
    fn a_stale_image_result_is_dropped_without_touching_the_visible_document() {
        let mut state = idle_state();
        let first = state
            .submit_image(ImageInput::Path(PathBuf::from("/tmp/第一张.png")))
            .unwrap();
        assert!(
            state.ignore_image_result(),
            "点「忽略这次识别」要真的把这一次作废"
        );
        let second = state
            .submit_image(ImageInput::Path(PathBuf::from("/tmp/第二张.png")))
            .unwrap();
        assert_ne!(job_id(&first), job_id(&second));

        let stale = Outcome::Image {
            id: job_id(&first),
            result: Ok(recognition(&["旧结果"])),
        };
        assert!(!state.apply_outcome(stale), "序号对不上就该丢掉");
        assert!(
            state.ocr.is_none() && state.image_rows.is_empty(),
            "过期结果回写进了界面"
        );
        assert_eq!(state.image_stage, RecognitionStage::Recognizing);
        assert_eq!(state.image_in_flight, Some(job_id(&second)));

        let fresh = Outcome::Image {
            id: job_id(&second),
            result: Ok(recognition(&["新结果", "第二块"])),
        };
        assert!(state.apply_outcome(fresh));
        assert_eq!(state.image_merged_text(), "新结果\n第二块");
        assert_eq!(state.image_stage, RecognitionStage::Done { blocks: 2 });
    }

    /// REQ-001 第四条：`auto` 永远不许原样交给机翻端点。定得出方向时给的是**具体那一对**，
    /// 定不出时不派发，并且把话说到"该去改哪一个设置"。
    #[test]
    fn the_translate_batch_always_carries_a_concrete_language_pair() {
        // 图上的文字是英文：检出 En，目标照默认走简中
        let mut state = idle_state();
        let first = job_id(
            &state
                .submit_image(ImageInput::Path(PathBuf::from("/tmp/一张.png")))
                .unwrap(),
        );
        state.apply_outcome(Outcome::Image {
            id: first,
            result: Ok(recognition(&["hello world"])),
        });
        let job = state.submit_image_translate(&[]).expect("英文该定得出方向");
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
        let mut same = idle_state();
        let id = job_id(
            &same
                .submit_image(ImageInput::Path(PathBuf::from("/tmp/一张.png")))
                .unwrap(),
        );
        same.apply_outcome(Outcome::Image {
            id,
            result: Ok(recognition(&["你好，今天怎么样"])),
        });
        let flipped = same
            .submit_image_translate(&[])
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
        let mut blank = idle_state();
        let id = job_id(
            &blank
                .submit_image(ImageInput::Path(PathBuf::from("/tmp/一张.png")))
                .unwrap(),
        );
        blank.apply_outcome(Outcome::Image {
            id,
            result: Ok(recognition(&["—— 2026 ——"])),
        });
        assert!(
            blank.submit_image_translate(&[]).is_none(),
            "认不出语言时不该把 auto 交出去"
        );
        assert_eq!(
            blank.translate_stage.label(),
            format!("翻译失败：{NO_DIRECTION_TEXT}"),
            "那句话要说清缺的是源语言"
        );
        assert!(
            blank.translate_in_flight.is_none(),
            "定不出方向时不能留下一个在飞的批次"
        );
    }

    /// 三个序号各自过期（REQ-009 第三条）。三段各有自己的计数器，但**都从 0 起计**，
    /// 所以翻译段与合成段同时在飞时号子会撞成同一个数——正因如此过期判定只能分通道看：
    /// 这一条把"同一次派发落在两段上"写死，谁也不许顺手清掉另一段的在飞标记。
    #[test]
    fn the_three_image_segments_expire_independently() {
        let mut state = translated_state(&["第一块"]);
        // 先派翻译（它把合成段记过期），此刻文档上还留着上一批的译文，于是合成也能派出去
        let translate_id = job_id(&state.submit_image_translate(&[]).unwrap());
        let compose_id = job_id(&state.submit_image_compose().unwrap());
        assert_eq!(
            translate_id, compose_id,
            "两条通道各自从 0 起计，号子本来就会撞成同一个数"
        );
        assert_eq!(state.translate_in_flight, Some(translate_id));
        assert_eq!(state.compose_in_flight, Some(compose_id));

        // 成品先回来：只摘合成段那一个标记，翻译段照旧在飞
        assert!(state.apply_outcome(Outcome::ImageComposed {
            owner: ChainOwner::Page,
            id: compose_id,
            result: Ok(a_composed(2)),
            elapsed_ms: 5,
        }));
        assert!(state.composed.is_some());
        assert_eq!(
            state.translate_in_flight,
            Some(translate_id),
            "合成落地顺手把翻译段也叫停了"
        );
        assert!(matches!(
            state.translate_stage,
            TranslateStage::Running { .. }
        ));

        // 译文再回来：只销翻译段那一个标记，已经换上的成品一张都不许动
        assert!(state.apply_outcome(Outcome::ImageTranslated {
            owner: ChainOwner::Page,
            id: translate_id,
            results: vec![(0, Ok("新一批译文".to_string()))],
            cancelled: false,
            elapsed_ms: 10,
        }));
        assert_eq!(
            state.ocr.as_ref().unwrap().blocks[0].translation.as_deref(),
            Some("新一批译文")
        );
        assert_eq!(state.compose_in_flight, None);
        assert!(state.composed.is_some(), "翻译落地把成品顶掉了");

        // 两段都落地之后，同一次派发的两条迟到结果都该丢掉（号子已经作废旧了）
        assert!(
            !state.apply_outcome(Outcome::ImageTranslated {
                owner: ChainOwner::Page,
                id: translate_id,
                results: vec![(0, Ok("更晚的译文".to_string()))],
                cancelled: false,
                elapsed_ms: 10,
            }),
            "对不上号的翻译结果落了地"
        );
        assert!(
            !state.apply_outcome(Outcome::ImageComposed {
                owner: ChainOwner::Page,
                id: compose_id,
                result: Ok(a_composed(9)),
                elapsed_ms: 5,
            }),
            "对不上号的成品落了地"
        );
        assert_eq!(
            state.ocr.as_ref().unwrap().blocks[0].translation.as_deref(),
            Some("新一批译文"),
            "过期结果写进了文档"
        );
        assert_eq!(state.composed.expect("成品还在").replaced, 2);

        // 识别段的号与这两段也撞，晚到的识别结果同样只认自己那条通道
        let mut fresh = idle_state();
        let ocr_id = job_id(
            &fresh
                .submit_image(ImageInput::Path(PathBuf::from("/tmp/一张.png")))
                .unwrap(),
        );
        fresh.ignore_image_result();
        assert!(
            !fresh.apply_outcome(Outcome::Image {
                id: ocr_id,
                result: Ok(recognition(&["晚到的一行"])),
            }),
            "已忽略的识别结果回写进了界面"
        );
        assert!(fresh.ocr.is_none());
    }

    /// REQ-009 第四条：合成失败**不动**上一张成品。用户没要求销毁他手上已经有的那张。
    #[test]
    fn a_failed_compose_keeps_the_previous_product() {
        let mut state = translated_state(&["第一块"]);
        let first = job_id(&state.submit_image_compose().unwrap());
        assert!(state.apply_outcome(Outcome::ImageComposed {
            owner: ChainOwner::Page,
            id: first,
            result: Ok(a_composed(3)),
            elapsed_ms: 20,
        }));
        assert!(state.composed.is_some());

        // 换一批译文再合成，这一次失败
        state.ocr.as_mut().unwrap().blocks[0].translation = Some("第二版".to_string());
        let second = job_id(&state.submit_image_compose().unwrap());
        assert!(state.apply_outcome(Outcome::ImageComposed {
            owner: ChainOwner::Page,
            id: second,
            result: Err(ice_vision::VisionError::Composite("画不出来".to_string())),
            elapsed_ms: 8,
        }));
        assert!(matches!(state.compose_stage, ComposeStage::Failed(_)));
        let kept = state.composed.expect("失败不该把上一张成品一起清掉");
        assert_eq!(kept.replaced, 3, "留下的必须还是上一张那一个");
    }

    /// 按「停止」那一刻只立哨兵：段的状态要等结果回来才改，界面不许提前说"已停止"。
    #[test]
    fn stopping_the_translate_segment_only_arms_the_sentinel() {
        let mut state = translated_state(&["第一块", "第二块"]);
        let job = state.submit_image_translate(&[]).unwrap();
        let Job::ImageTranslate { cancel, .. } = &job else {
            unreachable!()
        };
        assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(
            state.stop_image_stage(),
            Stop::Translate,
            "翻译段在飞时那颗停的是翻译"
        );
        assert!(
            cancel.load(std::sync::atomic::Ordering::Relaxed),
            "停止要把块边界哨兵立起来"
        );
        assert!(
            matches!(state.translate_stage, TranslateStage::Running { .. }),
            "结果还没回来，段不能说停"
        );
        assert!(state.translate_in_flight.is_some());

        // 哨兵立起来之后结果回来了：落地成 Stopped 而不是 Done
        assert!(state.apply_outcome(Outcome::ImageTranslated {
            owner: ChainOwner::Page,
            id: job_id(&job),
            results: vec![(0, Ok("你好".to_string()))],
            cancelled: true,
            elapsed_ms: 30,
        }));
        assert_eq!(
            state.translate_stage,
            TranslateStage::Stopped { ok: 1, failed: 0 }
        );
        assert!(
            state
                .image_cancel
                .load(std::sync::atomic::Ordering::Relaxed)
        );
    }

    /// 换图把下游两段一起作废：在飞的批次只摘掉标记，晚到的结果因此落不到新图上，
    /// 而上一张图的成品当场消失（留着就是"旧图配新译文"）。
    #[test]
    fn a_new_image_voids_the_translation_batch_and_the_product() {
        let mut state = translated_state(&["第一块"]);
        let translate = state.submit_image_translate(&[]).unwrap();
        state.ocr.as_mut().unwrap().blocks[0].translation = Some("你好".to_string());
        let compose = state.submit_image_compose().unwrap();

        let next = state
            .submit_image(ImageInput::Path(PathBuf::from("/tmp/第二张.png")))
            .unwrap();
        assert!(state.translate_in_flight.is_none());
        assert!(state.compose_in_flight.is_none());
        assert!(state.composed.is_none(), "上一张图的成品还挂在界面上");
        assert!(matches!(state.translate_stage, TranslateStage::Idle));
        assert!(matches!(state.compose_stage, ComposeStage::Idle));
        assert!(state.failed_blocks.is_empty());
        assert_ne!(job_id(&next), job_id(&translate));

        // 两次在飞的晚到结果都该丢掉
        assert!(!state.apply_outcome(Outcome::ImageTranslated {
            owner: ChainOwner::Page,
            id: job_id(&translate),
            results: vec![(0, Ok("旧批次".to_string()))],
            cancelled: false,
            elapsed_ms: 10,
        }));
        assert!(!state.apply_outcome(Outcome::ImageComposed {
            owner: ChainOwner::Page,
            id: job_id(&compose),
            result: Ok(a_composed(9)),
            elapsed_ms: 5,
        }));
        assert!(state.composed.is_none());
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

    /// 一份"翻好了"的状态：入场图 + 每块一条译文，合成段的前置条件就此成立。
    fn translated_state(texts: &[&str]) -> AppState {
        let mut state = idle_state();
        let id = job_id(
            &state
                .submit_image(ImageInput::Path(PathBuf::from("/tmp/一张.png")))
                .unwrap(),
        );
        state.apply_outcome(Outcome::Image {
            id,
            result: Ok(recognition(texts)),
        });
        for block in &mut state.ocr.as_mut().unwrap().blocks {
            block.translation = Some("译文".to_string());
        }
        state.image = Some(Admitted {
            image: image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
            scaled: false,
            width: 4,
            height: 4,
            label: "一张图.png".to_string(),
        });
        state
    }

    /// REQ-006 第四条：文本侧与图片侧各走各的序号，谁也不许让对方的在飞结果作废。
    /// 两支都从 0 起计，所以号子**会**撞成同一个数——正因如此过期判定必须分通道看，
    /// 这条断言把那个巧合直接写死。
    #[test]
    fn the_text_and_image_channels_invalidate_only_their_own_results() {
        let mut state = app_with_source("hello");
        let text = state.submit().expect("有源文就该派发");
        let image = state
            .submit_image(ImageInput::Path(PathBuf::from("/tmp/图.png")))
            .expect("同一时刻另一支也该派发得出");
        assert_eq!(
            job_id(&text),
            job_id(&image),
            "两支独立计数，号子撞上才证明判过期靠的不是数字本身"
        );

        assert!(state.apply_outcome(Outcome::Image {
            id: job_id(&image),
            result: Ok(recognition(&["图上第一块"])),
        }));
        assert_eq!(
            state.in_flight,
            Some(job_id(&text)),
            "识别结果落地不该把在飞的翻译算作过期"
        );
        assert!(state.translation.is_none(), "识别结果不该写进译文栏");

        assert!(state.apply_outcome(Outcome::Translation {
            id: job_id(&text),
            engine: EngineKind::Machine,
            result: Ok(ok("译文", EngineKind::Machine)),
        }));
        assert_eq!(state.translation.as_ref().unwrap().text, "译文");
        assert_eq!(
            state.image_merged_text(),
            "图上第一块",
            "翻译结果落地不该动识别列表"
        );
        assert!(matches!(state.image_stage, RecognitionStage::Done { .. }));
    }
}

/// 把一批"块号 → 译文"落到某一份文档上，顺手销账失败台账。
///
/// **图片页与贴图共用这一条**：两份"块不在了就丢掉、失败要进台账"的写法各写一遍，
/// 迟早一份严格一份宽松。丢的那一支尤其容易写歪：块不在了 = 用户在这批请求还在网上跑
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
