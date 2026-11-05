//! REQ-010 **第一层**：可自动算的下限（CHG-006 T-011）。
//!
//! 跑法：`cargo run -p ice-vision --example compose_metric`
//! 另存自绘输入图：`... --save-dir /tmp/p7-inputs`（**只存输入，不存成品**——成品里是译文内容）
//!
//! 这个工具存在的理由只有一条：`assets/samples/` 是空的，四项人工评分（回填可读性 ≥4.0 /
//! 位置准确性 ≥3.5 / 观感自然度 ≥3.0 / 覆盖层可用率 ≥0.9）今天拿不到。但"能自动算的那一半"
//! 不必跟着一起等——**自绘图的真值是我造图时定下来的**（背景色、文字色、背景档、框的位置），
//! 所以这四个数有东西可比：
//!
//! 1. **三档判定对不对**：`bg::estimate` 给的档 vs 造图时的真档；
//! 2. **擦除后与真背景的色差**：`paint::erase` 单独跑一遍，逐像素与真背景比（`Complex` 档反向
//!    断言"一个像素都不许动"）；
//! 3. **回填越框面积**：`InPlace` 块各行的行盒落在块框**外面**多少平方像素；
//! 4. **`degraded` 出现率与漏标数**：越框了却没标降级的，就是漏标——FR-2.7 要的是"看得见"，
//!    漏一条就等于界面少画一圈橙描边；
//! 5. **一个字都不许多**：每块把 `plan.lines` 拼回去必须逐字等于那份译文（TC-2.7-a 那句
//!    "无文字被裁切丢失"的无头形状），**自动档与强制回填档各判一次**——后一档允许越框，
//!    但越出去的部分仍要完整在图上（IMG-13 那一格专门量它）。
//!
//! **它是下限测试，不是达标证据**（与 P6 的 `self_metric.rs` 同一条口径）：干净自绘图是这一类
//! 算法最容易的情形，全绿只说明"没在简单情形下算错"，说明不了"在你的截图上好看"。
//! 四个阈值（4.0 / 3.5 / 3.0 / 0.9）与识别侧的 0.92 一个字都不动，也**SHALL NOT** 拿这里的
//! 数字去冒充它们。
//!
//! 译文走下面的内置对照表，**一次网络都不碰**——所以同一命令两次运行的输出逐字节相同
//! （没有随机数：噪声那张图是按坐标算出来的函数；不报时间戳；耗时单独一行且不进判据）。

use std::process::ExitCode;

use ice_core::document::{OcrDocument, PixelRect, TextBlock};
use ice_vision::bg::{self, BackgroundKind};
use ice_vision::composite::{CompositeOptions, Renderer};
use ice_vision::paint::{self, FileMetrics};
use ice_vision::typeset::{self, Align, BlockPlan, CompositeMode, Metrics, PlanKind, PlannedLine};
use image::{ImageEncoder, Rgba, RgbaImage};

/// 一块样图的宽高。够排一行 24px 的字，也不至于让"照片噪声"那张图里样本太少。
const CANVAS_W: u32 = 460;
const CANVAS_H: u32 = 150;

/// 渐变那张图每 100px 走的灰度级数（见 `gradient` 上面那段）
const GRADIENT_LEVELS_PER_100PX: f32 = 8.0;

/// 造图用的字号，也是块框高度反推的依据（`estimated_font_size` 取 `框高 × 0.75`）
const INK_SIZE: f32 = 24.0;

/// `Uniform` 档擦完之后允许的通道误差。实心填一个已知色，超 2 就是写歪了而不是量偏了。
const TOLERANCE_UNIFORM: u64 = 2;
/// `Gradient` 档：擦除走四角双线性，而真值是沿 x 的线性渐变，两者在角点之外必有差。
/// 8 是"看得出来但不算补丁"的那条线，跑出来的实测值写在结论里。
const TOLERANCE_GRADIENT: u64 = 8;

