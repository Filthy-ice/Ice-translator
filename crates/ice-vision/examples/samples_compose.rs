//! REQ-010 **第二层**的取数工具：逐张跑 `assets/samples/` 里的真图，走完整条回填链，
//! 结果落 CSV（CHG-006 T-011）。
//!
//! 跑法：`cargo run -p ice-vision --example samples_compose -- [目录] [--csv 输出.csv]`
//!
//! 它和 `compose_metric.rs` 是同一层判据的两半：那张图是**自绘的、真值已知**，所以能算"对不对"；
//! 这一张吃的是**真截图**（照片背景、渐变按钮、密排小字那些自绘图造不出来的东西），
//! 所以能算"在真实像素上发生了什么"。两边都**不产评分**——回填可读性 ≥4.0、位置准确性 ≥3.5、
//! 观感自然度 ≥3.0、覆盖层可用率 ≥0.9 这四项由人给（`docs/04` §5 那张表），本工具一个字都不代填。
//!
//! **译文这一步走的是"原文照抄"，一次网络都不碰**。理由有两条：
//! ① 可复现——MyMemory 那条端点是 IP 级拉黑的（P3 量过：不限速连发到第三批 20/20 全拒），
//!    把逐张翻译塞进取证脚本，同一目录第二次跑就可能给出另一组数，那不再是取证而是抽奖；
//! ② 这一层要量的是 **S5~S8 在真图像素上的行为**（判了哪一档、擦没擦、几块走了覆盖层、
//!    几块放不下），这些与译文是哪门语言无关。
//! 真实译文的观感（长译文挤不挤得下、覆盖层好不好看）请在应用里走：图片页「翻译全部」→「生成图片」，
//! 那是用户主动触发的那一次外发（本包隐私口径：图片字节与像素全程不出网，识别出的文本经 P3 通道
//! 送出去是点「翻译全部」那次主动触发的正常行为，SHALL NOT 写成"整条图片链零外发"）。
//!
//! 隐私：CSV 只有五列——文件名、块数、回填数、覆盖层数、降级数、耗时。
//! **识别出的文本、译文、图片字节一个字都不写**（NFR-3 图片侧同一口径）。
//! 用户的原件**只读**：不改、不删、不在旁边写任何东西；成品只在标准输出/CSV 里以数字形式出现。

use std::process::ExitCode;

use ice_vision::composite::{CompositeOptions, Renderer};
use ice_vision::error::VisionError;
use ice_vision::import::admit_path;
use ice_vision::ocr::recognize_with_rows;
use ice_vision::typeset::CompositeMode;

/// 与 `samples_run.rs` 同一份白名单：本版本解码支持的格式就是 `image` 那组 feature
const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// CSV 的表头——**列的集合是 spec 写死的**，多一列就是多泄一类内容
const CSV_HEADER: &str = "filename,blocks,replaced,overlaid,degraded,elapsed_ms";

