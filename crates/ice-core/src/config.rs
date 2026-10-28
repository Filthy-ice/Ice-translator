use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::glossary::Term;
use crate::language::{EngineKind, LangPair, Language};

pub const APP_ID: &str = "IceTranslator";
/// AI 服务 API Key 在钥匙串中的固定条目名（FR-4.2）
pub const KEYCHAIN_SERVICE: &str = "IceTranslator";
pub const KEYCHAIN_ACCOUNT: &str = "ai-api-key";

/// 全量窗的最小内尺寸（FR-3.2、`03-ui-design.md` §1）。
///
/// 这里的四个常量是两种形态下限的唯一出处：`ice-app::mode` 用它们拼 `MinInnerSize`
/// 交给 AppKit 逐帧执行，`Config::sanitized` 用它们在加载时夹回手改坏的配置。
/// 两处各写一份字面量就会漂（P4 的简洁窗高度下限写成 64，与设计的数值不一致）。
pub const MIN_FULL_W: f32 = 640.0;
pub const MIN_FULL_H: f32 = 420.0;
/// 简洁窗的最小内尺寸（FR-3.2，CHG-004 REQ-001/REQ-012：可缩放，但不小于 400×160）。
///
/// 高度两次抬高：v0.5 从 96 抬到 130（动作行改回与全量窗同字号同内边距后，最窄窗里它要占
/// 两行，96px 把译文区挤成零高）；v0.7 抬到 160（REQ-012 的两带 1:1 分摊要在下限处也让
/// 两带都装得下一行字——130 高时均分只剩 (130−30−10)/2 ≈ 45px，译文带连两行都放不下）。
///
/// 宽度 320 → 400 是用户 2026-10-28 走查提的"最小窗口太小了，按钮不允许换行"：320 那一档
/// 「退出」被挤到第二行。为什么是 400 由 `ice-app::views::compact` 里的
/// `the_action_row_stays_on_one_line_at_the_minimum_width` 按字形度量算出来并锁住
/// （七颗按钮的字宽 + 内边距 + 间距 + 面板边距 = 348px，400 再留 48px 给行尾的拖窗把手）。
pub const MIN_COMPACT_W: f32 = 400.0;
pub const MIN_COMPACT_H: f32 = 160.0;

/// 失焦半透明可调区间的两端（FR-3.7）。
///
/// 下限 0.30：再低悬浮窗就基本找不见了，用户会以为程序崩了（悬浮窗没有别的常驻入口）。
/// 上限 1.00：等于"开着开关但完全不透明"，与关开关等价，交给用户当"略微透一点"的微调用。
pub const MIN_UNFOCUSED_ALPHA: f32 = 0.30;
pub const MAX_UNFOCUSED_ALPHA: f32 = 1.00;
/// 默认不透明度：即 v0.6 起写死的 `DIM_ALPHA`，2026-10-28 改成可调后保留原值当默认档
pub const DEFAULT_UNFOCUSED_ALPHA: f32 = 0.85;