/// 造一张自绘输入图。四个字段就是它的**真值**，下面的判据全部对着它们比。
struct Case {
    name: &'static str,
    /// 背景是坐标的函数（**不用随机数**，否则两次运行不可比）
    background: fn(f32, f32) -> [u8; 4],
    /// 文字色
    ink: [u8; 3],
    /// 块框内的原文（画进图里的那段字）
    text: &'static str,
    /// 内置对照表给它的"译文"——不经网络，所以这一步与真端点无关
    translation: &'static str,
    /// 块框
    bbox: PixelRect,
    /// 造图时的真背景档
    expect_kind: BackgroundKind,
    /// 期望的画法与是否降级
    expect_plan: PlanKind,
    expect_degraded: bool,
    /// 这一格在测什么（打出来给人看，也写进 docs/04 §5）
    probe: &'static str,
}

/// 纯色浅底 + 深字：最容易的一档，`bg` 该逐位等于那个浅灰
fn flat_light(_x: f32, _y: f32) -> [u8; 4] {
    [245, 245, 245, 255]
}

/// 纯色深底 + 浅字：FR-2.8 验收"回填后文字颜色仍为白"的那一族
fn flat_dark(_x: f32, _y: f32) -> [u8; 4] {
    [28, 30, 36, 255]
}

/// 水平渐变的斜率（每 100px 走多少级）。**这个数是量出来的不是拍的**：同一张图、同一套参数
/// 跑三档斜率，占比依次是 4 级 → 0.000（判 `Uniform`）、8 级 → 0.170（判 `Gradient`）、
/// 16 级 → 0.604（判 `Complex`）。取 8 是因为它是唯一真落进 `Gradient` 那一档的，
/// 而这格要测的就是"缓变渐变走四角双线性，而不是填一个均值色"。
fn ramp_per_100(x: f32, per: f32) -> [u8; 4] {
    let level = (250.0 - x / 100.0 * per).clamp(90.0, 250.0).round() as u8;
    [level, level, level, 255]
}

fn gradient(x: f32, _y: f32) -> [u8; 4] {
    ramp_per_100(x, GRADIENT_LEVELS_PER_100PX)
}

/// 棋盘格噪声：8px 一格交替两色，按坐标算所以可复现
fn noise(x: f32, y: f32) -> [u8; 4] {
    let cell = ((x / 8.0) as i64 + (y / 8.0) as i64) % 2 == 0;
    if cell {
        [236, 120, 60, 255]
    } else {
        [40, 74, 120, 255]
    }
}

/// 透明底：底色浅灰但 **alpha 只有 40**（透明 PNG 截图那一族）。采样带按 alpha 加权时，
/// 这些像素仍该投给同一个色——`bg` 的颜色分量不该被 alpha 拽向纯黑。
fn translucent(_x: f32, _y: f32) -> [u8; 4] {
    [238, 240, 244, 40]
}