fn main() -> ExitCode {
    let mut dir = std::path::PathBuf::from("assets/samples");
    let mut csv: Option<std::path::PathBuf> = None;
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--csv" => match iter.next() {
                Some(path) => csv = Some(std::path::PathBuf::from(path)),
                None => {
                    eprintln!("--csv 后面要一个文件路径");
                    return ExitCode::from(2);
                }
            },
            "--help" | "-h" => {
                println!(
                    "用法：cargo run -p ice-vision --example samples_compose -- [目录] [--csv 输出.csv]"
                );
                println!("默认目录 assets/samples；CSV 默认打到标准输出。五列：{CSV_HEADER}");
                println!("译文取原文照抄（不联网、可复现）。四项人工评分不在这里产出。");
                return ExitCode::SUCCESS;
            }
            other => {
                let path = std::path::PathBuf::from(other);
                if !path.is_dir() {
                    eprintln!("目录不存在：{}", path.display());
                    return ExitCode::from(2);
                }
                dir = path;
            }
        }
    }

    if !ice_vision::native_ocr_supported() {
        println!("当前平台没有本地文字识别（REQ-007 的降级面）：不产出任何数字。");
        return ExitCode::from(1);
    }

    let candidates = collect_images(&dir);
    let mut rows = vec![CSV_HEADER.to_string()];
    let mut measured = 0usize;
    let mut quiet = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let started = std::time::Instant::now();

    for path in &candidates {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match run_one(path) {
            Ok((columns, no_text)) => {
                rows.push(format!("{name},{}", columns.join(",")));
                measured += 1;
                if no_text {
                    quiet += 1;
                }
            }
            // 一张图失败只记在它自己头上（与 REQ-001 那块失败隔离同一条纪律）：这一行
            // 四个计数写 0、耗时写 `-`（不假装量到过），类别另走 stderr 且只写类别不写路径
            Err(kind) => {
                rows.push(format!("{name},0,0,0,0,-"));
                failures.push(format!("{name}: {kind}"));
            }
        }
    }

    let text = rows.join("\n");
    match csv {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, format!("{text}\n")) {
                eprintln!("写 CSV 失败：{error}");
                return ExitCode::from(1);
            }
        }
        None => println!("{text}"),
    }

    // 失败原因先说，**再**判"样本不足"：上一版在这里直接早退，目录里明明有一张图、
    // 却因为把"没有文字"当失败而报成"样本不足"，把真话盖住了（2026-11-05 用户第一次放真图就撞上）
    for failure in &failures {
        eprintln!("  失败：{failure}");
    }

    if candidates.is_empty() {
        println!();
        println!(
            "样本不足：`{}` 里没有 png/jpg/jpeg 候选文件。",
            dir.display()
        );
        println!(
            "FR-2.4~2.8 的四项人工评分因此记**未闭环**——自绘那一层（`compose_metric.rs`）过没过都不改变这一点。"
        );
        return ExitCode::SUCCESS;
    }

    if measured == 0 {
        println!();
        println!(
            "有 {} 张候选图但一张都没跑通（失败类别见上面 stderr）。这**不是**样本不足，是取数失败——按未闭环记，别当成没有图。",
            candidates.len()
        );
        return ExitCode::from(1);
    }

    println!();
    println!(
        "实测 {measured} 张（其中 {quiet} 张整张没识别出文字——照片背景那类样张本来就可能没字，那是覆盖层兜底要用的对象，不是失败），失败 {} 张，总耗时 {} ms。",
        failures.len(),
        started.elapsed().as_millis()
    );
    println!(
        "这些是**计数**，不是评分。四项人工评分（4.0 / 3.5 / 3.0 / 0.9）由人给，见 `docs/04` §5 与 `docs/evidence/p7-scoring.md`。"
    );
    ExitCode::SUCCESS
}

/// 目录里的候选图，按文件名排序——**同一目录两次运行必须同一顺序同一组数**
fn collect_images(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        })
        .collect();
    found.sort();
    found
}

/// 一张真图走完整链，返回 CSV 的那几个数与"这张图整张没字"的那个旗标。
/// **只读**这张图，不写任何副作用。
///
/// `NoTextFound` **不是失败**：照片背景、海报、艺术字那几类样张本来就可能一个字都认不出来，
/// 界面上它走的是既有的"未发现文字"那一档（CHG-005 的 `RecognitionStage::NoText`）。
/// 上一版把它当失败，于是"目录里第一张真图"被报成取数失败——照片类样张恰恰是覆盖层兜底
/// 要考察的对象，把它排除掉等于把最想看的那一类扔了。
fn run_one(path: &std::path::Path) -> Result<(Vec<String>, bool), &'static str> {
    let started = std::time::Instant::now();
    // 入场：解码 + 超上限缩放（`admit_path` 自己就这条路）
    let admitted = admit_path(path).map_err(failure_kind)?;
    // 识别：本地 Vision，整段在这条线程上（行观测只用来看块数，不进 CSV）
    let mut document = match recognize_with_rows(&admitted.image) {
        Ok((_, document, _)) => document,
        Err(VisionError::NoTextFound) => {
            return Ok((
                vec![
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    started.elapsed().as_millis().to_string(),
                ],
                true,
            ));
        }
        Err(error) => return Err(failure_kind(error)),
    };
    // 译文=原文：不联网那一条理由写在文件头
    for block in &mut document.blocks {
        block.translation = Some(block.text.clone());
    }
    let composed = Renderer::new(CompositeOptions::default())
        .compose(&admitted.image, &document, CompositeMode::Auto)
        .map_err(|_| "compose")?;

    Ok((
        vec![
            document.enabled_blocks().count().to_string(),
            composed.replaced.to_string(),
            composed.overlaid.to_string(),
            composed.degraded().to_string(),
            started.elapsed().as_millis().to_string(),
        ],
        false,
    ))
}

/// 失败只写**类别**：`VisionError` 的 Display 里带着 OS 给的原文或图片尺寸，
/// 那些拼进 CSV 就等于把用户内容写到文件里（NFR-3）
fn failure_kind(error: VisionError) -> &'static str {
    match error {
        VisionError::UnsupportedPlatform => "platform",
        VisionError::Decode(_) => "decode",
        VisionError::TooLarge(_, _) => "too-large",
        VisionError::Recognition(_) => "ocr",
        VisionError::NoTextFound => "no-text",
        VisionError::Composite(_) => "compose",
        VisionError::Font(_) => "font",
        _ => "other",
    }
}
