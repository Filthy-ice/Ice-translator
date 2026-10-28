//! Ice Translator 客户端入口（P4 全量界面）。
//!
//! 启动时序按 `features/CHG-003/design.md`：
//! `tracing → 单实例锁 → 读配置 → 导入旧术语表 → 装配引擎 → tokio → 建窗 → 后台读钥匙串`。
//! 锁在建窗之前拿，拿不到就退出——第二个实例不该碰 `config.toml`、`history.jsonl` 或钥匙串。
//!
//! 钥匙串排在最后且不在主线程：macOS 可能弹授权框把读取阻塞住，建窗不能等它
//! （见 `app::spawn_key_probe`）。这是相对封存设计的一处实现更正，不改任何 REQ。

use std::path::{Path, PathBuf};
use std::process::exit;

use ice_core::config::Config;
use ice_core::error::CoreError;
use ice_core::glossary::Glossary;
use ice_core::history::HistoryStore;

use crate::app::{IceApp, build_translator, spawn_key_probe};
use crate::state::{AppState, KeyState};
use crate::system::secrets;
use crate::system::single_instance::{self, AcquireError, InstanceLock};

mod app;
mod jobs;
mod mode;
mod state;
mod system;
mod theme;
mod views;

#[cfg(test)]
mod stub;

/// 启动阶段的全部产物。`prepare` 只做纯拼装（可以拿临时目录无头断言），
/// 碰系统的三件事（钥匙串、历史文件、配置读取）在 `main` 里做完再交给它。
pub struct Prepared {
    pub config: Config,
    pub glossary: Glossary,
    pub key: KeyState,
    /// Key 明文：只用来装配 `AiEngine`，不进 `AppState`、不落盘、不进日志
    pub api_key: Option<String>,
    pub history: HistoryStore,
    /// 要在界面上给一次的启动提示（配置损坏降级、术语导入、历史/钥匙串不可用）
    pub notice: Option<String>,
    /// 启动过程中改了配置（导入术语、对齐 `key_configured`），需要落一次盘
    pub needs_save: bool,
}

/// `key` 传 `Err(KeyState::Unknown)` 表示"探测还没回来"：此时既不算缺 Key 也不算读取失败，
/// config 里的缓存标记原样保留，等 `IceApp::apply_key_probe` 拿到真值再对齐。
pub fn prepare(
    dir: &Path,
    key: Result<Option<String>, KeyState>,
    history: Result<HistoryStore, CoreError>,
) -> Prepared {
    let config_path = dir.join("config.toml");
    let (mut config, mut notice) = Config::load_at(&config_path).unwrap_or_else(|err| {
        (
            Config::default(),
            Some(format!("配置目录不可用，本次以默认配置启动：{err}")),
        )
    });

    // 旧术语表只读导入：不删、不改、不重命名用户自己的 glossary.toml
    let legacy = Glossary::import_legacy(&mut config, dir);
    if let Some(text) = legacy.notice {
        push_notice(&mut notice, text);
    }
    let mut needs_save = legacy.imported > 0;

    // 钥匙串是事实来源，config 里的 `key_configured` 只是缓存的标记：
    // 用户在"钥匙串访问"里手动删过条目时，两者会不一致，此时以钥匙串为准。
    let (key_state, api_key) = match key {
        Ok(found) => match found {
            Some(key) => {
                let (len, hash8) = secrets::fingerprint(&key);
                tracing::info!(key_len = len, key_hash8 = hash8, "读到 API Key");
                (KeyState::Configured { len, hash8 }, Some(key))
            }
            None => (KeyState::Missing, None),
        },
        Err(KeyState::Unknown) => (KeyState::Unknown, None),
        // 钥匙串被锁或权限被拒：AI 通道不可用，机翻照常，应用继续跑（REQ-005）
        Err(reason) => {
            tracing::warn!("钥匙串不可用，AI 通道本次不启用");
            push_notice(
                &mut notice,
                format!("钥匙串不可用：{}", reason_text(&reason)),
            );
            (reason, None)
        }
    };
    if key_state != KeyState::Unknown && config.ai.key_configured != key_state.configured() {
        config.ai.key_configured = key_state.configured();
        needs_save = true;
    }

    let store = match history {
        Ok(store) => store,
        Err(err) => {
            push_notice(&mut notice, format!("历史不可用，本次不记录历史：{err}"));
            HistoryStore::at(dir.join("history.jsonl"))
        }
    };

    Prepared {
        glossary: Glossary::from_config(&config),
        config,
        key: key_state,
        api_key,
        history: store,
        notice,
        needs_save,
    }
}