fn cases() -> Vec<Case> {
    let wide = PixelRect::new(60.0, 60.0, 340.0, INK_SIZE / 0.75);
    vec![
        Case {
            name: "plain-light",
            background: flat_light,
            ink: [22, 24, 28],
            text: "Set up your account",
            translation: "设置你的账户",
            bbox: wide,
            expect_kind: BackgroundKind::Uniform,
            expect_plan: PlanKind::InPlace,
            expect_degraded: false,
            probe: "最容易的一档：bg 逐位等于真色、擦完仍是那个色",
        },
        Case {
            name: "plain-dark",
            background: flat_dark,
            ink: [240, 240, 240],
            text: "Continue",
            translation: "继续",
            bbox: PixelRect::new(60.0, 60.0, 200.0, INK_SIZE / 0.75),
            expect_kind: BackgroundKind::Uniform,
            expect_plan: PlanKind::InPlace,
            expect_degraded: false,
            probe: "深底浅字：fg 取出来必须还是浅色（FR-2.8 的可自动那半）",
        },
        Case {
            name: "gradient",
            background: gradient,
            ink: [30, 32, 36],
            text: "Export the report",
            translation: "导出报告",
            bbox: wide,
            expect_kind: BackgroundKind::Gradient,
            expect_plan: PlanKind::InPlace,
            expect_degraded: false,
            probe: "缓变渐变（8 级/100px）：判 Gradient，擦除走四角双线性而不是填一个均值色",
        },
        Case {
            name: "checker-noise",
            background: noise,
            ink: [250, 250, 250],
            text: "New message",
            translation: "新消息",
            bbox: wide,
            expect_kind: BackgroundKind::Complex,
            expect_plan: PlanKind::Overlay,
            expect_degraded: false,
            probe: "照片/噪声那一族：判 Complex、一个像素都不擦（ADR-003 的兜底）",
        },
        Case {
            name: "edge-glued",
            background: flat_light,
            ink: [22, 24, 28],
            text: "Zoom in",
            translation: "放大",
            // 贴左缘：采样带只剩右、上、下三面，兜底方向必须是"不擦"而不是"猜一个背景"
            bbox: PixelRect::new(0.0, 60.0, 180.0, INK_SIZE / 0.75),
            expect_kind: BackgroundKind::Uniform,
            expect_plan: PlanKind::InPlace,
            expect_degraded: false,
            probe: "贴边块：带变窄仍取得到样本，样本不足时宁可不擦",
        },
        Case {
            name: "alpha-background",
            background: translucent,
            ink: [20, 22, 26],
            text: "Save draft",
            translation: "保存草稿",
            bbox: PixelRect::new(60.0, 60.0, 220.0, INK_SIZE / 0.75),
            // alpha 加权之后仍判纯色：透明像素不该把众数拽向纯黑
            expect_kind: BackgroundKind::Uniform,
            expect_plan: PlanKind::InPlace,
            expect_degraded: false,
            probe: "透明底 PNG：bg 的颜色分量仍是那个浅灰，alpha 如实带出来",
        },
        Case {
            // Q-008 里那条构造样图：docs/06 的 IMG-13，已注明是造的不是截的
            name: "IMG-13-narrow-long-de",
            background: flat_light,
            ink: [22, 24, 28],
            text: "Delete",
            // 一句已知的德语长译文，故意塞进一个窄框：缩到 55% 下限也放不下
            translation: "Diesen Eintrag unwiderruflich aus der Liste entfernen",
            bbox: PixelRect::new(60.0, 60.0, 140.0, 18.0),
            expect_kind: BackgroundKind::Uniform,
            expect_plan: PlanKind::Overlay,
            expect_degraded: true,
            probe: "窄框 + 长译文：改判覆盖层并标 degraded，不许裁字（FR-2.7）",
        },
    ]
}

