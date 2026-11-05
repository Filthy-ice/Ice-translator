//! S8 导出：像素 → 字节的唯一一条路（CHG-006 REQ-008 / FR-2.6）。
//!
//! 为什么在视觉层而不是界面层：编码只吃一张 `RgbaImage` 和一个格式，不认识窗口、面板、剪贴板
//! 这些界面概念。放进界面层，下一个想导出的人就会在第二个视图文件里再写一遍 `PngEncoder`。
//! 更要紧的是那条隐私纪律——**默认不落盘、只写用户显式选中的路径**——它讲的是"什么时候允许把
//! 字节交出去"，必须和"字节怎么来"住在同一层，否则纪律会在两层之间各写一半。本模块因此不开
//! 面板、不改用户选好的路径、不碰剪贴板。
//!
//! 三条与格式有关的事实，各有一条断言钉着：
//!
//! 1. **PNG 直通**：RGBA 四个通道原样进编码器，alpha 一个比特都不丢（REQ-002 那句"导出保留
//!    alpha"只对这一条成立）。
//! 2. **JPEG 是合成不是丢弃**：它没有 alpha 通道。`image` 的默认转换是直接把那一个字节删掉，
//!    于是一整片"透明的黑"就变成一块黑底。这里改成先按 src-over 合成到纯白再编码，与浏览器把
//!    透明图存成 JPEG 的口径一致；界面那句"JPEG 会丢掉透明"（Q-005）说的就是这一步。
//! 3. **写失败不毁成品**：`save` 只借看图（`&RgbaImage`），形状上就没有清空内存那份的能力；
//!    错误只带固定的两句文案，**没有路径、没有图片字节**（NFR-3——OS 的原文通常整条路径都在里面，
//!    所以不转发它，原因由界面读那两句常量说给用户）。

use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};

use crate::error::{VisionError, VisionResult};

/// JPEG 的压缩质量。需求只说了"保存 PNG/JPEG"（FR-2.6），没给数值：85 是截图那类图
/// "字的边缘不糊"与"文件不翻倍"之间常用的那一档。**可调，但不进配置**——它属于编码这一步，
/// 不是用户偏好。
pub const JPEG_QUALITY: u8 = 85;

/// 写不出去时给用户的那句话（三样原因共用：目标是个目录、没有权限、磁盘满）
pub const SAVE_IO_TEXT: &str = "写入失败：目标位置不可写、是个目录，或磁盘空间不足";

/// 编码器拒绝这份像素时给的那句话。正常输入走不到这里，走到了就是编码侧的事，与用户无关
pub const ENCODE_TEXT: &str = "编码失败：这张图没能写成图片文件";

/// 成品格式。界面那边是"用户在面板里选了哪种扩展名"（Q-005），这里只管编码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
}

/// 扩展名决定格式（Q-005）：`jpg` / `jpeg` 走 JPEG，其余一律 PNG。
///
/// 认不出的一律 `Png`，因为 PNG 是需求里排在前面那个，也是唯一保住 alpha 的那个；大小写都认，
/// 用户在面板里手打 `.PNG` 不算错。
pub fn format_of(path: &Path) -> Format {
    let extension = path
        .extension()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("jpg") | Some("jpeg") => Format::Jpeg,
        _ => Format::Png,
    }
}