/// 屏幕矩形，用于持久化窗口位置（FR-3.6）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl WindowRect {
    /// 位置是否落在某块屏幕的可用区内；屏幕拔掉后旧坐标会失败，此时应回到屏幕中心。
    pub fn within(self, monitor: &WindowRect) -> bool {
        self.x >= monitor.x - 1.0
            && self.y >= monitor.y - 1.0
            && self.right() <= monitor.right() + 1.0
            && self.bottom() <= monitor.bottom() + 1.0
    }

    pub const fn right(self) -> f32 {
        self.x + self.w
    }

    pub const fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn centered_in(monitor: &WindowRect, w: f32, h: f32) -> Self {
        Self {
            x: monitor.x + (monitor.w - w) / 2.0,
            y: monitor.y + (monitor.h - h) / 2.0,
            w,
            h,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiMode {
    Full,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DockEdge {
    Left,
    #[default]
    Right,
    /// v0.5 起运行时不再产生（REQ-003）：只为读得动旧配置文件而保留
    Top,
    /// v0.5 起运行时不再产生（REQ-003）：只为读得动旧配置文件而保留
    Bottom,
    /// 自由浮动
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub mode: UiMode,
    pub always_on_top: bool,
    pub theme: Theme,
    /// 界面缩放，合法区间 [0.8, 2.0]
    pub font_scale: f32,
    /// 简洁模式失焦时半透明（FR-3.7）
    pub dim_when_unfocused: bool,
    /// 失焦半透明降到的不透明度（FR-3.7，2026-10-28 起可调）。
    /// 合法区间 [`MIN_UNFOCUSED_ALPHA`]..=[`MAX_UNFOCUSED_ALPHA`]，越界由 `sanitized` 夹回。
    pub unfocused_alpha: f32,
    /// 复制即翻（FR-3.8）
    pub clipboard_watch: bool,
    pub full_rect: WindowRect,
    pub compact_rect: WindowRect,
    pub docked_edge: DockEdge,
    /// 贴边停靠时保留的可见热区宽度（FR-3.5）
    pub dock_sliver_px: f32,
    /// 距边缘小于该值即吸附（FR-3.5）
    pub snap_threshold_px: f32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            mode: UiMode::Full,
            always_on_top: false,
            theme: Theme::System,
            font_scale: 1.0,
            dim_when_unfocused: true,
            unfocused_alpha: DEFAULT_UNFOCUSED_ALPHA,
            clipboard_watch: false,
            full_rect: WindowRect {
                x: 120.0,
                y: 120.0,
                w: 1080.0,
                h: 720.0,
            },
            compact_rect: WindowRect {
                x: 1400.0,
                y: 320.0,
                w: 460.0,
                h: 160.0,
            },
            docked_edge: DockEdge::None,
            dock_sliver_px: 3.0,
            snap_threshold_px: 40.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslationConfig {
    pub default_engine: EngineKind,
    pub source: Option<Language>,
    pub target: Language,
    /// 除中英日之外额外启用的语言（FR-1.4）
    pub extra_languages: Vec<Language>,
    /// 最近使用的目标语言，最多 5 个
    pub recent_targets: Vec<Language>,
    /// AI 超长输入分段阈值（FR-1.9）
    pub ai_chunk_chars: usize,
    /// 单次请求超时（NFR-1）
    pub timeout_ms: u64,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            default_engine: EngineKind::Machine,
            source: None,
            target: Language::ZhHans,
            extra_languages: Vec::new(),
            recent_targets: vec![Language::ZhHans],
            ai_chunk_chars: 3500,
            timeout_ms: 15_000,
        }
    }
}

impl TranslationConfig {
    pub fn lang_pair(&self) -> LangPair {
        LangPair {
            source: self.source,
            target: self.target,
        }
    }

    pub fn remember_target(&mut self, lang: Language) {
        self.recent_targets.retain(|l| *l != lang);
        self.recent_targets.insert(0, lang);
        self.recent_targets.truncate(5);
    }

    /// 界面语言下拉可见的语言集合
    pub fn visible_languages(&self) -> Vec<Language> {
        let mut out: Vec<Language> = Language::preferred().to_vec();
        for lang in &self.extra_languages {
            if !out.contains(lang) {
                out.push(*lang);
            }
        }
        for lang in &self.recent_targets {
            if !out.contains(lang) {
                out.push(*lang);
            }
        }
        out
    }
}

/// AI 供应商预设（FR-1.5、FR-4.4）。
///
/// 国内几家已收敛到 OpenAI 兼容的 `/chat/completions`，差异只剩 base_url、默认模型名，
/// 以及方舟必须填推理接入点 ID。预设只负责"选了就少打字"，任何一项都仍可手改。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiProvider {
    Bailian,
    DeepSeek,
    Zhipu,
    Kimi,
    VolcengineArk,
    /// 其他 OpenAI 兼容服务，base_url 由用户填写
    #[default]
    Custom,
}

impl AiProvider {
    pub const fn all() -> [AiProvider; 6] {
        [
            AiProvider::Bailian,
            AiProvider::DeepSeek,
            AiProvider::Zhipu,
            AiProvider::Kimi,
            AiProvider::VolcengineArk,
            AiProvider::Custom,
        ]
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            AiProvider::Bailian => "阿里百炼",
            AiProvider::DeepSeek => "DeepSeek",
            AiProvider::Zhipu => "智谱 GLM",
            AiProvider::Kimi => "Kimi（月之暗面）",
            AiProvider::VolcengineArk => "火山方舟",
            AiProvider::Custom => "自定义 OpenAI 兼容服务",
        }
    }

    /// 预设的 base_url；Custom 返回空串，表示必须由用户填写
    pub const fn preset_base_url(self) -> &'static str {
        match self {
            AiProvider::Bailian => "https://dashscope.aliyuncs.com/compatible-mode/v1",
            AiProvider::DeepSeek => "https://api.deepseek.com",
            AiProvider::Zhipu => "https://open.bigmodel.cn/api/paas/v4",
            AiProvider::Kimi => "https://api.moonshot.cn/v1",
            AiProvider::VolcengineArk => "https://ark.cn-beijing.volces.com/api/v3",
            AiProvider::Custom => "",
        }
    }

    /// 预设的默认模型名。方舟没有可用默认值（要填推理接入点 ID），留空提示用户。
    /// DeepSeek 用 `deepseek-flash`：`deepseek-chat` 这个型号名已经不用了（用户 2026-10-28 确认）。
    pub const fn preset_model(self) -> &'static str {
        match self {
            AiProvider::Bailian => "qwen-plus",
            AiProvider::DeepSeek => "deepseek-flash",
            AiProvider::Zhipu => "glm-4-flash",
            AiProvider::Kimi => "moonshot-v1-8k",
            AiProvider::VolcengineArk => "",
            AiProvider::Custom => "",
        }
    }

    /// 界面提示：这一项是否需要用户自己填模型
    pub const fn needs_manual_model(self) -> bool {
        self.preset_model().is_empty()
    }

    /// 关闭"思考"时该往请求体里塞哪个字段（FR-1.9 长文提速，2026-10-28 用户要求"别忘了接口中关闭思考"）。
    ///
    /// 翻译是直出任务，思考链只会同时抬高首字延迟与 token 花费，所以默认关掉。
    /// 各家拼法不统一，这正是"差异收敛在预设里"的那一处：
    /// 百炼的 OpenAI 兼容模式认顶层布尔 `enable_thinking`，其余几家用 `thinking: {"type":"disabled"}`。
    ///
    /// Custom 返回 `None`：自定义地址后面是什么服务无从判断，多塞一个陌生字段可能被严格
    /// 服务端 400 拒掉——宁可不关思考，也不能让整条翻译链失败。
    pub const fn thinking_off(self) -> Option<ThinkingOff> {
        match self {
            AiProvider::Bailian => Some(ThinkingOff::Bool("enable_thinking")),
            AiProvider::DeepSeek
            | AiProvider::Zhipu
            | AiProvider::Kimi
            | AiProvider::VolcengineArk => Some(ThinkingOff::Typed("thinking")),
            AiProvider::Custom => None,
        }
    }
}

