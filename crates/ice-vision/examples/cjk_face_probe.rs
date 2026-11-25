//! 取证（CHG-017 REQ-008）：本机候选字体里**第几面是简体中文面**。
//!
//! 与 mac 侧 `ice-app/examples/heiti_probe.rs` 同一条纪律——面号不靠猜：`STHeiti Medium.ttc`
//! 的第 0 面是黑体-繁，两面都覆盖常用简体字，选错既不崩也不缺字，只有字形悄悄换成半繁体。
//! Noto Sans CJK 的 `.ttc` 同理（JP/KR/SC/TC/HK 五面都在同一个文件里，`fc-match` 裸 `sans:lang=zh`
//! 实测命中的是 JP 面）。所以这一支只能按 name 表选面，而 name 表里的原文要落在证据里。
//!
//! 用法：`cargo run -p ice-vision --example cjk_face_probe`。只读 `/usr/share/fonts` 下的
//! 既有文件，不写盘、不外联。

use ice_core::font::{LINUX_CJK_CANDIDATES, LINUX_LATIN_PATH};
use owned_ttf_parser::Face;

/// 候选就是生产用的那张表（`ice_core::font::LINUX_CJK_CANDIDATES`），拉丁后备单独点名：
/// 取证脚本自己抄一份路径，测的就不再是要发布的那条路径
fn candidates() -> Vec<&'static str> {
    LINUX_CJK_CANDIDATES
        .iter()
        .copied()
        .chain([LINUX_LATIN_PATH])
        .collect()
}

/// name 表里参与选面的几条：1 = family，16 = preferred family（`.ttc` 里的可变字体常用），
/// 4 = full name，6 = PostScript 名。
const NAME_IDS: &[u16] = &[1, 4, 6, 16];

fn dump(path: &str) {
    let Ok(bytes) = std::fs::read(path) else {
        println!("{path}: 读不到文件");
        return;
    };
    println!("{path}");
    for index in 0..8u32 {
        let Ok(face) = Face::parse(&bytes, index) else {
            println!("  #{index}: 没有这一面");
            break;
        };
        let names: Vec<String> = NAME_IDS
            .iter()
            .filter_map(|&id| {
                face.names()
                    .into_iter()
                    .find(|n| n.name_id == id)
                    .and_then(|n| n.to_string())
            })
            .collect();
        println!(
            "  #{index}: 汉字={} {names:?}",
            face.glyph_index('译').is_some()
        );
    }
}

fn main() {
    for path in candidates() {
        dump(path);
    }
}
