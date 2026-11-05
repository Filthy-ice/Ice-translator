//! Ice Translator 视觉层：图片 OCR 与译文回填。
//!
//! 图片处理必须完全在本机完成（NFR-3），本 crate 不发起任何网络请求。

pub mod bg;
pub mod composite;
pub mod error;
pub mod import;
pub mod layout;
pub mod ocr;
#[cfg(target_os = "macos")]
pub mod vision_sys;

pub use composite::{CompositeOptions, CompositeOutput, Renderer};
pub use error::{VisionError, VisionResult};
pub use import::Admitted;
pub use layout::{BlockDraft, MergeParams, NormalizedRect, RowObservation, flip_rect, merge_rows};
pub use ocr::{
    LOW_CONFIDENCE_WARN, MAX_EDGE_PX, OcrEngine, OcrProgress, VisionOcr, default_engine,
    recognize_with_rows,
};

/// 当前平台是否具备原生 OCR 能力（NFR-7 的降级依据）
pub const fn native_ocr_supported() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(test)]
mod tests {
    //! 跨平台编译面的静态核查（CHG-005 REQ-007 / T-008）。
    use std::fs;
    use std::path::{Path, PathBuf};

    /// `vision_sys` 里的每一条 ObjC 调用只能在 macOS 编译面上出现。
    ///
    /// 判据为什么用源码扫描而不是交叉编译：真机 Linux/Windows 验证排在 P8（NFR-7 只承诺 S 档），
    /// 而漏掉一处门控的后果不是"跑得慢"，是那个平台根本编不过——这一条在 mac 上就能静态成立。
    /// 扫描器看不懂语法，所以口径写死成两句：注释里的 `vision_sys` 不算（那是人写的说明），
    /// 代码里的每一次出现必须被一条 `#[cfg(target_os = "macos")]` 罩着。
    #[test]
    fn every_vision_sys_reference_is_behind_a_macos_gate() {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect_rs(&src, &mut files);
        assert!(
            !files.is_empty(),
            "一个 .rs 都没扫到，这条断言就成了空转：{src:?}"
        );

        let mut ungated: Vec<String> = Vec::new();
        let mut hits = 0;
        for file in &files {
            // 框架绑定本体不需要门控自己：它的模块声明在 `lib.rs` 已经被 cfg 罩住
            if file
                .file_name()
                .map(|name| name == "vision_sys.rs")
                .unwrap_or(false)
            {
                continue;
            }
            let name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            for (number, line, gated) in lines_with_gate(&read(file)) {
                if !line.contains("vision_sys") {
                    continue;
                }
                hits += 1;
                if !gated {
                    ungated.push(format!(
                        "{name}:{number} 在 macOS 门控之外引用了 vision_sys"
                    ));
                }
            }
        }
        assert!(
            ungated.is_empty(),
            "非 mac 编译面会引用到框架符号：\n{}",
            ungated.join("\n")
        );
        // 扫到零处也说明有问题：本 crate 一定至少有两处（模块声明 + S1 调用）
        assert!(
            hits >= 2,
            "一条 vision_sys 引用都没扫到，扫描器或目录不对：{hits}"
        );
    }