/// 关闭思考的字段形态（见 [`AiProvider::thinking_off`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingOff {
    /// `"<字段>": false`
    Bool(&'static str),
    /// `"<字段>": {"type": "disabled"}`
    Typed(&'static str),
}

/// AI 服务参数。**不包含 Key 本身**，只包含钥匙串条目引用（FR-4.2）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub provider: AiProvider,
    /// 留空表示用预设自带的地址
    pub base_url: String,
    /// 留空表示用预设自带的模型
    pub model: String,
    /// 留空表示用服务默认
    pub temperature: Option<f32>,
    pub key_configured: bool,
}

impl AiConfig {
    /// 实际生效的 base_url：手填优先，否则取预设
    pub fn effective_base_url(&self) -> String {
        let trimmed = self.base_url.trim();
        if trimmed.is_empty() {
            self.provider.preset_base_url().to_string()
        } else {
            trimmed.to_string()
        }
    }

    pub fn effective_model(&self) -> String {
        let trimmed = self.model.trim();
        if trimmed.is_empty() {
            self.provider.preset_model().to_string()
        } else {
            trimmed.to_string()
        }
    }

    pub fn is_ready(&self) -> bool {
        !self.effective_base_url().is_empty()
            && !self.effective_model().is_empty()
            && self.key_configured
    }

