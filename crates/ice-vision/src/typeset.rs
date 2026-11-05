//! S6 版式计划：一块译文在这块框里怎么排（CHG-006 REQ-004 / REQ-006）。
//!
//! 与 `bg.rs` 同一条纪律：**零 IO、零 `unsafe`、不碰任何 UI crate**（NFR-6）。这个模块连像素
//! 都不读——宽度问注入进来的 `Metrics`，首行墨迹范围由调用方从 `bg::first_line_extent` 取好再
//! 交进来，于是这里每一条判据都能在无头里用一把假尺子钉死。
//!
//! 四件不显然的事：
//!
//! 1. **度量是注入的**。没有它，"缩到 55% 仍放不下"这类断言要等系统字体文件在位才能测，而
//!    Linux 上根本没有那本字体（NFR-7）。真字体那份实现是 `paint::FileMetrics`。
//! 2. **两条标点禁则在切单位时解决，不在填行之后补救**。闭合类（`，。`）黏到前一个字尾部、开启类
//!    （`（《`）黏到后一个字头部，于是"行首出现逗号"在结构上不可能发生，填行那一圈不需要额外状态。
//! 3. **放不下不裁字**。缩到下限仍放不下就改判覆盖层并标 `degraded`；越框也不许少一个字（FR-2.7 的
//!    验收原文是"无文字被裁切丢失"）。`Skip` 不是这件事的出口，它只有一种来路：这块没有译文。
//! 4. **长度不在这里再设一个上限**。块的文本在进入本层之前已被 CHG-005 的
//!    `layout::MergeParams::max_chars`（1200）管住，本模块不新增第二个数，也不做截断——截断就是
//!    静默丢字。

use ice_core::document::{PixelRect, TextBlock};
use ice_core::language::breaks_lines_per_char;
use image::Rgba;

use crate::bg::{BackgroundEstimate, BackgroundKind, contrast, relative_luminance};
use crate::composite::CompositeOptions;

/// 缩字号的步进（docs/04 §4 第 4 条）
pub const SHRINK_STEP: f32 = 0.95;
/// 行高 = 字号 × 它。docs/04 没给行高模型，这是本包补的数：常规正文行距 1.2~1.5 的下沿，
/// 回填要的是"别压线"，宁可略松
pub const LINE_HEIGHT_RATIO: f32 = 1.25;
/// 首行中心与块框中心之差在此比例内（× 块宽）判居中。docs/04 §4 第 5 条只写了"依据首行左边界与
/// 块框中心的相对位置推断"，没给容差——没有容差这条判据无法实现
pub const ALIGN_CENTER_TOLERANCE: f32 = 0.15;
/// 覆盖层的文字区相对底板内缩多少像素（底板本身 = 块框，一个像素都不越框）
pub const OVERLAY_INSET: f32 = 3.0;
/// 覆盖层底色与字色的最低对比度（WCAG AA 正文）
pub const MIN_CONTRAST: f64 = 4.5;
/// 字色亮度分界 = √(0.05 × 1.05) − 0.05，即"底板走到纯黑"与"走到纯白"所能拿到的最大对比度相等
/// 的那一点（两边都是 4.58:1）。高于它往黑走、低于它往白走，于是走到极端必然达标，底色循环有界。
const LUMINANCE_SPLIT: f64 = 0.179_1;
/// 底色向极端色逼近的步数：走完就是纯黑/纯白
const PLATE_STEPS: u32 = 32;
/// 这些标点不许出现在行首（闭合类，黏到前一个字尾部）
pub const LINE_START_FORBIDDEN: &str = "，。、；：！？）》」』";
/// 这些标点不许出现在行末（开启类，黏到后一个字头部）
pub const LINE_END_FORBIDDEN: &str = "（《「『";

/// 画在哪一层由谁定：逐块自动，还是用户在界面单选里强制
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositeMode {
    /// 逐块由背景档位决定（`Complex` → 覆盖层，其余 → 回填）
    #[default]
    Auto,
    /// 强制回填：不许改判覆盖层，放不下就越框画
    ReplaceOnly,
    /// 强制覆盖层：一个像素都不擦
    OverlayAll,
}

