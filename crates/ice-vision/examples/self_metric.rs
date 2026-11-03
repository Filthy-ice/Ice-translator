//! REQ-008 第一层：程序自绘合成图 → 本机识别 → 逐图字符正确率（CHG-005 T-009）。
//!
//! 跑法：`cargo run -p ice-vision --example self_metric -- --repeat 3`
//!
//! 这一层为什么**能**自动算：标注不是照着图敲出来的，是构造出来的——文本、字号、每个字落在
//! 哪个像素全在我手里。于是这条链证明的是"链路通、坐标对、合并没串、不 panic"，
//! 它证明不了真实截图（艺术字、低对比度、照片背景、说明书小字）的质量。
//! docs/06 那条 `≥0.92` 的判据一个数字都不动，它属第二层（`samples_run.rs` + 用户放的图）。
//!
//! 默认输出只有计数与比率：识别到的文本一个字都不打印、不落盘（NFR-3 在图片侧的口径）。
//! `--verbose` 是唯一的例外，它把标注与识别串一起打出来用于排障——两边的文本都是本程序自己写的，
//! 不涉及任何用户内容，所以才敢打。
//!
//! 非 macOS 上这个可执行文件照样编译（`recognize_with_rows` 两条编译面都在），运行时直接说明
//! 本机没有识别能力、以非零退出码结束——不产出任何看起来像达标结论的数字。

use std::process::ExitCode;

use ab_glyph_rasterizer::{Rasterizer, point};
use ice_vision::import::admit_rgba;
use ice_vision::ocr::recognize_with_rows;
use image::{DynamicImage, Rgba, RgbaImage};
use owned_ttf_parser::{Face, OutlineBuilder};

/// 与界面用的是同一支字体（`ice-app::fonts`），这样"画字的路径"和用户屏幕上看到的是同一条。
const FONT_PATH: &str = "/System/Library/Fonts/STHeiti Medium.ttc";
/// 第 1 面 = `STHeitiSC-Medium`（黑体-简），第 0 面是繁体。
///
/// P5 踩过的坑：选错面既不崩也不缺字，只有肉眼看得出差别——所以这里显式给索引，不写 0 碰运气。
const HEITI_SC_FACE: u32 = 1;
/// 字号固定 28px：度量要的是"同一台机器、同一档字号"的可比数字，字号不该是变量。
const PIXEL_SIZE: f32 = 28.0;
/// 画布四周留白：Vision 对贴边的字会掉置信度，留白是给识别留的，不是给好看留的。
const MARGIN: f32 = 28.0;

/// 一段文字落在画布上的位置（`baseline` 是基线的 y，字形往上到 ascender、往下到 descender）。
struct Run {
    text: String,
    x: f32,
    baseline: f32,
}

/// 一个用例：名字 + 它的绘制清单。标注由 `runs` 现算，不手写第二份，否则改动会不同步。
struct Sample {
    name: &'static str,
    /// 这一用例在测什么（打出来，省得对着名字猜）
    purpose: &'static str,
    runs: Vec<Run>,
}

/// 一张自绘图的字形度量：画布尺寸 + 把字体坐标搬到像素坐标所需的偏移。
struct Sheet {
    width: u32,
    height: u32,
    /// 像素 y = baseline − 字形y × scale + `origin_y`
    origin_y: f32,
}