    /// 归一化后的 chat/completions 端点；地址缺失时返回空串（此时 `is_ready()` 必为 false）
    pub fn endpoint(&self) -> String {
        let base = self.effective_base_url();
        if base.is_empty() {
            return String::new();
        }
        let base = base.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        }
    }
}

/// 顶层配置。FR-4.1 / FR-4.3
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ui: UiConfig,
    pub translation: TranslationConfig,
    pub ai: AiConfig,
    /// 术语表（FR-4.5），渲染成 `[[term]] source=… target=…`。
    /// 必须是最后一个字段：TOML 的 array-of-tables 之后的键都会归属到它，放在中间会把配置写坏。
    #[serde(rename = "term")]
    pub terms: Vec<Term>,
}

impl Config {
    pub fn config_dir() -> Result<PathBuf, CoreError> {
        dirs::config_dir()
            .map(|p| p.join(APP_ID))
            .ok_or_else(|| CoreError::ConfigDir("系统找不到配置目录".into()))
    }

    pub fn config_path() -> Result<PathBuf, CoreError> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    /// 读取配置。文件不存在时返回默认值；内容损坏时回退默认值并记录原因（FR-4.3）。
    pub fn load() -> Result<(Config, Option<String>), CoreError> {
        Self::load_at(&Self::config_path()?)
    }

    pub fn load_at(path: &Path) -> Result<(Config, Option<String>), CoreError> {
        if !path.exists() {
            return Ok((Self::default(), None));
        }
        let raw = fs::read_to_string(path)?;
        match toml::from_str::<Config>(&raw) {
            Ok(cfg) => Ok((cfg.sanitized(), None)),
            Err(e) => {
                let reason = format!("配置文件 {path:?} 解析失败，本次以默认配置启动：{e}");
                tracing::warn!("{reason}");
                Ok((Self::default(), Some(reason)))
            }
        }
    }

    pub fn save(&self) -> Result<PathBuf, CoreError> {
        let path = Self::config_path()?;
        self.save_at(&path)?;
        Ok(path)
    }