/// 一块的画法结论
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    /// 就地擦除后回填
    InPlace,
    /// 不擦，在块框上铺一块底板再写
    Overlay,
    /// 什么都不做，那块区域原样保留
    Skip,
}

/// 原块的水平对齐；回填与覆盖层用同一个
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// 计划里的一行：文本 + 按同一把尺量出的宽度。
///
/// 宽度存下来而不是让绘制侧再算：REQ-005 那条"行宽与 `Metrics::text_width` 一致"量的就是"计划
/// 给的数"与"绘制用的数"是不是同一个。
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedLine {
    pub text: String,
    pub width: f32,
}

/// 文本度量。真实现是 `paint::FileMetrics`（走 `owned_ttf_parser`），测试用一把假尺。
pub trait Metrics {
    /// 这段文本按 `font_size` 排开有多宽
    fn text_width(&self, text: &str, font_size: f32) -> f32;

    /// 行高。默认 `字号 × LINE_HEIGHT_RATIO`——不用字面上下伸部，那会让中英混排的同一行抖动
    fn line_height(&self, font_size: f32) -> f32 {
        font_size * LINE_HEIGHT_RATIO
    }
}

/// 一块译文排完之后的全部结论。绘制侧只需要它加 `block.bbox`（覆盖层底板就是块框）。
#[derive(Debug, Clone, PartialEq)]
pub struct BlockPlan {
    pub kind: PlanKind,
    pub font_size: f32,
    pub lines: Vec<PlannedLine>,
    pub align: Align,
    /// 缩到下限仍放不下（FR-2.7）：G 区要用橙描边标出来
    pub degraded: bool,
    /// 字色：取 `BackgroundEstimate.fg`，FR-2.8 那句"白字深底回填后仍为白"就是这一条
    pub text_color: Rgba<u8>,
    /// 覆盖层底色；`Overlay` 之外恒为 `None`
    pub plate: Option<Rgba<u8>>,
    /// 文字可用区：回填 = 块框，覆盖层 = 块框内缩 [`OVERLAY_INSET`]
    pub area: PixelRect,
}

impl BlockPlan {
    /// 第 `index` 行的行盒。绘制侧按它落位，无头侧按它断言"字在计划那一格内"。
    ///
    /// `line_height` 由调用方从同一份 `Metrics` 取：几何归本模块，度量归注入的那把尺。
    pub fn line_box(&self, index: usize, line_height: f32) -> Option<PixelRect> {
        let line = self.lines.get(index)?;
        let x = match self.align {
            Align::Left => self.area.x,
            Align::Center => self.area.x + (self.area.w - line.width) / 2.0,
            Align::Right => self.area.right() - line.width,
        };
        Some(PixelRect::new(
            x,
            self.area.y + index as f32 * line_height,
            line.width,
            line_height,
        ))
    }
}

/// 首行墨迹在 x 上的覆盖范围（像素栅格坐标，右缘不含）
pub type InkSpan = (f32, f32);