/// 编成字节，不落盘。**只有落盘那一条出路走它**：剪贴板那一路（REQ-008 的"复制图片"）交给
/// `arboard::set_image` 的是裸 RGBA 像素（见 `ice-app::system::clipboard::write_image`），
/// 由系统自己编成 TIFF/PNG。两条出口拿到的是同一张图，编码各归各家——所以这一条注释原来那句
/// "剪贴板要的也是这份字节"是错的，T-010 落地时按实际形状更正。
pub fn encode(image: &RgbaImage, format: Format) -> VisionResult<Vec<u8>> {
    let (width, height) = (image.width(), image.height());
    let mut buffer = Vec::new();
    match format {
        Format::Png => PngEncoder::new(&mut buffer).write_image(
            image.as_raw(),
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
        Format::Jpeg => JpegEncoder::new_with_quality(&mut buffer, JPEG_QUALITY).write_image(
            &flatten_over_white(image),
            width,
            height,
            ExtendedColorType::Rgb8,
        ),
    }
    .map_err(|_| VisionError::Composite(ENCODE_TEXT.to_string()))?;
    Ok(buffer)
}

/// 写到用户选中的那个路径。路径原样使用，一个字符都不改（补扩展名是面板那一步的事，
/// 在界面层发生在我们已经知道用户看了什么之后，而不是这里偷偷替他改名）。
pub fn save(image: &RgbaImage, path: &Path, format: Format) -> VisionResult<()> {
    let bytes = encode(image, format)?;
    std::fs::write(path, bytes).map_err(|_| VisionError::Composite(SAVE_IO_TEXT.to_string()))
}

/// 这张图里有没有透明或半透明的像素。
///
/// 只服务一句话：用户在面板里选了 JPEG 而图里确实有透明时，界面要说清"透明的地方按白色合成"
/// （Q-005，也就是上面第 2 条那一步）。PNG 那一路用不到它——alpha 原样保留，没什么可提醒的。
/// 判据放在这一层而不是界面层，是因为"JPEG 会丢 alpha"这件事的口径就住在这里。
pub fn has_transparency(image: &RgbaImage) -> bool {
    image.pixels().any(|pixel| pixel.0[3] < 255)
}

/// 把 RGBA 合成到纯白上的 RGB 字节串。
///
/// 每个通道 `out = (c × a + 255 × (255 − a) + 127) ÷ 255`：`a = 255` 时是原色，`a = 0` 时是纯白，
/// `+127` 是四舍五入。整数除法取的是 floor，不加这一项，半透明的浅灰会整体暗半格。
fn flatten_over_white(image: &RgbaImage) -> Vec<u8> {
    image
        .pixels()
        .flat_map(|pixel| {
            let [red, green, blue, alpha] = pixel.0;
            let cover = alpha as u32;
            [red, green, blue]
                .into_iter()
                .map(move |channel| {
                    ((channel as u32 * cover + 255 * (255 - cover) + 127) / 255) as u8
                })
                .collect::<Vec<u8>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 三种 alpha 各来一点：全不透明、全透明、以及量化边缘上的那一个
    fn swatch() -> RgbaImage {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(0, 0, Rgba([10, 200, 30, 255]));
        image.put_pixel(1, 0, Rgba([0, 0, 0, 0]));
        image.put_pixel(0, 1, Rgba([255, 255, 255, 128]));
        image.put_pixel(1, 1, Rgba([200, 20, 20, 17]));
        image
    }

    /// 解回来：字节 → 图。走 `image` 的解码器，与用户用别的应用打开是同一件事
    fn decode(bytes: &[u8]) -> RgbaImage {
        image::load_from_memory(bytes)
            .expect("自己编出来的字节要解得开")
            .to_rgba8()
    }

    #[test]
    fn a_png_round_trip_keeps_every_alpha_channel() {
        let source = swatch();
        let bytes = encode(&source, Format::Png).expect("PNG 编码不该失败");
        let back = decode(&bytes);

        assert_eq!(back.dimensions(), (2, 2));
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(
                    back.get_pixel(x, y).0,
                    source.get_pixel(x, y).0,
                    "({x},{y}) 这个像素过了 PNG 一趟就变了"
                );
            }
        }
    }

    #[test]
    fn a_jpeg_flattens_transparency_onto_white_instead_of_dropping_it() {
        // 各用一张**纯色**图：JPEG 是 4:2:0 的，小图上相邻格子的色度会串到一起，
        // 把"这一格本来该是什么"糊掉。纯色块之间比的是方向，量化误差吃不掉结论。
        // 透明的那张走 `image` 的默认转换（直接删掉 alpha 通道）会露出黑底，第一条断言当场过不去。
        let transparent = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        let cleared = decode(&encode(&transparent, Format::Jpeg).expect("JPEG 编码"));
        assert!(
            cleared.pixels().all(|p| p.0[..3].iter().all(|c| *c >= 250)),
            "全透明的图存成 JPEG 没有合成到白上：{:?}",
            cleared.get_pixel(3, 3)
        );

        // 不透明的深色必须还是深色，否则"合成到白"就变成"整张图冲淡一遍"
        let solid = RgbaImage::from_pixel(8, 8, Rgba([10, 200, 30, 255]));
        let dark = decode(&encode(&solid, Format::Jpeg).expect("JPEG 编码"));
        let [red, green, blue, alpha] = dark.get_pixel(3, 3).0;
        assert!(
            red < 40 && green > 160 && blue < 60,
            "不透明色被改得太多了：{red},{green},{blue}"
        );
        assert_eq!(alpha, 255, "JPEG 解出来不该有半透明");
    }

    /// 那句"JPEG 会丢掉透明"只在真有透明时才该出现：整张图不透明时提它是吓人。
    /// 判据是 `alpha < 255`，所以半透明（128）与量化边缘上那一点（17）都算，全不透明不算。
    #[test]
    fn transparency_is_reported_only_when_some_pixel_is_not_opaque() {
        assert!(has_transparency(&swatch()), "那张样图里有三种 alpha");
        assert!(
            has_transparency(&RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 0]))),
            "全透明当然算"
        );
        assert!(
            !has_transparency(&RgbaImage::from_pixel(4, 4, Rgba([10, 200, 30, 255]))),
            "整张不透明却报有透明，JPEG 那一路会白提一句"
        );

        // 一个像素就够：判据是"有没有"，不是"占多少"
        let mut one = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
        one.put_pixel(3, 3, Rgba([255, 255, 255, 254]));
        assert!(has_transparency(&one), "只差一个像素的透明也要报");
    }

    #[test]
    fn the_two_formats_announce_themselves() {
        let png = encode(&swatch(), Format::Png).expect("PNG 编码");
        let jpeg = encode(&swatch(), Format::Jpeg).expect("JPEG 编码");
        // 两个分支写反时，字节头和 `format_of` 都还会"看起来对"，只有这两条能当场抓住
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "那不是 PNG 的字节");
        assert_eq!(&jpeg[..2], b"\xFF\xD8", "那不是 JPEG 的字节");
    }

    #[test]
    fn the_extension_decides_and_anything_unfamiliar_falls_back_to_png() {
        let png =
            |name: &str| assert_eq!(format_of(Path::new(name)), Format::Png, "{name} 该走 PNG");
        png("成品.png");
        png("成品.PNG");
        png("没写扩展名");
        png("归档.tar.gz");
        png("点都在中间.foo.png.tmp");
        assert_eq!(format_of(Path::new("成品.jpg")), Format::Jpeg);
        assert_eq!(format_of(Path::new("成品.JPEG")), Format::Jpeg);
    }

    #[test]
    fn bytes_on_disk_are_the_bytes_encoded() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("成品.png");
        let source = swatch();

        save(&source, &path, Format::Png).expect("写临时目录不该失败");

        assert_eq!(
            std::fs::read(&path).expect("读得回来"),
            encode(&source, Format::Png).expect("PNG 编码"),
            "落盘的和编码的不是同一份字节"
        );
    }

    #[test]
    fn a_write_failure_names_the_problem_and_never_the_path() {
        let dir = tempfile::tempdir().expect("临时目录");
        // 目标是个目录（用户在面板里选中了文件夹那一行的形状）
        let err = save(&swatch(), dir.path(), Format::Png).expect_err("往目录写应当失败");
        let text = err.to_string();
        assert!(text.contains(SAVE_IO_TEXT), "那句话没说明原因：{text}");
        assert!(
            !text.contains(&dir.path().to_string_lossy().to_string()),
            "错误文案里带出了路径：{text}"
        );

        // 父目录不存在（磁盘那一路）
        let missing = dir.path().join("没有这一层").join("成品.png");
        let again = save(&swatch(), &missing, Format::Png).expect_err("父目录都没有，写不成");
        assert_eq!(again.to_string(), text, "两种写失败不该给两句话");
        assert!(!missing.exists(), "失败了却不许留下半个文件");
    }
}
