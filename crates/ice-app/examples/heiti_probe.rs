//! 取证：`/System/Library/Fonts/STHeiti *.ttc` 里第几面能覆盖界面用到的简体字。
//!
//! 走的是要发布的那条路径本身（`FontData::from_blob(bytes, index)` + `has_glyph`），
//! 所以这里通了就说明实现通了。用法：`cargo run -p ice-app --example heiti_probe`。
use egui::{FontData, FontDefinitions, FontFamily, FontId};

/// 界面与正文里真会出现的字，含常见标点与拉丁（拉丁也换成黑体是用户要的"全部"）
const SAMPLE: &str =
    "中文翻译简体字数检测耗时缓存分段设置窗口悬浮窗全量退出置顶复制自动浅色深色，。：（）、AaBb019";

fn probe(path: &str, idx: u32) {
    let Ok(bytes) = std::fs::read(path) else {
        println!("{path} #{idx}: 读不到文件");
        return;
    };
    let name = format!("heiti{idx}");
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        name.clone(),
        FontData::from_blob(std::sync::Arc::new(bytes), idx).into(),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, name.clone());
    }
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts);
    // 字体清单要到第一次 run 才建立（`context.rs:1387`），先空跑一帧
    ctx.run_ui(egui::RawInput::default(), |_ui| {})
        .drop_without_applying_deltas();
    // 要发布的形态：黑体排在 Proportional 族的第一位，中文按这个族解析
    let font_id = FontId::new(14.0, FontFamily::Proportional);
    let missing: Vec<char> = SAMPLE
        .chars()
        .filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&font_id, *c)))
        .collect();
    println!("{path} #{idx}: 缺字 {} 个 {:?}", missing.len(), missing);
}

fn main() {
    for path in [
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
    ] {
        for idx in 0..2 {
            probe(path, idx);
        }
    }
}