    /// 逐行给出"这一行是否落在某条 `#[cfg(target_os = "macos")]` 之下"。
    ///
    /// 门控按大括号深度成栈：属性行给紧跟其后的那个条目定门，条目开着的花括号把这段门控
    /// 一直延伸到配平为止。注释行整行跳过——它们不参与编译，不会因为写了 `vision_sys`
    /// 就让 Linux 编不过。
    fn lines_with_gate(text: &str) -> Vec<(usize, String, bool)> {
        let mut out = Vec::new();
        // 栈里每一项 = 该条目所在线段的结束深度 与 它的门控文字
        let mut stack: Vec<(usize, Option<String>)> = Vec::new();
        let mut depth: usize = 0;
        let mut pending: Option<String> = None;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            let inherited = stack.last().and_then(|(_, gate)| gate.clone());
            if line.starts_with("#[") {
                // `#[derive(..)]`、`#[inline]` 这类不改变门控；只有 `#[cfg(..)]` 才是新的一层
                pending = if line.starts_with("#[cfg(") {
                    Some(line.to_string())
                } else {
                    None
                };
                out.push((
                    index + 1,
                    raw.to_string(),
                    covers(pending.as_deref(), inherited),
                ));
                continue;
            }
            if line.starts_with("//") {
                continue;
            }
            let carried = pending.is_some();
            let gate = pending.take().or_else(|| inherited.clone());
            let gated = covers(gate.as_deref(), inherited.clone());
            let opens = line.chars().filter(|c| *c == '{').count();
            let closes = line.chars().filter(|c| *c == '}').count();
            // 属性挂在一条跨多行的签名上（`pub fn f(` / 参数 / `) -> T {`）时，门控要一直
            // 攥到那一行真的开了花括号或以 `;` 收尾为止，否则函数体就落在门控外面
            if carried && opens == 0 && closes == 0 && !line.ends_with(';') {
                pending = gate.clone();
            }
            let entered = depth + opens - closes.min(depth);
            if opens > closes {
                stack.push((entered, gate));
            }
            depth = entered;
            while stack.last().is_some_and(|(top, _)| depth < *top) {
                stack.pop();
            }
            out.push((index + 1, raw.to_string(), gated));
        }
        out
    }

    /// 这一行的门控：自己那层或外面任何一层是 macOS 专属就算罩住了——往里只会更窄，
    /// 不会把一段 mac 专属放宽到别的平台。`#[cfg(test)]` 那一段也算罩住：测试代码不进任何
    /// 平台的发布编译面，本模块里作为反例字符串写出的 `vision_sys` 因此不该算一次引用。
    fn covers(gate: Option<&str>, inherited: Option<String>) -> bool {
        is_macos_gate(gate)
            || is_macos_gate(inherited.as_deref())
            || is_test_gate(gate)
            || is_test_gate(inherited.as_deref())
    }

    fn is_test_gate(attr: Option<&str>) -> bool {
        attr.is_some_and(|a| a.contains("cfg(test)"))
    }

    fn is_macos_gate(attr: Option<&str>) -> bool {
        // `not(target_os = "macos")` 也含 "macos"，所以先排除取反的那一种
        let Some(attr) = attr else {
            return false;
        };
        attr.contains("target_os") && attr.contains("macos") && !attr.contains("not(")
    }

    fn read(file: &Path) -> String {
        fs::read_to_string(file).expect("读得到自己 crate 里的源文件")
    }

    fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs(&path, out);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                out.push(path);
            }
        }
    }

    /// 反向的那一条：门控写反（`not(target_os = "macos")`）不该被当成"罩住了"
    #[test]
    fn an_inverted_gate_does_not_count_as_a_gate() {
        assert!(!is_macos_gate(Some("#[cfg(not(target_os = \"macos\"))]")));
        assert!(is_macos_gate(Some("#[cfg(target_os = \"macos\")]")));
        assert!(!is_macos_gate(None));
    }

    /// 扫描器不能只在"全都合格"时沉默：这三段反例逐条给出行号，证明它真的看得见。
    #[test]
    fn the_scanner_catches_each_way_of_escaping_the_gate() {
        let bare = lines_with_gate("use crate::vision_sys::recognize_rows;\n");
        assert_eq!(bare.len(), 1);
        assert!(!bare[0].2, "没有门控的 use 被放过了");

        let inverted = lines_with_gate(
            "#[cfg(not(target_os = \"macos\"))]\npub fn x() { crate::vision_sys::y() }\n",
        );
        assert!(
            inverted
                .iter()
                .any(|(_, line, gated)| line.contains("vision_sys") && !*gated),
            "门控写反也被当成了罩住：{inverted:?}"
        );

        let nested = lines_with_gate(
            "#[cfg(target_os = \"macos\")]\npub fn a() {\n    if true {\n        crate::vision_sys::b();\n    }\n}\n",
        );
        let call = nested
            .iter()
            .find(|(_, line, _)| line.contains("vision_sys"))
            .expect("嵌套调用要扫得到");
        assert!(call.2, "函数体深处的引用没继承外层门控：{nested:?}");

        // 注释里提一句框架名字不该进结果，更不该算一次引用
        let commented = lines_with_gate("/// `vision_sys` 把对象拆成中间态\npub fn c() {}\n");
        assert!(
            commented
                .iter()
                .all(|(_, line, _)| !line.starts_with("///")),
            "注释行还是被收进结果了：{commented:?}"
        );
    }
}
