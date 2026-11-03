//! REQ-008 第二层的工具：逐张跑一个目录里的真实样图，结果落 CSV（CHG-005 T-009）。
//!
//! 跑法：`cargo run -p ice-vision --example samples_run -- assets/samples --csv /tmp/ocr-metrics.csv`
//!
//! 这个工具存在的理由很窄：`assets/samples/` 是空的，IMG-1~IMG-12 那十二张只能来自这台机器
//! （报错弹窗、PPT、说明书、深色截图），我不能凭空造（spec 事实表 FACT 那条）。
//! 所以本包交付"放图 + 一条命令出数"，标注与 ≥0.92 的定论留给用户——
//! **目录空的时候这里不产出任何看起来像结论的数字**，按停止条件 6 记"未闭环"。
//!
//! CSV 只有四列：文件名、行数、块数、耗时。**识别出的文本一个字都不写**——
//! 与日志的隐私口径同形（NFR-3 图片侧：只允许 rows / blocks / elapsed_ms / 尺寸 / 错误码）。

use std::process::ExitCode;

use ice_vision::import::admit_path;
use ice_vision::ocr::recognize_with_rows;

/// 本版本解码支持的格式与 `ice-vision` 的 `image` feature 一致（png / jpeg），大小写都收。
const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

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
                    "用法：cargo run -p ice-vision --example samples_run -- [目录] [--csv 输出.csv]"
                );
                println!(
                    "默认目录 assets/samples；CSV 默认打到标准输出。四列：filename,rows,blocks,elapsed_ms"
                );
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

    let mut rows = vec!["filename,rows,blocks,elapsed_ms".to_string()];
    let mut measured = 0usize;
    let mut failed = 0usize;
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("读不到 {}：{err}", dir.display());
            return ExitCode::from(2);
        }
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.extension()
                .map(|ext| {
                    let ext = ext.to_string_lossy();
                    IMAGE_EXTENSIONS
                        .iter()
                        .any(|known| ext.eq_ignore_ascii_case(known))
                })
                .unwrap_or(false)
        })
        .collect();
    // 文件名排序：同一目录两次跑要给出同一份 CSV，否则差异会被误读成识别不稳定
    paths.sort();

    for path in &paths {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match measure(path) {
            Ok((rows_found, blocks, elapsed_ms)) => {
                measured += 1;
                rows.push(format!("{name},{rows_found},{blocks},{elapsed_ms}"));
            }
            Err(message) => {
                failed += 1;
                // 错误码/错误文字进 stderr，不进 CSV：CSV 是要留档的那份计数表
                eprintln!("{name} 未能完成识别：{message}");
            }
        }
    }

    let body = rows.join("\n");
    match &csv {
        Some(path) => {
            if let Err(err) = std::fs::write(path, format!("{body}\n")) {
                eprintln!("写不进 {}：{err}", path.display());
                return ExitCode::from(2);
            }
            println!("已写入 {}（{} 张）", path.display(), measured);
        }
        None => println!("{body}"),
    }

    if measured == 0 {
        println!();
        println!(
            "样本不足：{} 里没有可识别的图片（当前支持 PNG / JPEG）。",
            dir.display()
        );
        println!(
            "按停止条件 6，FR-2.2 的 ≥0.92 判据记为**未闭环**，不是通过；自绘度量（self_metric.rs）不能替它。"
        );
    } else {
        println!();
        println!(
            "计数已落表，但**字符正确率不在这里**：它要每张图的人工标注（docs/06 IMG-1~IMG-12）。"
        );
        println!("本工具只证明「这些图能被跑通、跑了多少行多少块、用了多久」，不证明识别质量。");
    }
    if failed > 0 {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// 一张图：S0 入场（含 >8000px 的等比缩放）→ S1~S3，只带回归数。
fn measure(path: &std::path::Path) -> Result<(usize, usize, u64), String> {
    let admitted = admit_path(path).map_err(|err| err.to_string())?;
    let (rows, _document, progress) =
        recognize_with_rows(&admitted.image).map_err(|err| err.to_string())?;
    Ok((rows.len(), progress.blocks, progress.elapsed_ms))
}
