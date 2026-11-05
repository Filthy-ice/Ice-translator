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
}

impl Confirm {
    pub const fn prompt(self) -> &'static str {
        match self {
            Confirm::ClearHistory => "清空全部历史？此操作不可撤销。",
            Confirm::ClearKey => "清除钥匙串里的 API Key？清除后 AI 通道不可用。",
        }
    }

    pub const fn confirm_label(self) -> &'static str {
        match self {
            Confirm::ClearHistory => "确认清空",
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
        engine: EngineKind,
        blocks: Vec<(u32, TranslationRequest)>,
        /// 块边界停止的哨兵：界面置真，工作循环在**下一块请求之前**看到它就收手。
        /// 在飞的那一次请求不撤回（撤不回），所以停止最少要等一块的时间。
        cancel: Arc<AtomicBool>,
    },
    /// 合成一张成品（REQ-003~REQ-006）。整段是一次同步的 CPU 活，走 `spawn_blocking`。
    ImageCompose {
        id: RequestId,
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
        done: usize,
        total: usize,
    },
    /// 逐块翻译收工。`results` 是 `(块 id, 该块结果)`：一块 `Err` 只作废它自己，
    /// 其余块的 `Ok` 照样落地（REQ-001 第二条）。`cancelled` 表示这是在块边界停下来的，
    /// 已翻好的那些照样有效、不是半成品（Q-006 裁定"留"）。
    ImageTranslated {
        id: RequestId,
        results: Vec<(u32, Result<String, ProviderError>)>,
        cancelled: bool,
        elapsed_ms: u64,
    },
    ImageComposed {
        id: RequestId,
        result: Result<Composed, VisionError>,
        elapsed_ms: u64,
    },
}

/// 提示条最多同时显示这么多条，避免把界面挤没
const MAX_TOASTS: usize = 3;

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
            Outcome::ImageTranslateProgress { id, done, total } => {
                self.apply_translate_progress(id, done, total)
            }
            Outcome::ImageTranslated {
                id,
                results,
                cancelled,
                elapsed_ms,
            } => self.apply_image_translated(id, results, cancelled, elapsed_ms),
            Outcome::ImageComposed {
                id,
                result,
                elapsed_ms,
            } => self.apply_image_composed(id, result, elapsed_ms),
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
            source: image.image.clone(),
            document: document.clone(),
            mode: self.compose_mode,
        })
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
            for (block_id, result) in results {
                self.failed_blocks.retain(|failed| *failed != block_id);
                match result {
                    // 块不在了 = 用户在这一批还在网上跑的时候把它删了，结果无处落，丢掉
                    Ok(text) => {
                        if let Some(block) = document.blocks.iter_mut().find(|b| b.id == block_id) {
                            block.translation = Some(text);
                            ok += 1;
                        }
                    }
                    Err(_) => self.failed_blocks.push(block_id),
                }
            }
        }
        let failed = self.failed_blocks.len();
        self.translate_stage = if cancelled {
            TranslateStage::Stopped { ok, failed }
        } else {
            TranslateStage::Done { ok, failed }
        };
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
            Job::ImageOcr { .. } | Job::ImageTranslate { .. } | Job::ImageCompose { .. } => {
                panic!("这条断言只适用于文本任务")
            }
        }
    }

    fn engine_of(job: &Job) -> EngineKind {
        match job {
            Job::Translation { engine, .. } => *engine,
            Job::ImageOcr { .. } | Job::ImageTranslate { .. } | Job::ImageCompose { .. } => {
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
            | Job::ImageCompose { id, .. } => *id,
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
                id: translate_id,
                results: vec![(0, Ok("更晚的译文".to_string()))],
                cancelled: false,
                elapsed_ms: 10,
            }),
            "对不上号的翻译结果落了地"
        );
        assert!(
            !state.apply_outcome(Outcome::ImageComposed {
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
            id: first,
            result: Ok(a_composed(3)),
            elapsed_ms: 20,
        }));
        assert!(state.composed.is_some());

        // 换一批译文再合成，这一次失败
        state.ocr.as_mut().unwrap().blocks[0].translation = Some("第二版".to_string());
        let second = job_id(&state.submit_image_compose().unwrap());
        assert!(state.apply_outcome(Outcome::ImageComposed {
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
            id: job_id(&translate),
            results: vec![(0, Ok("旧批次".to_string()))],
            cancelled: false,
            elapsed_ms: 10,
        }));
        assert!(!state.apply_outcome(Outcome::ImageComposed {
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