fn main() -> ExitCode {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(message) => {
            eprintln!(
                "{message}\n用法：cargo run -p ice-vision --example self_metric -- [--repeat N] [--save-dir DIR] [--verbose]"
            );
            return ExitCode::from(2);
        }
    };

    if !ice_vision::native_ocr_supported() {
        println!(
            "当前平台没有本地文字识别（REQ-007 的降级面）：自动度量在此无意义，不产出任何数字。"
        );
        return ExitCode::from(1);
    }

    let font_bytes = match std::fs::read(FONT_PATH) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("读不到度量用字体 {FONT_PATH}：{err}");
            return ExitCode::from(2);
        }
    };
    let face = match Face::parse(&font_bytes, HEITI_SC_FACE) {
        Ok(face) => face,
        Err(err) => {
            eprintln!("字体第 {HEITI_SC_FACE} 面解析失败：{err}");
            return ExitCode::from(2);
        }
    };

    let samples = samples(&face);
    let mut observations: Vec<Observation> = Vec::new();
    let mut missing_glyphs = 0usize;
    for round in 0..args.repeat {
        for sample in &samples {
            let image = render(&face, &sample.runs);
            if let Some(dir) = &args.save_dir {
                let path = dir.join(format!("{round:02}-{}.png", sample.name));
                if let Err(err) = DynamicImage::ImageRgba8(image.clone()).save(&path) {
                    eprintln!("存不进 {}：{err}", path.display());
                }
            }
            missing_glyphs += count_missing(&face, &sample.runs);
            match recognize_one(&image) {
                Ok(outcome) => {
                    let accuracy = char_accuracy(&label_of(&sample.runs), &outcome.text);
                    observations.push(Observation {
                        name: sample.name,
                        rows: outcome.rows,
                        blocks: outcome.blocks,
                        elapsed_ms: outcome.elapsed_ms,
                        accuracy,
                    });
                    if args.verbose {
                        println!(
                            "[{round}] {} rows={} blocks={} elapsed_ms={} accuracy={:.4}\n    label={}\n    ocr={}",
                            sample.name,
                            outcome.rows,
                            outcome.blocks,
                            outcome.elapsed_ms,
                            accuracy,
                            label_of(&sample.runs),
                            outcome.text
                        );
                    }
                }
                Err(message) => {
                    eprintln!("第 {round} 轮的 {} 识别失败：{message}", sample.name);
                    return ExitCode::from(1);
                }
            }
        }
    }

    for sample in &samples {
        let mine: Vec<&Observation> = observations
            .iter()
            .filter(|o| o.name == sample.name)
            .collect();
        println!(
            "{} ({}): rows={} blocks={} accuracy={:.4} elapsed_ms={}",
            sample.name,
            sample.purpose,
            average(mine.iter().map(|o| o.rows)),
            average(mine.iter().map(|o| o.blocks)),
            mean(mine.iter().map(|o| o.accuracy)),
            average(mine.iter().map(|o| o.elapsed_ms as usize)),
        );
    }
    summarize(&observations, missing_glyphs, &samples);
    ExitCode::SUCCESS
}

/// 平均编辑距离之外的总口径：全部用例的平均与最低字符正确率。
///
/// 最后那句不是客套，是这条度量的边界声明——自动数字再高也不等于 FR-2.2 达标。
fn summarize(observations: &[Observation], missing_glyphs: usize, samples: &[Sample]) {
    let accuracies: Vec<f64> = observations.iter().map(|o| o.accuracy).collect();
    if accuracies.is_empty() {
        return;
    }
    let min = accuracies.iter().copied().fold(f64::INFINITY, f64::min);
    println!();
    println!(
        "用例 {} 类 × {} 轮，共 {} 张自绘图",
        samples.len(),
        observations.len() / samples.len().max(1),
        observations.len()
    );
    println!(
        "字符正确率：平均 {:.4}，最低 {min:.4}",
        mean(accuracies.iter().copied())
    );
    if missing_glyphs > 0 {
        println!(
            "注意：有 {missing_glyphs} 个字符在 {FONT_PATH} 里找不到字形——标注里出现了画不出来的字，上面这些数字要打折看。"
        );
    }
    println!("这条自动度量是**下限测试**：程序自绘的干净文字是 Vision 最容易的一类。");
    println!(
        "它不替代 docs/06 的 IMG-1~IMG-12 与 ≥0.92 判据（那一层要真实截图 + 人工标注，见 samples_run.rs）。"
    );
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let mut count = 0usize;
    let mut sum = 0.0f64;
    for value in values {
        count += 1;
        sum += value;
    }
    if count == 0 { 0.0 } else { sum / count as f64 }
}

fn average(values: impl Iterator<Item = usize>) -> usize {
    let mut count = 0usize;
    let mut sum = 0usize;
    for value in values {
        count += 1;
        sum += value;
    }
    sum.checked_div(count).unwrap_or(0)
}

fn samples(face: &Face) -> Vec<Sample> {
    // 三类"干净文字"：行距给到 2.2 倍字号，行与行明显是独立段
    let wide = PIXEL_SIZE * 2.2;
    // 段落那类：行距压到 1.25 倍，五行本该被 REQ-004 的判据接成块——这一条测的就是合并
    let tight = PIXEL_SIZE * 1.25;

    vec![
        Sample {
            name: "en_plain",
            purpose: "纯英文行",
            runs: lines(
                face,
                &[
                    "The quick brown fox jumps over the lazy dog",
                    "Ice Translator reads the picture on this machine",
                    "Please confirm the key before continuing",
                ],
                wide,
            ),
        },
        Sample {
            name: "zh_plain",
            purpose: "纯中文行",
            runs: lines(
                face,
                &[
                    "请在设置里填写接口地址",
                    "本机识别不上传任何图片",
                    "翻译结果可以逐块编辑",
                ],
                wide,
            ),
        },
        Sample {
            name: "mixed",
            purpose: "中英混排",
            runs: lines(
                face,
                &[
                    "打开 Settings 后粘贴 API Key",
                    "第 3 步：点击 Translate 按钮",
                    "字号 14px 适合正文内容",
                ],
                wide,
            ),
        },
        Sample {
            name: "paragraph",
            purpose: "多行同段落，考 REQ-004 的合并",
            runs: lines(
                face,
                &[
                    "本报告比较了三种识别方案的准确率，",
                    "并在同一台机器上重复运行了十次，",
                    "结果显示本地方案在耗时上更稳定，",
                    "但样本量仍然偏小，需要继续验证。",
                ],
                tight,
            ),
        },
        Sample {
            name: "table",
            purpose: "两列表格，考表格防护（同行不同列不许并成一块）",
            runs: table(
                face,
                &[
                    ("引擎", "字符正确率"),
                    ("Vision", "0.94"),
                    ("DeepL", "0.97"),
                    ("本机方案", "0.91"),
                ],
                wide,
                PIXEL_SIZE * 3.0,
            ),
        },
    ]
}

