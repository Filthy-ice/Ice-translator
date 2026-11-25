//! 系统字体路径与 face 号的唯一出处（CHG-006 Q-003）。
//!
//! 这两样以前在仓里各写了一份：`ice-app::fonts` 交给 egui，`ice-vision` 的自绘度量交给
//! `owned_ttf_parser`。屏幕上的字和度量图上的字必须来自同一支字体的同一面，否则
//! "量出来的数"和"用户看到的"根本是两件事——而这两处一旦各自漂移，没有任何一条测试会发现。
//!
//! 选面这件事只有肉眼判得了：`STHeiti Medium.ttc` 里第 0 面是黑体-繁（`STHeitiTC-Medium`）、
//! 第 1 面才是黑体-简（`STHeitiSC-Medium`），两面都覆盖常用简体字，**选错既不崩也不缺字**，
//! 只是字形悄悄换成半繁体。所以这里给的是显式常量，不是 `0` 碰运气；界面侧的核对锁在
//! `ice-app::fonts::tests`，度量侧直接用同一个常量。
//!
//! 后备那支（`GOTHIC_PATH`）是因为黑体里没有谚文字形也没有西里尔字形：用同一张 `Face` 画韩文
//! 只会得到一片空白，量出来的 0.0000 说不清是"没认出来"还是"没画出来"。它是单面字体，不必猜
//! `.ttc` 的第几面，cmap 里谚文、西里尔、假名、拉丁都在。

/// macOS 上 黑体-简 的文件位置（`系统偏好设置 → 字体册` 里显示为"黑体-简 中等"）
pub const HEITI_PATH: &str = "/System/Library/Fonts/STHeiti Medium.ttc";

/// 同一文件里的第 1 面 = `STHeitiSC-Medium`（第 0 面是繁体，见模块注释）
pub const HEITI_SC_FACE_INDEX: u32 = 1;

/// 黑体缺字形时的后备字体
pub const GOTHIC_PATH: &str = "/System/Library/Fonts/Supplemental/AppleGothic.ttf";

/// `heiti_probe` 逐面取证时扫的候选：第一条是生产用的那支，后两条只是对照。
///
/// 生产代码只读 [`HEITI_PATH`]；这张表存在的意义是把"对照也在这一个文件里点名"这件事固定住，
/// 免得探测脚本再抄一份路径出去。
pub const HEITI_PROBE_CANDIDATES: &[&str] = &[
    HEITI_PATH,
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
];

/// Linux 侧的中文字体候选（CHG-017 REQ-008）。
///
/// 与 mac 那条"显式常量而不是 `0` 碰运气"是同一条纪律，只是这里给的是**候选表**而不是单个面号：
/// `.ttc` 里的面次序不是发行版承诺（本机实测 `NotoSansCJK-Regular.ttc` 第 2 面是
/// `Noto Sans CJK SC`，第 0 面是 JP；`fc-list` 同一文件还能列出 TC/HK/KR），面号由
/// `ice-vision::paint` 按 name 表现选。顺序即优先级：Regular 面是绘制用的那支，Bold 只在
/// Regular 缺席时兜底，Droid 是"只有 fallback 字体"的旧桌面。
///
/// 生产代码读这张表；`ice-vision/examples/cjk_face_probe.rs` 取证也扫同一张表，免得对照与
/// 真实路径漂移（[`HEITI_PROBE_CANDIDATES`] 的理由）。
pub const LINUX_CJK_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
];

/// 选面时认的族名后缀：`Noto Sans CJK SC` 的这一面才是简体中文字形（JP 面的"直/骨/门"等字
/// 是日式写法，覆盖同样齐全，选错不崩也不缺字——与 [`HEITI_SC_FACE_INDEX`] 那条完全同型）
pub const LINUX_SC_FAMILY_SUFFIX: &str = "SC";

/// 等宽面的名字里含这个词，选主字体时要避开：`Noto Sans Mono CJK SC` 的拉丁是等宽的，
/// 用它度量正文会把英文行拉宽
pub const LINUX_MONO_FAMILY_WORD: &str = "Mono";

/// 中文字体之外那一支纯拉丁后备（黑体缺字形时的 mac 后备 `GOTHIC_PATH` 的对应物）。
/// 面号这里可以是 0：DejaVu 是单面字体，没有"第几面"这回事。
pub const LINUX_LATIN_PATH: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

/// 一个中文字体都没有时给用户的那句话——REQ-008 要求可行动，不许只报 `VisionError::Font`
pub const LINUX_MISSING_HINT: &str =
    "本机没有可用的中文字体，安装 fonts-noto-cjk 后重启即可恢复译文图片导出";

#[cfg(test)]
mod tests {
    use super::*;

    /// face 号写死成 1 是"半繁体字形只有肉眼看得出"这件事的唯一防线；改成 0 不会有任何测试变红，
    /// 除了这一条。
    #[test]
    fn the_simplified_face_is_index_one() {
        assert_eq!(HEITI_SC_FACE_INDEX, 1);
    }

    /// 探测脚本扫的第一条必须就是生产用的那支：否则对照表跟真实路径漂移了，取证取的是别的字体。
    #[test]
    fn the_probe_list_starts_with_the_production_font() {
        assert_eq!(HEITI_PROBE_CANDIDATES[0], HEITI_PATH);
    }

    /// Linux 的候选表第一条必须是 Noto Regular（本机 2026-11-25 `fc-match` 实测命中它），
    /// 且表里不许有 mac 路径——移植的边界是"只加 Linux 分支"，公共表里混进苹果路径就是越界。
    #[test]
    fn the_linux_candidates_are_linux_paths_ordered_by_priority() {
        assert_eq!(
            LINUX_CJK_CANDIDATES[0],
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"
        );
        assert!(
            LINUX_CJK_CANDIDATES
                .iter()
                .all(|p| p.starts_with("/usr/share/fonts/")
                    || p.starts_with("/usr/local/share/fonts/")),
            "候选表里出现了非 Linux 字体目录"
        );
        assert!(LINUX_CJK_CANDIDATES.iter().all(|p| !p.contains("System")));
    }

    /// 后备那支必须是纯拉丁（有 `a` 没汉字）之外的可信单面字体；路径写错不会崩，只会让
    /// "黑体没有的字形没人接"变成静默降级，所以钉在常量上而不是运行期才发现
    #[test]
    fn the_latin_fallback_is_named_explicitly() {
        assert_eq!(
            LINUX_LATIN_PATH,
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
        );
        assert_eq!(LINUX_SC_FAMILY_SUFFIX, "SC");
        assert_eq!(LINUX_MONO_FAMILY_WORD, "Mono");
        assert!(LINUX_MISSING_HINT.contains("fonts-noto-cjk"));
    }
}