fn push_notice(notice: &mut Option<String>, text: impl Into<String>) {
    match notice {
        Some(existing) => existing.push_str(&format!("；{}", text.into())),
        None => *notice = Some(text.into()),
    }
}

/// `KeyState::Error` 自带面向用户的文案，其余状态用枚举的短标签
fn reason_text(key: &KeyState) -> &str {
    match key {
        KeyState::Error(reason) => reason,
        other => other.label(),
    }
}

fn config_dir() -> Result<PathBuf, CoreError> {
    Config::config_dir()
}

/// 应用图标（2026-10-28 定稿的 `assets/icon/icon-1024.png`），随二进制一起编进来。
const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/icon/icon-1024.png");

/// 交给 Dock 的边长。Dock 磁贴最大约 128pt，视网膜下 256px 就到顶，再大只是白占内存。
const APP_ICON_PX: u32 = 256;

/// 把图标解成 eframe 要的 RGBA，并降采样到 [`APP_ICON_PX`]。
///
/// 为什么非要这一项：**不设的话 eframe 退回它自己 `data/icon.png`——那张黑底白 `e`
/// 是 egui 的 logo**（`native/epi_integration.rs:506`），这就是"图标定下了但 Dock 还是不对"
/// 的根因。macOS 上没有"窗口图标"这个东西（winit 的 `set_window_icon` 是空实现，
/// `platform_impl/macos/window_delegate.rs:1541`），能改 Dock 磁贴的只有
/// `NSApplication.setApplicationIconImage`，而 eframe 只在 `viewport.icon` 有值时去调它。
///
/// 为什么不直接把 1024 那张交出去：`from_png_bytes` 会解成 4MB RGBA，而 eframe 下发时
/// `IconDataExt::to_image` 又 clone 一份，启动那一帧白占约 8MB。降采样后是 256KB。
fn app_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(APP_ICON_PNG).ok()?.into_rgba8();
    let size = APP_ICON_PX.min(image.width().min(image.height()));
    let resized =
        image::imageops::resize(&image, size, size, image::imageops::FilterType::Lanczos3);
    Some(egui::IconData {
        width: resized.width(),
        height: resized.height(),
        rgba: resized.into_raw(),
    })
}