/// 一列竖排的行，左对齐到同一条基线 x。
fn lines(face: &Face, texts: &[&str], gap: f32) -> Vec<Run> {
    let baseline_first = MARGIN + ascender_px(face);
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| Run {
            text: (*text).to_string(),
            x: MARGIN,
            baseline: baseline_first + gap * index as f32,
        })
        .collect()
}

/// 两列表格：第二列的起点按最宽的第一列对齐，中间是 `gutter` 宽的栏距。
fn table(face: &Face, rows: &[(&str, &str)], gap: f32, gutter: f32) -> Vec<Run> {
    let baseline_first = MARGIN + ascender_px(face);
    let left_widest = rows
        .iter()
        .map(|(left, _)| text_width(face, left))
        .fold(0.0f32, f32::max);
    let right_x = MARGIN + left_widest + gutter;
    let mut runs = Vec::with_capacity(rows.len() * 2);
    for (index, (left, right)) in rows.iter().enumerate() {
        let baseline = baseline_first + gap * index as f32;
        runs.push(Run {
            text: (*left).to_string(),
            x: MARGIN,
            baseline,
        });
        runs.push(Run {
            text: (*right).to_string(),
            x: right_x,
            baseline,
        });
    }
    runs
}

fn ascender_px(face: &Face) -> f32 {
    face.ascender() as f32 * scale(face)
}

fn scale(face: &Face) -> f32 {
    PIXEL_SIZE / face.units_per_em() as f32
}

fn text_width(face: &Face, text: &str) -> f32 {
    text.chars().map(|ch| advance_px(face, ch)).sum()
}

fn advance_px(face: &Face, ch: char) -> f32 {
    face.glyph_index(ch)
        .and_then(|glyph| face.glyph_hor_advance(glyph))
        .unwrap_or(0) as f32
        * scale(face)
}

/// 标注文本按绘制顺序拼接——表格那一类因此天然带着"左栏在前"的顺序，与识别的确定性排序同向。
fn label_of(runs: &[Run]) -> String {
    normalize(&runs.iter().map(|run| run.text.as_str()).collect::<String>())
}

/// 只去空白：其余（大小写、标点全半角）都算进正确率，不做"看起来更好"的粉饰。
fn normalize(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn count_missing(face: &Face, runs: &[Run]) -> usize {
    runs.iter()
        .flat_map(|run| run.text.chars())
        .filter(|ch| face.glyph_index(*ch).is_none())
        .count()
}

/// 画布尺寸由内容算出来：贴边会让 Vision 掉置信度，超出会切掉字——两头都得避开。
fn layout(face: &Face, runs: &[Run]) -> Sheet {
    let s = scale(face);
    let mut right = MARGIN;
    let mut lowest = MARGIN;
    let mut highest = f32::INFINITY;
    for run in runs {
        right = right.max(run.x + text_width(face, &run.text));
        highest = highest.min(run.baseline - face.ascender() as f32 * s);
        lowest = lowest.max(run.baseline - face.descender() as f32 * s);
    }
    Sheet {
        width: right.ceil() as u32 + MARGIN as u32,
        height: (lowest - highest).ceil() as u32 + 2 * MARGIN as u32,
        origin_y: MARGIN - highest,
    }
}

/// 字形轮廓 → RGBA 位图（黑字白底）。
///
/// 一趟 `Rasterizer` 装整张图：笔画不重叠时逐字建栅格器反而更容易看出坐标错，
/// 而这一层要测的东西里不包括"字与字重叠怎么办"。
fn render(face: &Face, runs: &[Run]) -> RgbaImage {
    let sheet = layout(face, runs);
    let mut raster = Rasterizer::new(sheet.width as usize, sheet.height as usize);
    let s = scale(face);
    for run in runs {
        let mut pen_x = run.x;
        for ch in run.text.chars() {
            let Some(glyph) = face.glyph_index(ch) else {
                pen_x += advance_px(face, ch);
                continue;
            };
            let mut builder = ToPixels {
                raster: &mut raster,
                scale: s,
                pen_x,
                baseline: run.baseline + sheet.origin_y,
                cursor: (0.0, 0.0),
                start: (0.0, 0.0),
            };
            face.outline_glyph(glyph, &mut builder);
            pen_x += advance_px(face, ch);
        }
    }

    let mut image = RgbaImage::from_pixel(sheet.width, sheet.height, Rgba([255, 255, 255, 255]));
    raster.for_each_pixel_2d(|x, y, coverage| {
        let shade = (255.0 * (1.0 - coverage.clamp(0.0, 1.0))) as u8;
        if (x, y) < (sheet.width, sheet.height) {
            image.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
        }
    });
    image
}

/// `ttf_parser` 给的是字体单位、y 轴向上；画布是像素、y 轴向下。这一层只做这一个仿射。
struct ToPixels<'r> {
    raster: &'r mut Rasterizer,
    scale: f32,
    pen_x: f32,
    baseline: f32,
    cursor: (f32, f32),
    start: (f32, f32),
}