/// 只留非空白字符，按序拼回来。用来判"有没有被裁字"：空格在换行处被吞是设计如此，
/// 少一个字母就是缺陷。
fn letters(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

/// 同一块在「就地回填」档下的计划：那一档不许改建判覆盖层，允许越框，但**不许裁字**。
/// 越框面积同时给出来——docs/06 的 W-4.3 要的就是"越出去的字数与那圈橙描边对得上"，
/// 自动档改判覆盖层（越框 0）而回填档真越出去，这两个数一起看才知道降级链没走偏。
struct ReplaceOnly {
    kind: PlanKind,
    overflow: f32,
    joined: String,
}

fn plan_replace_only(
    canvas: &RgbaImage,
    case: &Case,
    metrics: &FileMetrics,
    options: &CompositeOptions,
) -> ReplaceOnly {
    let estimate = bg::estimate(canvas, &case.bbox, options);
    let first_line = bg::first_line_extent(canvas, &case.bbox, estimate.bg);
    let mut block = TextBlock::with_rows(0, case.text, case.bbox, 0.99, vec![0]);
    block.translation = Some(case.translation.to_string());
    let plan = typeset::plan(
        &block,
        &estimate,
        metrics,
        options,
        CompositeMode::ReplaceOnly,
        first_line,
    );
    let line_height = metrics.line_height(plan.font_size);
    let overflow: f32 = (0..plan.lines.len())
        .filter_map(|index| plan.line_box(index, line_height))
        .map(|box_line| area_outside(&box_line, &case.bbox))
        .sum();
    ReplaceOnly {
        kind: plan.kind,
        overflow,
        joined: plan
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<String>(),
    }
}

/// 按真值造一张图，并把原文用真字体画进块框里——**不拿识别链生成**，与 P6 同一条纪律
fn draw_input(case: &Case, metrics: &FileMetrics) -> RgbaImage {
    let mut canvas = RgbaImage::from_pixel(CANVAS_W, CANVAS_H, Rgba([0, 0, 0, 255]));
    for (x, y, pixel) in canvas.enumerate_pixels_mut() {
        *pixel = Rgba((case.background)(x as f32, y as f32));
    }

    let width = metrics.text_width(case.text, INK_SIZE);
    let plan = BlockPlan {
        kind: PlanKind::InPlace,
        font_size: INK_SIZE,
        lines: vec![PlannedLine {
            text: case.text.to_string(),
            width: width.max(case.bbox.w),
        }],
        align: Align::Left,
        degraded: false,
        text_color: Rgba([case.ink[0], case.ink[1], case.ink[2], 255]),
        plate: None,
        area: case.bbox,
    };
    // 只写字形、不擦不铺底板：这张图的角色是"输入"
    paint::draw_text(&mut canvas, &plan, metrics);
    canvas
}

/// 行盒落在块框外面的面积（平方像素）。各行的行盒在竖向不重叠，所以逐行相加不会重算。
fn area_outside(line: &PixelRect, frame: &PixelRect) -> f32 {
    let overlap_w = (line.right().min(frame.right()) - line.x.max(frame.x)).max(0.0);
    let overlap_h = (line.bottom().min(frame.bottom()) - line.y.max(frame.y)).max(0.0);
    (line.area() - overlap_w * overlap_h).max(0.0)
}

/// 擦完之后与真背景的通道最大差。`Complex` 档不该有差（它根本不擦），另有一条反向断言。
fn erase_drift(canvas: &RgbaImage, case: &Case) -> u64 {
    let options = CompositeOptions::default();
    // `estimate` 不返回错误：贴边就夹进图里继续，样本不足判 Complex（兜底方向永远是"不擦"）
    let estimate = bg::estimate(canvas, &case.bbox, &options);
    let mut copy = canvas.clone();
    paint::erase(&mut copy, &case.bbox, &estimate);

    let (x0, y0) = (case.bbox.x as u32, case.bbox.y as u32);
    let (x1, y1) = (
        (case.bbox.x + case.bbox.w) as u32,
        (case.bbox.y + case.bbox.h) as u32,
    );
    let mut worst = 0u64;
    for y in y0..y1.min(canvas.height()) {
        for x in x0..x1.min(canvas.width()) {
            let wanted = (case.background)(x as f32, y as f32);
            let got = copy.get_pixel(x, y).0;
            for channel in 0..3 {
                let delta = i64::from(got[channel]) - i64::from(wanted[channel]);
                worst = worst.max(delta.unsigned_abs());
            }
        }
    }
    worst
}

/// `Complex` 档的反向判据：擦除前后整张图必须逐字节相同
fn erase_touched_anything(canvas: &RgbaImage, case: &Case) -> bool {
    let options = CompositeOptions::default();
    let estimate = bg::estimate(canvas, &case.bbox, &options);
    let mut copy = canvas.clone();
    paint::erase(&mut copy, &case.bbox, &estimate);
    copy.as_raw() != canvas.as_raw()
}

fn main() -> ExitCode {
    let mut save_dir: Option<std::path::PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--save-dir" => match args.next() {
                Some(path) => save_dir = Some(std::path::PathBuf::from(path)),
                None => {
                    eprintln!("--save-dir 后面要一个目录");
                    return ExitCode::from(2);
                }
            },
            "--help" | "-h" => {
                println!(
                    "用法：cargo run -p ice-vision --example compose_metric [--save-dir 目录]"
                );
                println!("只造自绘输入图走 S5~S8 四个数；成品不落盘（里面是译文内容）。");
                println!("它是**下限测试**，不是四项人工评分的替代。");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("不认识的参数：{other}（--help 看用法）");
                return ExitCode::from(2);
            }
        }
    }

    // 读不到真字体就没有度量可言：字宽、行高、缺字形全部依赖它（REQ-005 停止条件 3）
    let metrics = match FileMetrics::open() {
        Ok(metrics) => metrics,
        Err(error) => {
            eprintln!("本机字体不可用，量不了：{error}");
            eprintln!(
                "这不是缺陷也不是回归——这一层要求黑体那本字体在位（`ice-core::font` 那个路径）。"
            );
            return ExitCode::from(1);
        }
    };

    let options = CompositeOptions::default();
    let renderer = Renderer::new(options);
    let mut problems: Vec<String> = Vec::new();
    let mut measured = 0usize;
    let mut degraded_count = 0usize;
    let mut drawable = 0usize;
    let mut rows = Vec::new();

    for case in cases() {
        let canvas = draw_input(&case, &metrics);
        if let Some(dir) = save_dir.as_ref() {
            if let Err(error) = dir_save(&canvas, dir, case.name) {
                eprintln!("存自绘输入图失败（{dir:?}）：{error}");
                return ExitCode::from(1);
            }
        }

        let estimate = bg::estimate(&canvas, &case.bbox, &options);
        let first_line = bg::first_line_extent(&canvas, &case.bbox, estimate.bg);
        let mut block = TextBlock::with_rows(0, case.text, case.bbox, 0.99, vec![0]);
        block.translation = Some(case.translation.to_string());
        let plan = typeset::plan(
            &block,
            &estimate,
            &metrics,
            &options,
            CompositeMode::Auto,
            first_line,
        );

        let mut document = OcrDocument::empty(CANVAS_W, CANVAS_H);
        document.blocks = vec![block];
        let composed = match renderer.compose(&canvas, &document, CompositeMode::Auto) {
            Ok(composed) => composed,
            Err(error) => {
                problems.push(format!("{}：合成失败 {error}", case.name));
                continue;
            }
        };

        measured += 1;
        drawable += 1;
        if composed.degraded() > 0 {
            degraded_count += 1;
        }

        // ① 三档判定
        let kind_ok = estimate.kind == case.expect_kind;
        // ② 擦除色差（Complex 档改判"一个像素都不许动"）
        let (erase_note, erase_ok) = if case.expect_kind == BackgroundKind::Complex {
            let untouched = !erase_touched_anything(&canvas, &case);
            ("像素未改".to_string(), untouched)
        } else {
            let drift = erase_drift(&canvas, &case);
            let tolerance = if case.expect_kind == BackgroundKind::Uniform {
                TOLERANCE_UNIFORM
            } else {
                TOLERANCE_GRADIENT
            };
            (
                format!("最大差 {drift}（限 {tolerance}）"),
                drift <= tolerance,
            )
        };
        // ③ 越框面积（只对实际回填的块有意义）
        let overflow: f32 = if plan.kind == PlanKind::InPlace {
            let line_height = metrics.line_height(plan.font_size);
            (0..plan.lines.len())
                .filter_map(|index| plan.line_box(index, line_height))
                .map(|box_line| area_outside(&box_line, &case.bbox))
                .sum()
        } else {
            0.0
        };
        // ④ 降级：该标的要标，越了框又不标就是漏标
        let degraded = plan.degraded;
        let mislabeled = overflow > 0.0 && !degraded && plan.kind == PlanKind::InPlace;

        // 字一个都不许多、一个都不许少：两种画法各拼一次。
        // 比的是**非空白字符逐字等**——换行处吞掉的那个空格是排版正确性的一部分
        // （行尾留一个悬挂空格反而会把行宽算错，右侧对齐时还会溢出一格），
        // 而"少一个字母"这类真裁剪在这条比对下仍然当场现形。
        let joined = plan
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<String>();
        let kept_every_char = letters(&joined) == letters(case.translation);
        let replace_only = plan_replace_only(&canvas, &case, &metrics, &options);

        for (what, held) in [
            ("三档判定", kind_ok),
            ("自动档不裁字", kept_every_char),
            (
                "强制回填档不裁字",
                letters(&replace_only.joined) == letters(case.translation),
            ),
            (
                "强制回填档不改建判覆盖层",
                replace_only.kind == PlanKind::InPlace,
            ),
            ("画法与期望一致", plan.kind == case.expect_plan),
            ("降级标记", degraded == case.expect_degraded),
            ("擦除那一档", erase_ok),
            ("无漏标", !mislabeled),
        ] {
            if !held {
                let (name, probe) = (case.name, case.probe);
                let (want_plan, want_degraded) = (case.expect_plan, case.expect_degraded);
                problems.push(format!(
                    "{name}：{what} 不符（期望 {want_plan:?}/degraded={want_degraded}，\
                     实得 {:?}/degraded={degraded}，越框 {overflow:.1}px²，{erase_note}）——这一格测的是：{probe}",
                    plan.kind
                ));
            }
        }

        rows.push(format!(
            "{:<22} 档={:<8} 占比={:.3} 画法={:<7} degraded={:<5} 擦除={} 越框={:>6.1}px² 强制回填越框={:>7.1}px² 缺字={} {}",
            case.name,
            kind_name(estimate.kind),
            estimate.variance,
            plan_name(plan.kind),
            degraded,
            erase_note,
            overflow,
            replace_only.overflow,
            composed
                .missing_glyphs
                .iter()
                .map(|(_, count)| count)
                .sum::<usize>(),
            case.probe
        ));
    }

    for row in &rows {
        println!("{row}");
    }
    println!();
    println!(
        "自绘样图 {measured} 张：degraded 出现率 = {}/{}（IMG-13 那一格是专打降级链的构造样图，不是随机抽样）",
        degraded_count, drawable
    );
    println!(
        "下限判据：三档判定、擦除色差、越框面积、降级不漏标。**不产出四项人工评分，也不替代它们**。"
    );

    if problems.is_empty() {
        println!("✓ 第一层四项全过（这只是干净自绘图上的下限）");
        return ExitCode::SUCCESS;
    }
    println!("✗ 第一层有 {} 条不符：", problems.len());
    for problem in &problems {
        println!("  - {problem}");
    }
    ExitCode::from(1)
}

fn kind_name(kind: BackgroundKind) -> &'static str {
    match kind {
        BackgroundKind::Uniform => "Uniform",
        BackgroundKind::Gradient => "Gradient",
        BackgroundKind::Complex => "Complex",
    }
}

fn plan_name(kind: PlanKind) -> &'static str {
    match kind {
        PlanKind::InPlace => "InPlace",
        PlanKind::Overlay => "Overlay",
        PlanKind::Skip => "Skip",
    }
}

/// 只存**输入图**。成品里有译文内容，按隐私口径不落在这一条路径上。
fn dir_save(canvas: &RgbaImage, dir: &std::path::Path, name: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut buffer = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buffer)
        .write_image(
            canvas.as_raw(),
            canvas.width(),
            canvas.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.png")), buffer).map_err(|e| e.to_string())
}