/// 排一块译文。**纯函数**：输入全是值，不读像素也不读字体。
///
/// `first_line` 是原块首行的墨迹范围，由 [`crate::bg::first_line_extent`] 在**擦除之前**从原图取。
/// 量不到时按左对齐——截图里的文字绝大多数左对齐，猜错的代价最小；而渐变/照片背景上整片都算
/// 墨迹时首行宽度 ≈ 块宽，左、中、右在像素上本来就无法区分，判出什么都等价。
pub fn plan(
    block: &TextBlock,
    estimate: &BackgroundEstimate,
    metrics: &dyn Metrics,
    options: &CompositeOptions,
    mode: CompositeMode,
    first_line: Option<InkSpan>,
) -> BlockPlan {
    let align = align_from_first_line(first_line, block.bbox);
    let base = BlockPlan {
        kind: PlanKind::Skip,
        font_size: block.estimated_font_size(),
        lines: Vec::new(),
        align,
        degraded: false,
        text_color: estimate.fg,
        plate: None,
        area: block.bbox,
    };
    let Some(text) = block.translated_text() else {
        return base;
    };

    // 天然的覆盖层来路：用户强制，或者背景说不清（`Complex` 永不擦）
    let overlay_area = area_of(block.bbox, true);
    let forced = match mode {
        CompositeMode::OverlayAll => true,
        CompositeMode::ReplaceOnly => false,
        CompositeMode::Auto => estimate.kind == BackgroundKind::Complex,
    };
    let mut layout = fit(
        block,
        text,
        if forced { overlay_area } else { block.bbox },
        metrics,
        options,
    );
    // `ReplaceOnly` 是用户显式说了"别铺底板"，放不下也只能越框
    let fell_back = layout.overflow
        && !forced
        && mode != CompositeMode::ReplaceOnly
        && options.fallback_to_overlay;
    let overlay = forced || fell_back;
    // 改判覆盖层的那一块要按内缩后的宽度重排，否则底板里塞不进原来那一串行
    if fell_back {
        layout = fit(block, text, overlay_area, metrics, options);
    }

    BlockPlan {
        kind: if overlay {
            PlanKind::Overlay
        } else {
            PlanKind::InPlace
        },
        font_size: layout.font_size,
        lines: layout.lines,
        align,
        // 只有"缩到底还放不下"算降级：`Complex` 改判覆盖层讲的是背景，不是字被挤
        degraded: layout.overflow,
        text_color: estimate.fg,
        plate: overlay.then(|| plate_color(estimate.bg, estimate.fg)),
        area: area_of(block.bbox, overlay),
    }
}

/// 文字可用区：覆盖层内缩，回填用整块框。内缩不许把区域缩成负数，所以先夹到边长的一半
fn area_of(bbox: PixelRect, overlay: bool) -> PixelRect {
    if !overlay {
        return bbox;
    }
    let inset = OVERLAY_INSET.min(bbox.w / 2.0).min(bbox.h / 2.0);
    PixelRect::new(
        bbox.x + inset,
        bbox.y + inset,
        bbox.w - inset * 2.0,
        bbox.h - inset * 2.0,
    )
}

/// 排一遍的结果，连同"缩到底也放不下"那个事实
struct Layout {
    font_size: f32,
    lines: Vec<PlannedLine>,
    overflow: bool,
}

/// 从 `estimated_font_size()` 起按 [`SHRINK_STEP`] 缩到 `min_font_scale` 下限（FR-2.7 那句 55%）。
///
/// 有一个后果要写在这里而不是等真机发现：起始字号 = 0.75 × 框高、行高 = 1.25 × 字号，于是起始
/// 容量恒为 1 行；缩到 55% 时容量仍是 1 行（`1 ÷ (1.25 × 0.75 × 0.55) = 1.94`）。也就是说**译文
/// 一旦需要换行，就只能靠缩字号在一行内解决**，解决不了就走覆盖层 + 降级。这与 docs/04 §4 的
/// 写法一致；它在真实截图上是不是太严，属 REQ-010 的人工评分（W-4.3），调数不改结构。
fn fit(
    block: &TextBlock,
    text: &str,
    area: PixelRect,
    metrics: &dyn Metrics,
    options: &CompositeOptions,
) -> Layout {
    let floor = block.estimated_font_size() * options.min_font_scale.max(0.0);
    let mut font_size = block.estimated_font_size();
    loop {
        let lines = break_lines(text, area.w, font_size, metrics);
        let capacity = (area.h / metrics.line_height(font_size)).floor().max(0.0);
        let overflow =
            lines.len() as f32 > capacity || lines.iter().any(|line| line.width > area.w);
        if !overflow || font_size <= floor {
            return Layout {
                font_size,
                lines,
                overflow,
            };
        }
        font_size = (font_size * SHRINK_STEP).max(floor);
    }
}