/// 拿不到锁就退出：不建窗口、不写文件（REQ-009）
fn acquire_lock(dir: &Path) -> InstanceLock {
    match single_instance::acquire(dir) {
        Ok(lock) => lock,
        Err(err) => {
            let pid = match err {
                AcquireError::AlreadyRunning { pid } => Some(pid),
                AcquireError::Io(_) => None,
            };
            tracing::error!(pid = pid, "无法启动：单实例锁被占用或不可写");
            eprintln!("{err}");
            exit(1);
        }
    }
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let dir = match config_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("Ice Translator 找不到配置目录：{err}");
            exit(1);
        }
    };

    let lock = acquire_lock(&dir);
    tracing::info!(pid = lock.pid(), path = %lock.path().display(), "取得单实例锁");
    let prepared = prepare(&dir, Err(KeyState::Unknown), HistoryStore::open_default());

    let config_path = dir.join("config.toml");
    if prepared.needs_save {
        if let Err(err) = prepared.config.save_at(&config_path) {
            // 不致命：本次会话仍然用内存里的配置，只是下次启动会重做一遍导入
            tracing::error!("启动配置未能落盘: {err}");
        }
    }

    // Key 还在后台线程上读，此刻先只装机翻；探测到 Key 后由 apply_key_probe 补上 AI 引擎
    let translator = build_translator(&prepared.config, prepared.api_key);
    let state = AppState::new(
        prepared.config,
        prepared.glossary,
        prepared.key,
        prepared.notice,
    );
    let mut viewport = mode::initial_viewport(&state.config.ui);
    // Dock 磁贴（macOS）/ 任务栏（Windows）走这一项，缺省会显示成 egui 的 logo，见 `app_icon`
    viewport.icon = app_icon().map(std::sync::Arc::new);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("Ice Translator 无法启动异步运行时：{err}");
            exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    // 启动形态直接由保存的模式决定，避免"先开大窗再缩成简洁窗"的闪烁（FR-3.1）
    if let Err(err) = eframe::run_native(
        "Ice Translator",
        options,
        Box::new(move |cc| {
            let key_probe = spawn_key_probe(&runtime, &cc.egui_ctx);
            Ok(Box::new(IceApp::new(
                state,
                runtime,
                translator,
                prepared.history,
                config_path,
                Some(lock),
                key_probe,
            )))
        }),
    ) {
        eprintln!("Ice Translator 窗口创建失败：{err}");
        exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_core::glossary::Term;
    use ice_core::language::Language;
    use std::fs;

    fn write_legacy(dir: &Path, body: &str) {
        fs::write(dir.join("glossary.toml"), body).unwrap();
    }

    /// 测试一律用临时目录里的历史文件：`open_default()` 会碰到用户真实的历史
    fn history_in(dir: &Path) -> Result<HistoryStore, CoreError> {
        Ok(HistoryStore::at(dir.join("history.jsonl")))
    }

    #[test]
    fn the_embedded_app_icon_survives_decode_and_is_downscaled_for_the_dock() {
        let icon = app_icon().expect("assets/icon/icon-1024.png 解不出来");
        assert_eq!(icon.width, icon.height, "Dock 磁贴是方的，源图也得是方的");
        assert_eq!(icon.width, APP_ICON_PX);
        assert_eq!(
            icon.rgba.len(),
            (APP_ICON_PX * APP_ICON_PX * 4) as usize,
            "RGBA 缓冲必须与边长吻合，否则 eframe 建 NSBitmapImageRep 会错位"
        );
        assert!(
            icon.rgba.iter().step_by(4).skip(200).any(|&a| a == 0),
            "图标四角本该是透明的；整张不透明说明源图被换成了带底色的方图"
        );
    }

    #[test]
    fn legacy_glossary_is_imported_once_and_the_config_is_marked_for_saving() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(
            dir.path(),
            "[[terms]]\nsource = \"pod\"\ntarget = \"舱体\"\n\n[[terms]]\nsource = \"node\"\ntarget = \"节点\"\n",
        );

        let prepared = prepare(dir.path(), Ok(None), history_in(dir.path()));

        assert_eq!(prepared.config.terms.len(), 2);
        assert_eq!(prepared.glossary.terms.len(), 2);
        assert!(
            prepared.needs_save,
            "导入结果必须落盘，否则每次启动都会重导"
        );
        assert!(prepared.notice.unwrap().contains("导入 2 条术语"));

        // 落盘之后原文件仍在、且内容未变（只读导入）
        prepared
            .config
            .save_at(&dir.path().join("config.toml"))
            .unwrap();
        let raw = fs::read_to_string(dir.path().join("glossary.toml")).unwrap();
        assert!(raw.contains("[[terms]]"));

        let again = prepare(dir.path(), Ok(None), history_in(dir.path()));
        assert!(!again.needs_save, "config 里已有术语就不该再导入");
        assert_eq!(again.config.terms.len(), 2);
    }

    #[test]
    fn a_broken_legacy_glossary_does_not_block_startup() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(dir.path(), "这不是 TOML = =");

        let prepared = prepare(dir.path(), Ok(None), history_in(dir.path()));

        assert!(prepared.config.terms.is_empty());
        assert!(!prepared.needs_save);
        assert!(prepared.notice.unwrap().contains("旧术语表解析失败"));
    }

    #[test]
    fn the_legacy_file_is_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(
            dir.path(),
            "[[terms]]\nsource = \"pod\"\ntarget = \"舱体\"\n",
        );
        let before = fs::read_to_string(dir.path().join("glossary.toml")).unwrap();
        let mtime = fs::metadata(dir.path().join("glossary.toml"))
            .unwrap()
            .modified()
            .unwrap();

        let prepared = prepare(dir.path(), Ok(None), history_in(dir.path()));
        prepared
            .config
            .save_at(&dir.path().join("config.toml"))
            .unwrap();

        assert_eq!(
            fs::read_to_string(dir.path().join("glossary.toml")).unwrap(),
            before
        );
        assert_eq!(
            fs::metadata(dir.path().join("glossary.toml"))
                .unwrap()
                .modified()
                .unwrap(),
            mtime
        );
    }

    #[test]
    fn a_key_in_the_keychain_is_reflected_in_config_and_fingerprinted() {
        let dir = tempfile::tempdir().unwrap();
        let prepared = prepare(
            dir.path(),
            Ok(Some("sk-test-value".into())),
            history_in(dir.path()),
        );

        assert!(prepared.config.ai.key_configured);
        assert!(prepared.needs_save, "标记与钥匙串不一致就要写回去");
        assert!(matches!(prepared.key, KeyState::Configured { len: 13, .. }));
        assert_eq!(prepared.api_key.as_deref(), Some("sk-test-value"));
        assert!(prepared.notice.is_none());
    }

    #[test]
    fn a_stale_key_configured_flag_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.ai.key_configured = true;
        config.save_at(&dir.path().join("config.toml")).unwrap();

        // 用户在"钥匙串访问"里把条目删了
        let prepared = prepare(dir.path(), Ok(None), history_in(dir.path()));

        assert!(!prepared.config.ai.key_configured);
        assert!(prepared.needs_save);
        assert_eq!(prepared.key, KeyState::Missing);
        assert!(prepared.api_key.is_none());
    }

    #[test]
    fn an_unavailable_keychain_degrades_to_machine_translation_with_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let broken = Err(KeyState::Error("钥匙串被锁定".into()));

        let prepared = prepare(dir.path(), broken, history_in(dir.path()));

        assert!(prepared.api_key.is_none());
        assert!(!prepared.config.ai.key_configured);
        let notice = prepared.notice.unwrap();
        assert!(notice.contains("钥匙串不可用"), "{notice}");
        assert!(notice.contains("钥匙串被锁定"), "{notice}");
    }

    #[test]
    fn an_unavailable_history_file_is_reported_and_startup_continues() {
        let dir = tempfile::tempdir().unwrap();
        let broken = Err(CoreError::History("权限被拒".into()));

        let prepared = prepare(dir.path(), Ok(None), broken);

        let notice = prepared.notice.unwrap();
        assert!(notice.contains("历史不可用"), "{notice}");
        // 历史坏掉不影响翻译：源文与提交路径都还在
        let mut state = AppState::new(prepared.config, prepared.glossary, prepared.key, None);
        state.source = "hello".into();
        assert!(state.submit().is_some());
    }

    #[test]
    fn several_problems_are_joined_into_one_notice() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(dir.path(), "坏掉的 TOML = =");

        let prepared = prepare(
            dir.path(),
            Err(KeyState::Error("钥匙串被锁定".into())),
            Err(CoreError::History("权限被拒".into())),
        );

        let notice = prepared.notice.unwrap();
        assert_eq!(notice.matches('；').count(), 2, "{notice}");
        assert!(notice.contains("旧术语表解析失败"), "{notice}");
        assert!(notice.contains("钥匙串不可用"), "{notice}");
        assert!(notice.contains("历史不可用"), "{notice}");
    }

    #[test]
    fn a_pending_key_probe_keeps_the_cached_flag_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.ai.key_configured = true;
        config.save_at(&dir.path().join("config.toml")).unwrap();

        let prepared = prepare(dir.path(), Err(KeyState::Unknown), history_in(dir.path()));

        assert_eq!(prepared.key, KeyState::Unknown);
        assert!(prepared.api_key.is_none());
        assert!(
            prepared.config.ai.key_configured,
            "读取还没回来，不能按缺 Key 处理"
        );
        assert!(!prepared.needs_save);
        assert!(
            prepared.notice.is_none(),
            "探测未完成不是错误，不该提示用户"
        );
    }

    #[test]
    fn a_clean_startup_has_no_notice_and_nothing_to_save() {
        let dir = tempfile::tempdir().unwrap();
        let prepared = prepare(dir.path(), Ok(None), history_in(dir.path()));
        assert!(prepared.notice.is_none());
        assert!(!prepared.needs_save);
        assert_eq!(prepared.config, Config::default());
        assert_eq!(prepared.config.translation.target, Language::ZhHans);
    }

    #[test]
    fn an_existing_config_is_sanitized_and_terms_survive() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.ui.font_scale = 99.0;
        config.terms = vec![Term::new("pod", "舱体")];
        config.save_at(&dir.path().join("config.toml")).unwrap();

        let prepared = prepare(dir.path(), Ok(None), history_in(dir.path()));

        assert_eq!(prepared.config.ui.font_scale, 2.0);
        assert_eq!(prepared.config.terms.len(), 1);
        assert!(!prepared.needs_save);
    }
}
