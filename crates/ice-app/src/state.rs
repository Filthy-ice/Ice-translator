//! 界面状态与状态转移。
//!
//! 这里只放纯逻辑，不碰 egui 绘制，因此 REQ-001~REQ-004、REQ-007 的行为都能无头断言。

use std::path::PathBuf;

use ice_core::config::{Config, UiMode};
use ice_core::document::{OcrDocument, PixelRect, TextBlock};
use ice_core::glossary::{Glossary, Term};
use ice_core::history::{HISTORY_CAPACITY, HistoryRecord, HistoryStore};
use ice_core::language::{EngineKind, Language};
use ice_providers::error::ProviderError;
use ice_providers::request::{TranslationRequest, TranslationResult};
use ice_vision::error::VisionError;
use ice_vision::import::Admitted;
use ice_vision::layout::{RowObservation, flip_rect};

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

/// 一次待派发的任务：文本翻译，或图片识别（P6 的 S0~S3 一整段）。
///
/// 序号在两支里都显式放着：派发方（`AppState::submit*`）写它，接活的一方按支取出——
/// 不给它加 `fn id()`，因为除了测试没有第二个调用点，一个只有测试在用的访问器只是装饰。
#[derive(Debug, Clone, PartialEq)]
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
}

/// 提示条最多同时显示这么多条，避免把界面挤没
const MAX_TOASTS: usize = 3;

/// 图片页识别段的五态（CHG-005 REQ-006）。
///
/// 这是**识别那一段**的状态：P7 的翻译段与合成段各自还要一条（FR-2.9 的完整语义），
/// 本包只做到这里，所以叫 Stage 而不是 Progress——它不承诺百分比。
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

    /// 识别中才允许点「忽略」；其余状态这颗按钮该是灰的或不出现
    pub const fn cancellable(&self) -> bool {
        matches!(self, RecognitionStage::Recognizing)
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
    /// 窗口当前是不是被"双击标题栏"铺满了整块屏（任务 #25）。
    ///
    /// **运行时状态，不落盘**：铺满是这一屏这一次观看的临时姿势，重启后回到 `full_rect`
    /// 才是"记住位置尺寸"（FR-3.6）。它同时是 `mode::capture` 的抑制开关——铺满期间实时
    /// 几何就是整屏，让它写进 `full_rect` 就等于把 D-6/D-10 那一类缺陷换个入口重犯一遍：
    /// 还原时已经没有"铺满之前的尺寸"可回了。
    pub filled: bool,
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
            filled: false,
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
        self.config.translation.target = new_target;
        self.config.translation.remember_target(new_target);
        self.config_dirty = true;
        self.toast(
            Tone::Info,
            format!(
                "检测到{}，已自动改为 {} → {}",
                detection.language.display_name(),
                detection.language.display_name(),
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

    /// P6 之后 `Job` 有两种，这几条测试断的全是文本那一支——取不出来就说明用错了分支。
    fn request_of(job: &Job) -> &TranslationRequest {
        match job {
            Job::Translation { request, .. } => request,
            Job::ImageOcr { .. } => panic!("这条断言只适用于文本任务"),
        }
    }

    fn engine_of(job: &Job) -> EngineKind {
        match job {
            Job::Translation { engine, .. } => *engine,
            Job::ImageOcr { .. } => panic!("这条断言只适用于文本任务"),
        }
    }

    /// 两支都带 `id`，测试里取它就在这儿展开一次。
    /// 不给 `Job` 加 `fn id()`：派发方本来就是按支 match 的，那个访问器只有测试在用。
    fn job_id(job: &Job) -> RequestId {
        match job {
            Job::Translation { id, .. } | Job::ImageOcr { id, .. } => *id,
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
                !stage.cancellable(),
                "结果已经落地，没有在飞的识别，不该给「忽略这次识别」"
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