/// 按宽度把译文切成行：汉字/假名逐字、其余按词、标点不落行首也不落行末。
///
/// 判"逐字"用的是字符本身（[`breaks_lines_per_char`]）而不是 `Language::breaks_by_char`：合成链
/// 里没有任何语言对象可问（`compose` 不收语言参数，用户选的语向也推不出块内文字的真实书写系统，
/// 中英混排的块太常见）。硬换行（`\n`，识别链按行合并时留下的）照原样断开。
pub fn break_lines(
    text: &str,
    max_width: f32,
    font_size: f32,
    metrics: &dyn Metrics,
) -> Vec<PlannedLine> {
    let mut lines: Vec<PlannedLine> = Vec::new();
    let space = metrics.text_width(" ", font_size);
    for paragraph in text.split('\n') {
        let mut current = String::new();
        let mut width = 0.0f32;
        for unit in units_of(paragraph) {
            let unit_width = metrics.text_width(&unit.text, font_size);
            let gap = if current.is_empty() || !unit.space_before {
                0.0
            } else {
                space
            };
            if !current.is_empty() && width + gap + unit_width > max_width {
                lines.push(PlannedLine {
                    text: std::mem::take(&mut current),
                    width,
                });
                // 行首不留空格：空格本来就只记在单位属性上，不进文本
                current.push_str(&unit.text);
                width = unit_width;
            } else {
                if gap > 0.0 {
                    current.push(' ');
                }
                current.push_str(&unit.text);
                width += gap + unit_width;
            }
        }
        if !current.is_empty() {
            lines.push(PlannedLine {
                text: current,
                width,
            });
        }
    }
    lines
}

/// 一个必须待在同行里的片段
struct Unit {
    text: String,
    /// 前面原本有一个空格：可以在这里断行，但断完不算进行首
    space_before: bool,
}

/// 把一段文字切成单位。
///
/// 汉字/假名一字一单位（哪里都能断），其余连续字符成一个单位（词内不断）；空格不进单位文本，
/// 只记成"下一个单位前面有空格"。开启类标点做下一个单位的前缀、闭合类标点追加到上一个单位尾部
/// ——两条禁则由此变成结构性质，代价是带标点的单位比同行别的单位宽一格，这正是标点被挤回上一行
/// 末的应有之义。
fn units_of(paragraph: &str) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::new();
    let mut current = String::new();
    let mut space_before = false;
    // 开启类标点要黏住后一个字；段首没有后一个字时它自己成单位（如实：这里满足不了禁则）
    let mut prefix = String::new();
    // 当前单位是不是还在一段拉丁词里（词内不断，汉字/假名之后重新起算）
    let mut inside_word = false;
    for ch in paragraph.chars() {
        if ch.is_whitespace() {
            flush(&mut units, &mut current, &mut space_before);
            space_before = true;
            inside_word = false;
            continue;
        }
        if LINE_START_FORBIDDEN.contains(ch) {
            flush(&mut units, &mut current, &mut space_before);
            match units.last_mut() {
                Some(previous) => previous.text.push(ch),
                // 段首孤零零一个标点：不强行造空单位，也不许它去黏住一个空格
                None => current.push(ch),
            }
            space_before = false;
            inside_word = false;
            continue;
        }
        if LINE_END_FORBIDDEN.contains(ch) {
            flush(&mut units, &mut current, &mut space_before);
            prefix.push(ch);
            inside_word = false;
            continue;
        }
        let per_char = breaks_lines_per_char(ch);
        if inside_word && !per_char {
            current.push(ch);
        } else {
            flush(&mut units, &mut current, &mut space_before);
            current.push_str(&prefix);
            prefix.clear();
            current.push(ch);
        }
        inside_word = !per_char;
        if per_char {
            // 汉字/假名逐字成单位：下一个字符必然另起一行也可以
            flush(&mut units, &mut current, &mut space_before);
        }
    }
    flush(&mut units, &mut current, &mut space_before);
    if !prefix.is_empty() {
        match units.last_mut() {
            Some(last) => last.text.push_str(&prefix),
            None => units.push(Unit {
                text: prefix,
                space_before: false,
            }),
        }
    }
    units
}

fn flush(units: &mut Vec<Unit>, current: &mut String, space_before: &mut bool) {
    if current.is_empty() {
        // 没有单位可发，那个"前面有过空格"的记号要留给下一个单位
        return;
    }
    units.push(Unit {
        text: std::mem::take(current),
        space_before: *space_before,
    });
    // 空格记号被这个单位带走了
    *space_before = false;
}