    /// 先写临时文件再改名，避免中途崩溃留下半份配置。
    pub fn save_at(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let rendered =
            toml::to_string_pretty(self).map_err(|e| CoreError::ConfigParse(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, rendered)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 把越界数值夹回合法区间，防止坏配置导致窗口不可用。
    pub fn sanitized(mut self) -> Self {
        self.ui.font_scale = self.ui.font_scale.clamp(0.8, 2.0);
        // clamp 对 NaN 两条比较都为假、会原样返回 NaN，所以先单独挡一次（TOML 能写出 nan）。
        // alpha 是 NaN 时窗体会整个看不见，宁可退回默认档。
        self.ui.unfocused_alpha = if self.ui.unfocused_alpha.is_finite() {
            self.ui
                .unfocused_alpha
                .clamp(MIN_UNFOCUSED_ALPHA, MAX_UNFOCUSED_ALPHA)
        } else {
            DEFAULT_UNFOCUSED_ALPHA
        };
        self.ui.dock_sliver_px = self.ui.dock_sliver_px.clamp(2.0, 24.0);
        self.ui.snap_threshold_px = self.ui.snap_threshold_px.clamp(8.0, 200.0);
        // v0.5 起停靠只认左/右（Q-010）。Top/Bottom 留在枚举里只为反序列化兼容，
        // 读到就按自由浮动处理：藏进菜单栏或 Dock 的那条边唤不出来，留着等于又弄丢一次窗口。
        self.ui.docked_edge = match self.ui.docked_edge {
            DockEdge::Top | DockEdge::Bottom => DockEdge::None,
            edge => edge,
        };
        self.ui.full_rect.w = self.ui.full_rect.w.max(MIN_FULL_W);
        self.ui.full_rect.h = self.ui.full_rect.h.max(MIN_FULL_H);
        self.ui.compact_rect.w = self.ui.compact_rect.w.max(MIN_COMPACT_W);
        self.ui.compact_rect.h = self.ui.compact_rect.h.max(MIN_COMPACT_H);
        if self.translation.timeout_ms < 1_000 {
            self.translation.timeout_ms = 1_000;
        }
        if self.translation.ai_chunk_chars < 500 {
            self.translation.ai_chunk_chars = 500;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_uses_defaults_for_missing_sections() {
        let cfg: Config = toml::from_str("[ui]\nalways_on_top = true\n").unwrap();
        assert!(cfg.ui.always_on_top);
        assert_eq!(cfg.ui.font_scale, 1.0);
        assert_eq!(cfg.translation.target, Language::ZhHans);
    }

    #[test]
    fn roundtrip_keeps_values() {
        let mut cfg = Config::default();
        cfg.ui.mode = UiMode::Compact;
        cfg.ui.always_on_top = true;
        cfg.translation.default_engine = EngineKind::Ai;
        cfg.translation.target = Language::Ja;
        cfg.ai.base_url = "https://api.example.com/v1".into();
        cfg.ai.model = "some-model".into();

        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn corrupt_config_is_rejected_not_panic() {
        let err = toml::from_str::<Config>("ui = ][ this is not toml").is_err();
        assert!(err);
    }

    #[test]
    fn load_at_falls_back_to_defaults_with_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "this is = = not valid toml").unwrap();
        let (cfg, reason) = Config::load_at(&path).unwrap();
        assert_eq!(cfg, Config::default());
        assert!(reason.unwrap().contains("解析失败"));
    }

    #[test]
    fn load_at_missing_file_returns_defaults_without_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, reason) = Config::load_at(&dir.path().join("absent.toml")).unwrap();
        assert_eq!(cfg, Config::default());
        assert!(reason.is_none());
    }

    #[test]
    fn save_at_creates_dirs_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let mut cfg = Config::default();
        cfg.ui.always_on_top = true;
        cfg.save_at(&path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("toml.tmp").exists());
        let (loaded, _) = Config::load_at(&path).unwrap();
        assert_eq!(loaded, cfg);
    }

    #[test]
    fn sanitize_clamps_out_of_range_values() {
        let mut cfg = Config::default();
        cfg.ui.font_scale = 99.0;
        cfg.ui.snap_threshold_px = 0.1;
        cfg.ui.compact_rect.w = 10.0;
        cfg.translation.timeout_ms = 5;
        let cfg = cfg.sanitized();
        assert_eq!(cfg.ui.font_scale, 2.0);
        assert_eq!(cfg.ui.snap_threshold_px, 8.0);
        assert_eq!(cfg.ui.compact_rect.w, MIN_COMPACT_W);
        assert_eq!(cfg.translation.timeout_ms, 1_000);
    }

