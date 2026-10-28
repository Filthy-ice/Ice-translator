//! 中英混排语言检测的复核探针（`06-test-cases.md` TC-1.2-b）。
//!
//! 用法：`cargo run -p ice-core --example detect_probe`
//!
//! 表里每一条都是**真人会复制进输入框**的形状，`期望` 列按"这句话的语法骨架是哪门语言"
//! 人工标注（技术术语借词不算骨架）。探针同时打印检测出的语言、置信度与内部计数，
//! 因为只判对错看不出是被字母数压过去还是被权重压过去。

use ice_core::detect::{Detection, detect};

/// 一条待复核的实例。
struct Case {
    text: &'static str,
    /// 期望语言，`None` 表示"应当检不出"（纯数字/符号/空）。
    expected: Option<&'static str>,
    note: &'static str,
}

const CORPUS: &[Case] = &[
    // —— 纯单语，基线 ——
    Case {
        text: "缓存失效策略需要按分片批处理",
        expected: Some("简体中文"),
        note: "纯中文基线",
    },
    Case {
        text: "The cache invalidation strategy needs batch processing per shard",
        expected: Some("英语"),
        note: "纯英文基线",
    },
    // —— 中文骨架 + 英文借词/标识符（本次复核的主战场）——
    Case {
        text: "请在 config.toml 里把 cache_ttl 改成 300 秒",
        expected: Some("简体中文"),
        note: "配置文件里改参数：中文人说的一句话",
    },
    Case {
        text: "这个 K8s pod 一直 CrashLoopBackOff，node 也 NotReady",
        expected: Some("简体中文"),
        note: "运维口语，标识符很长但骨架是中文",
    },
    Case {
        text: "麻烦确认下 SLA 里的 P99 是不是按自然月统计",
        expected: Some("简体中文"),
        note: "全是缩写术语的中文句",
    },
    Case {
        text: "缓存失效 cache invalidation 策略需要按分片批处理",
        expected: Some("简体中文"),
        note: "中英对照写法",
    },
    Case {
        text: "我看了下 PR 的 diff，有个 typo 在 config 里",
        expected: Some("简体中文"),
        note: "代码评审口语",
    },
    // —— 英文骨架 + 少量中文 ——
    Case {
        text: "flush 缓存 before running the benchmark again",
        expected: Some("英语"),
        note: "英文句里嵌一个中文词",
    },
    Case {
        text: "The error message says 连接超时 and nobody knows why",
        expected: Some("英语"),
        note: "英文句里引用一段中文报错",
    },
    Case {
        text: "please translate 这段话 into English",
        expected: Some("英语"),
        note: "英文祈使句夹四个汉字",
    },
    // —— 其它文字系统（确认没有因为改动而互相压过）——
    Case {
        text: "メモリリークを調査します",
        expected: Some("日语"),
        note: "日文混汉字",
    },
    Case {
        text: "안녕하세요 반갑습니다",
        expected: Some("韩语"),
        note: "韩文基线",
    },
    Case {
        text: "Шинный кэш устарел",
        expected: Some("俄语"),
        note: "俄文基线",
    },
    // —— 应当检不出 ——
    Case {
        text: "2026-10-19 12:30:45",
        expected: None,
        note: "纯数字与符号",
    },
];

fn main() {
    let mut wrong = 0usize;
    println!(
        "{:<4}{:<34}{:<10}{:>7}  {:>6}{:>6}{:>6}  备注",
        "#", "实例", "检出", "置信度", "汉字", "英词", "字母"
    );
    println!("{}", "-".repeat(120));
    for (i, c) in CORPUS.iter().enumerate() {
        let d = detect(c.text);
        let got = label_of(d);
        let ok = c.expected == Some(got) || (c.expected.is_none() && d.is_none());
        if !ok {
            wrong += 1;
        }
        let (han, words, letters) = counts(c.text);
        let conf = d
            .map(|x| format!("{:.2}", x.confidence))
            .unwrap_or_default();
        println!(
            "{:<4}{:<34}{:<10}{:>7}  {:>6}{:>6}{:>6}  {}{}",
            i + 1,
            trim(c.text),
            got,
            conf,
            han,
            words,
            letters,
            if ok { "✓" } else { "✗ 期望 " },
            if ok {
                c.note.to_string()
            } else {
                format!("{} ← {}", c.expected.unwrap_or("检不出"), c.note)
            }
        );
    }
    println!("\n判错 {wrong} 条 / 共 {} 条", CORPUS.len());
    if wrong > 0 {
        std::process::exit(1);
    }
}

fn label_of(d: Option<Detection>) -> &'static str {
    match d {
        None => "检不出",
        Some(x) => x.language.display_name(),
    }
}

/// 内部计数只为定位成因，不参与判定。切分口径与 `detect` 一致：字母与数字连成一段，
/// 段里至少有一个字母才算一个术语（`k8s`、`P99` 各一段，`300` 不算）。
fn counts(text: &str) -> (usize, usize, usize) {
    let han = text
        .chars()
        .filter(|c| {
            let x = *c as u32;
            (0x4E00..=0x9FFF).contains(&x) || (0x3400..=0x4DBF).contains(&x)
        })
        .count();
    let letters = text
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || matches!(*c as u32, 0x00C0..=0x024F))
        .count();
    let words = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| {
            t.chars()
                .any(|c| c.is_ascii_alphabetic() || matches!(c as u32, 0x00C0..=0x024F))
        })
        .count();
    (han, words, letters)
}

fn trim(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 16 {
        s.to_string()
    } else {
        chars[..16].iter().collect::<String>() + "…"
    }
}