/// 由首行墨迹推原块对齐（docs/04 §4 第 5 条）。
///
/// 顺序是先判居中、再判贴哪一侧：首行填满整块时中心差为 0，判居中——那一刻左/中/右在像素上分
/// 不开，而居中与贴边同宽，不影响观感。
pub fn align_from_first_line(first_line: Option<InkSpan>, bbox: PixelRect) -> Align {
    let Some((left, right)) = first_line else {
        return Align::Left;
    };
    let tolerance = ALIGN_CENTER_TOLERANCE * bbox.w;
    if (((left + right) / 2.0) - bbox.center_x()).abs() <= tolerance {
        return Align::Center;
    }
    if left - bbox.x <= bbox.right() - right {
        Align::Left
    } else {
        Align::Right
    }
}

/// 覆盖层底色：从 `bg` 出发朝黑或朝白逼近，直到与字色的对比度达标（REQ-004 的公式）。
///
/// 方向按**字色**的亮度判，不按"字比背景亮还是暗"：深色模式里常见的深黑底 + 更深的灰字，按后者
/// 会选"加深"，而加深到纯黑也拉不开它与那行深灰字的距离——按字色落在 [`LUMINANCE_SPLIT`] 哪一侧
/// 走，取的那一边才是能达标的那条路，循环也因此有界（极端色对任何字色都够 4.5:1）。
/// 底板恒为不透明：透明底 PNG 上铺一块 α=0 的"底板"等于没铺，字会直接压回原图。
pub fn plate_color(bg: Rgba<u8>, ink: Rgba<u8>) -> Rgba<u8> {
    let extreme = if relative_luminance(&ink) >= LUMINANCE_SPLIT {
        [0.0, 0.0, 0.0]
    } else {
        [255.0, 255.0, 255.0]
    };
    for step in 0..=PLATE_STEPS {
        let candidate = mix(bg, extreme, step as f32 / PLATE_STEPS as f32);
        if contrast(&candidate, &ink) >= MIN_CONTRAST {
            return candidate;
        }
    }
    Rgba([extreme[0] as u8, extreme[1] as u8, extreme[2] as u8, 255])
}