    /// 手改配置改出比原生约束还小的简洁窗时，夹回下限而不是拒绝启动（CHG-004 REQ-001 边界）。
    /// 高度下限曾经写成 64，与 `MIN_COMPACT_SIZE` 的 96 不一致——那条缝里能存下 AppKit
    /// 根本不允许的尺寸，启动后窗口会被原生约束顶回 96，配置与真实几何就此对不上。
    #[test]
    fn sanitize_lifts_a_hand_edited_compact_rect_to_the_native_floor() {
        let mut cfg = Config::default();
        cfg.ui.compact_rect.w = 100.0;
        cfg.ui.compact_rect.h = 40.0;
        let rect = cfg.sanitized().ui.compact_rect;
        assert_eq!((rect.w, rect.h), (MIN_COMPACT_W, MIN_COMPACT_H));
    }

    /// 失焦不透明度可调之后，加载侧要有同一道护栏：用户手改 `unfocused_alpha` 越界
    /// 只能夹到区间两端，不能把 0.0 或 2.0 原样带进 `setAlphaValue:`。
    #[test]
    fn sanitize_keeps_the_unfocused_alpha_inside_its_band() {
        let mut cfg = Config::default();
        cfg.ui.unfocused_alpha = 0.0;
        assert_eq!(
            cfg.sanitized().ui.unfocused_alpha,
            MIN_UNFOCUSED_ALPHA,
            "透过头了要停在看不见的边缘，而不是整块窗消失"
        );

        let mut cfg = Config::default();
        cfg.ui.unfocused_alpha = 2.0;
        assert_eq!(cfg.sanitized().ui.unfocused_alpha, MAX_UNFOCUSED_ALPHA);

        let mut cfg = Config::default();
        cfg.ui.unfocused_alpha = f32::NAN;
        assert_eq!(cfg.sanitized().ui.unfocused_alpha, DEFAULT_UNFOCUSED_ALPHA);

        let mut cfg = Config::default();
        cfg.ui.unfocused_alpha = 0.6;
        assert_eq!(
            cfg.sanitized().ui.unfocused_alpha,
            0.6,
            "区间内的值不该被动过"
        );
    }

    #[test]
    fn sanitize_leaves_a_larger_compact_rect_alone() {
        let mut cfg = Config::default();
        cfg.ui.compact_rect.w = 700.0;
        cfg.ui.compact_rect.h = 200.0;
        let rect = cfg.sanitized().ui.compact_rect;
        assert_eq!((rect.w, rect.h), (700.0, 200.0), "无上限，不该被夹小");
    }

    #[test]
    fn ai_endpoint_accepts_both_forms() {
        let mut cfg = AiConfig {
            base_url: "https://api.example.com/v1/".into(),
            ..Default::default()
        };
        assert_eq!(
            cfg.endpoint(),
            "https://api.example.com/v1/chat/completions"
        );
        cfg.base_url = "https://gateway.internal/v1/chat/completions".into();
        assert_eq!(
            cfg.endpoint(),
            "https://gateway.internal/v1/chat/completions"
        );
    }

    #[test]
    fn preset_provider_supplies_url_and_model() {
        let cfg = AiConfig {
            provider: AiProvider::DeepSeek,
            key_configured: true,
            ..Default::default()
        };
        assert_eq!(cfg.endpoint(), "https://api.deepseek.com/chat/completions");
        assert_eq!(cfg.effective_model(), "deepseek-flash");
        assert!(cfg.is_ready(), "选完供应商就能用，不该再要求手填地址");
    }

    #[test]
    fn manual_values_beat_the_preset() {
        let cfg = AiConfig {
            provider: AiProvider::DeepSeek,
            base_url: "https://proxy.example.com/v1".into(),
            model: "deepseek-reasoner".into(),
            ..Default::default()
        };
        assert_eq!(
            cfg.endpoint(),
            "https://proxy.example.com/v1/chat/completions"
        );
        assert_eq!(cfg.effective_model(), "deepseek-reasoner");
    }

