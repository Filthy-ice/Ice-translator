//! 界面字体：中文从系统的苹方换成**黑体-简**（2026-10-28 变更 H）。
//!
//! 用户原话"能不能把字体全部更换为黑体？感觉字看上去有点发灰不舒服。"
//!
//! **发灰的成因不是颜色。** egui 0.37 自带的字体清单里没有任何中文字形，中文是每帧按字符
//! 现问 `egui_system_font_provider`（内部走 fontique → CoreText）。本机取证打到的那一行是：
//!
//! ```text
//! DEBUG egui_system_font_provider: Using "system:PingFang SC:0:3" for '文'
//! ```
//!
//! 也就是当前正文用的是**苹方**——它偏细，在这台 1920×1080（1x，非视网膜）的屏上、
//! 12.5px 的小字号正好落在它最难看的区间，笔画边缘被抗锯齿糊开，看着就是"发灰"。
//! 用户说的"黑体"在 macOS 上就是 黑体-简（`STHeiti Medium.ttc`，字重比苹方重、字形方正），
//! 换过去正是他要的观感——所以这里换的是**字体族**，一处颜色都没动。
//!
//! 三处不显然的约束：
//!
//! 1. **`STHeiti Medium.ttc` 里有两面，第 0 面是黑体-繁（`STHeitiTC-Medium`），第 1 面才是
//!    黑体-简（`STHeitiSC-Medium`）**。两面都覆盖常用简体字，所以选错不会崩、也不会缺字，
//!    只会悄悄用上半繁体字形——这类错只有肉眼看得到，因此这里把索引写死并留了 `#[test]` 核对。
//! 2. **Monospace 族里黑体排在 Hack 之后而不是之前**：对照页等宽列靠 Hack 的等宽拉丁，
//!    把黑体插到第 0 位会让拉丁也变成比例宽度、列对不齐。中文照旧命中黑体（Hack 没有汉字字形，
//!    同一族里往后找第一个有的）。
//! 3. **这台机器上没有这个文件也得照常起来**：读不到就什么都不做，中文回落到系统 provider
//!    （就是改动前的行为），并记一条 debug——不能因为找不到字体让界面空白。
//!
//! 一个已知代价：`STHeiti Medium.ttc` 是 55MB，`FontData` 要求字面数据在内存里
//! （`egui::FontData::from_blob` 只接 `Arc<[u8]>` 那类共享字节，没有文件映射的入口），
//! 所以进程多常驻约 55MB。这是"固定用某个系统的某一款字体"的必要开销。

use egui::{Context, FontData, FontDefinitions, FontFamily};

/// macOS 上 黑体-简 的文件位置（`系统偏好设置 → 字体册`里显示为"黑体-简 中等"）
pub const HEITI_PATH: &str = "/System/Library/Fonts/STHeiti Medium.ttc";
/// 同一文件里的第 1 面 = `STHeitiSC-Medium`（第 0 面是繁体，见模块注释）
pub const HEITI_SC_FACE_INDEX: u32 = 1;
/// 装进 `FontDefinitions` 时用的族名，只在内部用，不上界面
pub const HEITI_FAMILY: &str = "heiti-sc";

/// 装字体要用的那批汉字：界面与正文里真会出现的字
#[cfg(test)]
pub const HEITI_CHECK_SAMPLE: &str =
    "中文翻译简体字数检测耗时缓存分段设置窗口悬浮全量退出置顶复制自动浅色深色";

/// 按 macOS 默认字体清单 + 黑体首选，拼出要交给 egui 的那份 `FontDefinitions`。
///
/// 拆成纯函数是为了能断言"黑体真的排在第 0 位"——`set_fonts` 之后 egui 不暴露族名顺序，
/// 光看界面只能靠肉眼，而"装上了却排在 Ubuntu-Light 后面"正是最容易犯又最难发现的错。
/// 返回 `None` = 这台机器上拿不到这个文件。
pub fn definitions() -> Option<FontDefinitions> {
    let bytes = std::fs::read(HEITI_PATH).ok()?;
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        HEITI_FAMILY.to_owned(),
        FontData::from_blob(std::sync::Arc::new(bytes), HEITI_SC_FACE_INDEX).into(),
    );
    // 正文与控件（Proportional）：黑体排在最前，拉丁一并换成黑体——用户要的是"全部"
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, HEITI_FAMILY.to_owned());
    // 等宽（Monospace）：Hack 留在第 0 位保住拉丁等宽，黑体紧随其后接管汉字
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    let at = mono
        .iter()
        .position(|name| name == "Hack")
        .map_or(mono.len(), |i| i + 1);
    mono.insert(at, HEITI_FAMILY.to_owned());
    Some(fonts)
}

/// 把黑体装成界面首选字体。返回 `false` = 这台机器上拿不到这个文件，界面按改动前的方式跑。
///
/// 只应在启动时调用一次：`set_fonts` 是整体替换字体清单，每帧调用会把已缓存的字形图集作废。
pub fn install(ctx: &Context) -> bool {
    let Some(fonts) = definitions() else {
        tracing::debug!("没找到黑体字体文件，中文沿用系统解析");
        return false;
    };
    ctx.set_fonts(fonts);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::FontId;

    /// 装好之后，汉字必须真的落在黑体这一族上（而不是回落给系统 provider）。
    /// 这一条同时兜住"文件在但面索引错了/文件坏了"——那种情况下 `has_glyphs` 会假。
    #[test]
    fn han_glyphs_come_from_the_installed_heiti() {
        let ctx = Context::default();
        if !install(&ctx) {
            // 不是 macOS，或这台机器没有这个文件：没得装就不该硬失败
            return;
        }
        // 字体清单要到第一帧才建起来（`egui/context.rs` 的 "No fonts available until first call"）
        ctx.run_ui(egui::RawInput::default(), |_ui| {})
            .drop_without_applying_deltas();
        let id = FontId::new(12.5, FontFamily::Proportional);
        assert!(
            ctx.fonts_mut(|f| f.has_glyphs(&id, HEITI_CHECK_SAMPLE)),
            "黑体装上去了却画不出常用汉字，多半是面索引选错了"
        );
    }

    /// 首选位真的是黑体：Proportional 第 0 位、Monospace 第 1 位（Hack 之后）
    #[test]
    fn heiti_is_the_first_proportional_face_and_second_monospace_one() {
        let Some(fonts) = definitions() else {
            return;
        };
        assert_eq!(
            fonts.families[&FontFamily::Proportional]
                .first()
                .map(String::as_str),
            Some(HEITI_FAMILY),
            "正文没落在黑体上，界面会继续用苹方——就是用户说的那个\"发灰\""
        );
        let mono = &fonts.families[&FontFamily::Monospace];
        assert_eq!(mono.first().map(String::as_str), Some("Hack"));
        assert_eq!(mono.get(1).map(String::as_str), Some(HEITI_FAMILY));
        assert!(fonts.font_data.contains_key(HEITI_FAMILY));
    }
}