fn mix(from: Rgba<u8>, to: [f32; 3], ratio: f32) -> Rgba<u8> {
    let channel = |value: u8, target: f32| {
        (f32::from(value) + (target - f32::from(value)) * ratio).round() as u8
    };
    Rgba([
        channel(from.0[0], to[0]),
        channel(from.0[1], to[1]),
        channel(from.0[2], to[2]),
        255,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一把假尺：每个字符（含空格）占 `字号 × 1` 宽。方字模型让每一格的算术都能手算，
    /// 于是断言写的是"缩六步""三行""四十字一个不少"，而不是把算法在测试里再跑一遍。
    struct Square;

    impl Metrics for Square {
        fn text_width(&self, text: &str, font_size: f32) -> f32 {
            text.chars().count() as f32 * font_size
        }
    }

    const DARK: Rgba<u8> = Rgba([10, 10, 10, 255]);
    const LIGHT: Rgba<u8> = Rgba([250, 250, 250, 255]);

    fn block(w: f32, h: f32, translation: Option<&str>) -> TextBlock {
        let mut block = TextBlock::new(1, "原文", PixelRect::new(0.0, 0.0, w, h), 0.9);
        block.translation = translation.map(str::to_string);
        block
    }

    fn estimate(kind: BackgroundKind) -> BackgroundEstimate {
        BackgroundEstimate {
            bg: DARK,
            fg: LIGHT,
            variance: 0.0,
            kind,
            corners: [DARK; 4],
        }
    }

    /// 起始字号 = `h × 0.75`：假尺一行正好是 50×50 那块框里最合适的尺寸
    fn run(h: f32, text: &str, mode: CompositeMode, kind: BackgroundKind) -> BlockPlan {
        plan(
            &block(400.0, h, Some(text)),
            &estimate(kind),
            &Square,
            &CompositeOptions::default(),
            mode,
            Some((0.0, 400.0)),
        )
    }

    fn texts(lines: &[PlannedLine]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    /// 把分行结果拼回去：用来断言"一个字都没少"，比数行数更直白
    fn joined(lines: &[PlannedLine]) -> String {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn it_shrinks_in_nine_five_percent_steps_until_the_line_fits() {
        // 400 宽、50 高 ⇒ 起始 37.5；方字一行装 10 字，14 字要 2 行而容量只有 1 行
        let plan = run(
            50.0,
            &"字".repeat(14),
            CompositeMode::Auto,
            BackgroundKind::Uniform,
        );
        assert_eq!(plan.kind, PlanKind::InPlace);
        assert!(!plan.degraded);
        assert_eq!(plan.lines.len(), 1);
        // 37.5 × 0.95^6 ≈ 27.57，正好是"一行装得下 14 字"的第一档
        assert!(
            (plan.font_size - 37.5 * 0.95f32.powi(6)).abs() < 0.01,
            "步进不该跳档或少走：{}",
            plan.font_size
        );
    }

    #[test]
    fn the_floor_stops_the_shrink_and_retiers_to_overlay() {
        let text = "字".repeat(40);
        let plan = run(50.0, &text, CompositeMode::Auto, BackgroundKind::Uniform);
        assert_eq!(plan.kind, PlanKind::Overlay);
        assert!(plan.degraded);
        assert!(
            (plan.font_size - 37.5 * 0.55).abs() < 0.01,
            "该停在 55% 下限：{}",
            plan.font_size
        );
        // 改判覆盖层后按内缩宽度重排：底板里塞不下的部分越框，但一个字都不许少
        assert_eq!(
            plan.lines
                .iter()
                .map(|l| l.text.chars().count())
                .sum::<usize>(),
            40
        );
        assert_eq!(joined(&plan.lines), text);
    }

    #[test]
    fn forced_replace_overflows_instead_of_overlaying() {
        let text = "字".repeat(40);
        let plan = run(
            50.0,
            &text,
            CompositeMode::ReplaceOnly,
            BackgroundKind::Uniform,
        );
        assert_eq!(plan.kind, PlanKind::InPlace);
        assert!(plan.degraded);
        assert_eq!(plan.plate, None);
        assert_eq!(plan.area, PixelRect::new(0.0, 0.0, 400.0, 50.0));
        // FR-2.7 的验收是"无文字被裁切丢失"：宁可越框
        assert_eq!(joined(&plan.lines), text);
    }

    #[test]
    fn skip_only_comes_from_a_missing_translation() {
        let missing = plan(
            &block(400.0, 50.0, None),
            &estimate(BackgroundKind::Complex),
            &Square,
            &CompositeOptions::default(),
            CompositeMode::OverlayAll,
            None,
        );
        assert_eq!(missing.kind, PlanKind::Skip);
        assert!(missing.lines.is_empty());
        assert_eq!(missing.plate, None);
        // 翻出来是空的同样没得画（不是"画不下就算了"那条路）
        let blank = run(50.0, "   ", CompositeMode::Auto, BackgroundKind::Uniform);
        assert_eq!(blank.kind, PlanKind::Skip);
        // 而"画不下"绝不走 Skip：上面那几条测的都是 Overlay/InPlace
        let tight = run(
            50.0,
            &"字".repeat(400),
            CompositeMode::Auto,
            BackgroundKind::Uniform,
        );
        assert_ne!(tight.kind, PlanKind::Skip);
        assert!(tight.degraded);
    }

    #[test]
    fn latin_breaks_at_spaces_and_never_splits_a_word() {
        let lines = break_lines("alpha beta", 90.0, 10.0, &Square);
        assert_eq!(texts(&lines), ["alpha", "beta"]);
        assert_eq!(lines[1].width, 40.0, "行首那个空格不该占位");
    }

    #[test]
    fn cjk_breaks_at_any_character() {
        let lines = break_lines("一二三四", 30.0, 10.0, &Square);
        assert_eq!(texts(&lines), ["一二三", "四"]);
    }

    #[test]
    fn a_mixed_line_breaks_at_the_word_boundary_not_inside_the_word() {
        // "中文" 逐字、"alpha" 整词：宽度只够 一 + 中 + alpha 的话，alpha 必须整体下来
        let lines = break_lines("一中alpha", 50.0, 10.0, &Square);
        assert_eq!(texts(&lines), ["一中", "alpha"]);
    }

    #[test]
    fn a_comma_never_starts_a_line_and_nothing_is_dropped() {
        let lines = break_lines("一二三四五六七八九，十", 30.0, 10.0, &Square);
        assert_eq!(texts(&lines), ["一二三", "四五六", "七八", "九，十"]);
        assert_eq!(joined(&lines), "一二三四五六七八九，十");
        for line in &lines {
            let head = line.text.chars().next().unwrap();
            assert!(!LINE_START_FORBIDDEN.contains(head), "行首出现了 {head}");
        }
    }

    #[test]
    fn an_opening_bracket_never_ends_a_line() {
        let lines = break_lines("甲乙丙（丁戊己", 30.0, 10.0, &Square);
        assert_eq!(texts(&lines), ["甲乙丙", "（丁戊", "己"]);
        for line in &lines {
            let tail = line.text.chars().next_back().unwrap();
            assert!(!LINE_END_FORBIDDEN.contains(tail), "行末出现了 {tail}");
        }
    }

    #[test]
    fn a_hard_break_from_the_source_rows_is_respected() {
        // 识别链把三行并成一块时留的是 `\n`：译文若带同样的换行，段落边界不许被贪心填行抹掉
        let lines = break_lines("甲乙\n丙丁", 400.0, 10.0, &Square);
        assert_eq!(texts(&lines), ["甲乙", "丙丁"]);
    }

    #[test]
    fn alignment_uses_the_first_line_and_the_center_tolerance() {
        let bbox = PixelRect::new(10.0, 0.0, 100.0, 20.0);
        assert_eq!(
            align_from_first_line(Some((40.0, 80.0)), bbox),
            Align::Center
        );
        assert_eq!(align_from_first_line(Some((10.0, 40.0)), bbox), Align::Left);
        assert_eq!(
            align_from_first_line(Some((75.0, 110.0)), bbox),
            Align::Right
        );
        assert_eq!(align_from_first_line(None, bbox), Align::Left);
        // 容差就是 0.15 × 块宽（这块中心在 60，容差 15）：首行中心偏 14 判居中，偏 16 判贴边
        assert_eq!(
            align_from_first_line(Some((47.0, 101.0)), bbox),
            Align::Center
        );
        assert_eq!(
            align_from_first_line(Some((47.0, 105.0)), bbox),
            Align::Right
        );
    }

    #[test]
    fn the_three_modes_decide_the_layer_and_the_user_overrides_the_background() {
        let short = "译文六个字";
        let cases = [
            (
                CompositeMode::Auto,
                BackgroundKind::Uniform,
                PlanKind::InPlace,
            ),
            (
                CompositeMode::Auto,
                BackgroundKind::Gradient,
                PlanKind::InPlace,
            ),
            (
                CompositeMode::Auto,
                BackgroundKind::Complex,
                PlanKind::Overlay,
            ),
            (
                CompositeMode::OverlayAll,
                BackgroundKind::Uniform,
                PlanKind::Overlay,
            ),
            (
                CompositeMode::ReplaceOnly,
                BackgroundKind::Complex,
                PlanKind::InPlace,
            ),
        ];
        for (mode, kind, expected) in cases {
            let plan = run(50.0, short, mode, kind);
            assert_eq!(plan.kind, expected, "{mode:?} + {kind:?}");
            // 放得下的块不该因为铺了底板就被叫作降级
            assert!(!plan.degraded, "{mode:?} + {kind:?} 被判成降级了");
            assert_eq!(plan.plate.is_some(), expected == PlanKind::Overlay);
        }
    }

    #[test]
    fn the_overlay_area_is_inset_and_the_plate_is_the_box() {
        let plan = run(
            50.0,
            "译文",
            CompositeMode::OverlayAll,
            BackgroundKind::Uniform,
        );
        assert_eq!(plan.area, PixelRect::new(3.0, 3.0, 394.0, 44.0));
        // 底板就是块框：这是 OverlayAll "块外逐字节不变"那条硬判据的前提
        assert_eq!(plan.lines.len(), 1);
    }

    /// 同一块框，只换首行墨迹的范围：对齐三档各走一遍时用
    fn aligned(first_line: InkSpan) -> BlockPlan {
        plan(
            &block(400.0, 50.0, Some("译文")),
            &estimate(BackgroundKind::Uniform),
            &Square,
            &CompositeOptions::default(),
            CompositeMode::Auto,
            Some(first_line),
        )
    }

    #[test]
    fn a_line_box_sits_where_the_plan_says_it_sits() {
        let left = aligned((0.0, 100.0)); // 首行只占左边一格 ⇒ 左对齐
        let height = Square.line_height(left.font_size);
        assert_eq!(left.align, Align::Left);
        assert_eq!(
            left.line_box(0, height).unwrap(),
            PixelRect::new(0.0, 0.0, 75.0, height)
        );
        assert_eq!(left.line_box(1, height), None, "只有一行就不许编出第二格");

        let centered = aligned((190.0, 210.0));
        assert_eq!(centered.align, Align::Center);
        assert_eq!(
            centered.line_box(0, height).unwrap().x,
            (400.0 - 75.0) / 2.0
        );
    }

    #[test]
    fn the_plate_moves_to_whichever_side_can_reach_four_half() {
        // 深底 + 更深的字：只能往亮处走（按"字比背景暗就加深背景"会死循环在纯黑上）
        let ink = Rgba([40, 40, 40, 255]);
        let plate = plate_color(Rgba([20, 20, 20, 255]), ink);
        assert!(contrast(&plate, &ink) >= MIN_CONTRAST, "往亮处走仍不达标");
        assert!(plate.0[0] > 20, "底板该变亮：{}", plate.0[0]);

        // 浅底 + 更浅的字：往黑走
        let ink = Rgba([200, 200, 200, 255]);
        let plate = plate_color(Rgba([210, 210, 210, 255]), ink);
        assert!(contrast(&plate, &ink) >= MIN_CONTRAST);
        assert!(plate.0[0] < 210, "底板该变暗：{}", plate.0[0]);

        // 已经达标就别乱动：黑底白字 21:1，往黑再逼近一格只是多余
        assert_eq!(plate_color(DARK, LIGHT), DARK);
    }

    #[test]
    fn the_plate_is_always_opaque() {
        let plate = plate_color(Rgba([200, 200, 200, 0]), Rgba([210, 210, 210, 0]));
        assert_eq!(plate.0[3], 255, "透明底 PNG 上铺的底板必须是不透明的");
    }

    #[test]
    fn the_ink_probe_reads_the_first_line_only() {
        // 这一条测的是 `bg::first_line_extent`（对齐推断唯一的像素入口）：只读擦除前的原图，
        // 且第二行再宽也不许影响首行的范围
        let mut image = image::RgbaImage::from_pixel(40, 20, DARK);
        for y in 2..5 {
            for x in 4..14 {
                image.put_pixel(x, y, LIGHT);
            }
        }
        for y in 9..12 {
            for x in 4..34 {
                image.put_pixel(x, y, LIGHT);
            }
        }
        let bbox = PixelRect::new(4.0, 2.0, 30.0, 11.0);
        assert_eq!(
            crate::bg::first_line_extent(&image, &bbox, DARK),
            Some((4.0, 14.0))
        );
        assert_eq!(
            align_from_first_line(crate::bg::first_line_extent(&image, &bbox, DARK), bbox),
            Align::Left
        );
        let blank = image::RgbaImage::from_pixel(10, 10, DARK);
        assert_eq!(
            crate::bg::first_line_extent(&blank, &PixelRect::new(0.0, 0.0, 10.0, 10.0), DARK),
            None
        );
    }
}