    /// 2026-10-28 用户要求"别忘了接口中关闭思考"。这条锁的是两家的拼法别写反：
    /// 百炼只认顶层布尔 `enable_thinking`，其余几家用 `thinking.type`，Custom 一个都不发。
    #[test]
    fn thinking_off_field_follows_each_vendors_spelling() {
        assert_eq!(
            AiProvider::Bailian.thinking_off(),
            Some(ThinkingOff::Bool("enable_thinking"))
        );
        for p in [
            AiProvider::DeepSeek,
            AiProvider::Zhipu,
            AiProvider::Kimi,
            AiProvider::VolcengineArk,
        ] {
            assert_eq!(
                p.thinking_off(),
                Some(ThinkingOff::Typed("thinking")),
                "{} 应该用对象写法",
                p.display_name()
            );
        }
        assert_eq!(
            AiProvider::Custom.thinking_off(),
            None,
            "地址是用户手填的，服务未知，不许发陌生字段"
        );
    }

    #[test]
    fn custom_without_url_is_not_ready() {
        let cfg = AiConfig {
            provider: AiProvider::Custom,
            key_configured: true,
            ..Default::default()
        };
        assert!(cfg.effective_base_url().is_empty());
        assert!(!cfg.is_ready());
    }

    #[test]
    fn every_preset_except_custom_has_an_address() {
        for p in AiProvider::all() {
            if p == AiProvider::Custom {
                assert!(p.preset_base_url().is_empty());
            } else {
                assert!(
                    p.preset_base_url().starts_with("https://"),
                    "{} 必须用 https",
                    p.display_name()
                );
                assert_eq!(p.needs_manual_model(), p.preset_model().is_empty());
            }
        }
    }

    #[test]
    fn config_without_provider_field_still_loads() {
        // 老配置没有 provider 这一项，必须按 Custom 理解而不是报错
        let cfg: Config = toml::from_str("[ai]\nbase_url = \"https://a/v1\"\nmodel = \"m\"\n")
            .expect("旧配置应能加载");
        assert_eq!(cfg.ai.provider, AiProvider::Custom);
        assert_eq!(cfg.ai.effective_base_url(), "https://a/v1");
    }

    #[test]
    fn ai_not_ready_without_key() {
        let mut cfg = AiConfig {
            base_url: "https://a/v1".into(),
            model: "m".into(),
            ..Default::default()
        };
        assert!(!cfg.is_ready());
        cfg.key_configured = true;
        assert!(cfg.is_ready());
    }

    #[test]
    fn remember_target_keeps_five_and_most_recent_first() {
        let mut cfg = TranslationConfig::default();
        for lang in [
            Language::En,
            Language::Ja,
            Language::De,
            Language::Fr,
            Language::Es,
            Language::Ru,
        ] {
            cfg.remember_target(lang);
        }
        assert_eq!(cfg.recent_targets.len(), 5);
        assert_eq!(cfg.recent_targets[0], Language::Ru);
        assert!(!cfg.recent_targets.contains(&Language::ZhHans));
    }

    #[test]
    fn visible_languages_always_includes_preferred() {
        let cfg = TranslationConfig {
            extra_languages: vec![Language::De, Language::En],
            ..Default::default()
        };
        let visible = cfg.visible_languages();
        for lang in Language::preferred() {
            assert!(visible.contains(lang));
        }
        assert!(visible.contains(&Language::De));
        assert_eq!(visible.iter().filter(|l| **l == Language::En).count(), 1);
    }

    #[test]
    fn window_off_screen_detection() {
        let monitor = WindowRect {
            x: 0.0,
            y: 0.0,
            w: 1440.0,
            h: 900.0,
        };
        assert!(
            WindowRect {
                x: 100.0,
                y: 100.0,
                w: 400.0,
                h: 300.0
            }
            .within(&monitor)
        );
        assert!(
            !WindowRect {
                x: 2200.0,
                y: 100.0,
                w: 400.0,
                h: 300.0
            }
            .within(&monitor)
        );
        let centered = WindowRect::centered_in(&monitor, 400.0, 300.0);
        assert!(centered.within(&monitor));
    }
}