impl ToPixels<'_> {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.pen_x + x * self.scale, self.baseline - y * self.scale)
    }
}

impl OutlineBuilder for ToPixels<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = self.at(x, y);
        self.cursor = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.at(x, y);
        self.raster
            .draw_line(point(self.cursor.0, self.cursor.1), point(to.0, to.1));
        self.cursor = to;
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let control = self.at(x1, y1);
        let to = self.at(x, y);
        self.raster.draw_quad(
            point(self.cursor.0, self.cursor.1),
            point(control.0, control.1),
            point(to.0, to.1),
        );
        self.cursor = to;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let c1 = self.at(x1, y1);
        let c2 = self.at(x2, y2);
        let to = self.at(x, y);
        self.raster.draw_cubic(
            point(self.cursor.0, self.cursor.1),
            point(c1.0, c1.1),
            point(c2.0, c2.1),
            point(to.0, to.1),
        );
        self.cursor = to;
    }

    fn close(&mut self) {
        self.raster.draw_line(
            point(self.cursor.0, self.cursor.1),
            point(self.start.0, self.start.1),
        );
        self.cursor = self.start;
    }
}

/// 一张自绘图跑完整条识别链（S0 入场 → S1~S3），只带回计数与拼好的识别串。
fn recognize_one(image: &RgbaImage) -> Result<Outcome, String> {
    let admitted = admit_rgba(image.clone(), "self_metric");
    let (rows, document, progress) =
        recognize_with_rows(&admitted.image).map_err(|err| err.to_string())?;
    let text = normalize(
        &document
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<String>(),
    );
    Ok(Outcome {
        rows: rows.len(),
        blocks: progress.blocks,
        elapsed_ms: progress.elapsed_ms,
        text,
    })
}

struct Outcome {
    rows: usize,
    blocks: usize,
    elapsed_ms: u64,
    text: String,
}

struct Observation {
    name: &'static str,
    rows: usize,
    blocks: usize,
    elapsed_ms: u64,
    accuracy: f64,
}

/// `1 − 编辑距离 / 标注长度`（docs/04 §5 那条口径），下限夹到 0。
fn char_accuracy(label: &str, recognized: &str) -> f64 {
    let left: Vec<char> = label.chars().collect();
    let right: Vec<char> = recognized.chars().collect();
    if left.is_empty() {
        return if right.is_empty() { 1.0 } else { 0.0 };
    }
    let distance = edit_distance(&left, &right);
    (1.0 - distance as f64 / left.len() as f64).max(0.0)
}

/// 只存两行的 Wagner–Fischer：标注最长也就两百来字，但这不是把它写成 O(n·m) 内存的理由。
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

struct Args {
    repeat: usize,
    save_dir: Option<std::path::PathBuf>,
    verbose: bool,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Args {
            repeat: 1,
            save_dir: None,
            verbose: false,
        };
        let mut iter = std::env::args().skip(1);
        while let Some(flag) = iter.next() {
            match flag.as_str() {
                "--verbose" => args.verbose = true,
                "--repeat" => {
                    let value = iter.next().ok_or("--repeat 后面要一个正整数")?;
                    args.repeat = value
                        .parse::<usize>()
                        .map_err(|_| format!("--repeat 的值不是正整数：{value}"))?
                        .max(1);
                }
                "--save-dir" => {
                    let dir = iter.next().ok_or("--save-dir 后面要一个目录")?;
                    let path = std::path::PathBuf::from(dir);
                    std::fs::create_dir_all(&path)
                        .map_err(|err| format!("建不出 {}：{err}", path.display()))?;
                    args.save_dir = Some(path);
                }
                other => return Err(format!("不认识的参数：{other}")),
            }
        }
        Ok(args)
    }
}
