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
}
