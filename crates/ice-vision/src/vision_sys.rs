//! 全 crate 唯一触碰 macOS Vision 框架的文件（CHG-005 REQ-002）。
//!
//! 之所以单独成文件、并由 `lib.rs` 整文件按 `cfg(target_os = "macos")` 挂进来：
//! 1. **`!Send` 的 `Retained` 不许出栈**。objc2-vision 全文没有一处 `unsafe impl Send/Sync`，
//!    所以框架对象只能在本文件的调用内生死，出栈的只有纯 Rust 的 [`RowObservation`]。
//! 2. **零外发的取证面收敛到一处**：`cargo tree` 与 grep 只需盯这个文件与 `Cargo.toml`。
//! 3. 非 macOS 平台根本不编译 Vision 绑定（NFR-7 的降级由 `ocr::default_engine()` 返回 None 承担）。

use image::RgbaImage;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFData, CFRetained, CGRect};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    kCGColorSpaceSRGB,
};
use objc2_foundation::{NSArray, NSDictionary, NSString};
use objc2_vision::{
    VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
    VNRequestTextRecognitionLevel,
};

use crate::error::{VisionError, VisionResult};
use crate::layout::{NormalizedRect, RowObservation};

/// 语言候选（ADR-002 定的三条，写死而不开放给用户：候选表决定识别行为）。
const RECOGNITION_LANGUAGES: [&str; 3] = ["zh-Hans", "en", "ja"];

/// 一行像素的字节数：RGBA8，无行填充。
const BYTES_PER_PIXEL: usize = 4;

/// S1：把内存里的 RGBA 图交给 Vision 做一次同步文本识别，返回逐行观察。
///
/// 同步阻塞，调用方负责放进 `spawn_blocking`。空结果不是错误路径而是 `NoTextFound`，
/// 界面据此走"未发现文字"那档中性提示。识别出的文本一个字都不写日志（NFR-3）。
pub fn recognize_rows(image: &RgbaImage) -> VisionResult<Vec<RowObservation>> {
    let cg_image = build_cg_image(image)?;

    // # Safety
    // `initWithCGImage_options` 的首参要求一个刚 alloc、尚未初始化的 `Allocated`——alloc 只给出
    // 未活性内存，未经 init 不成活，所以这两步必须连在一起写、中间不许 early-return。
    // `cg_image` 在本次调用期间被借用持有（handler 自己会 retain 它），`options` 是类型正确的空字典。
    let handler = unsafe {
        VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            &cg_image,
            &empty_options(),
        )
    };

    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    let languages = NSArray::from_retained_slice(
        &RECOGNITION_LANGUAGES
            .iter()
            .map(|&lang| NSString::from_str(lang))
            .collect::<Vec<_>>(),
    );
    request.setRecognitionLanguages(&languages);
    request.setUsesLanguageCorrection(true);
    // 开着它等于把"用哪种语言"交给框架自由发挥（且只在 Revision3 起生效），度量结果不可复现。
    request.setAutomaticallyDetectsLanguage(false);

    // 元素类型上抛给父类：ObjC 侧数组本来只存 id，转的只是 Rust 的泛型标记。
    // # Safety
    // `VNRecognizeTextRequest` 是 `VNRequest` 的子类，指针表示同一对象。
    let request_as_vn_request: Retained<VNRequest> =
        unsafe { Retained::cast_unchecked(request.clone()) };
    let requests = NSArray::from_retained_slice(&[request_as_vn_request]);

    if let Err(error) = handler.performRequests_error(&requests) {
        // 只记错误码与描述长度，描述正文可能含识别出的文字，不进日志。
        let desc_len = error.localizedDescription().len();
        return Err(VisionError::Recognition(format!(
            "perform_failed_code={} desc_len={}",
            error.code(),
            desc_len
        )));
    }

    let Some(observations) = request.results() else {
        return Err(VisionError::NoTextFound);
    };
    let mut rows = Vec::with_capacity(observations.len());
    for (row_index, observation) in observations.iter().enumerate() {
        // 只取第一名候选：把候选全塞进界面数据等于把"哪条是结果"推给用户。
        let Some(candidate) = observation.topCandidates(1).iter().next() else {
            continue;
        };
        let text = candidate.string().to_string();
        let confidence = candidate.confidence();
        let box_norm = to_normalized(unsafe { observation.boundingBox() });
        rows.push(RowObservation {
            text,
            normalized_box: box_norm,
            confidence,
            row_index,
        });
    }
    if rows.is_empty() {
        return Err(VisionError::NoTextFound);
    }
    Ok(rows)
}

/// 归一化框直接取自 `CGRect`，翻转与夹取一律留给 `layout::flip_rect`（那里才有像素基准）。
fn to_normalized(rect: CGRect) -> NormalizedRect {
    NormalizedRect {
        x: rect.origin.x as f32,
        y: rect.origin.y as f32,
        w: rect.size.width as f32,
        h: rect.size.height as f32,
    }
}

/// RGBA8 字节 → `CGImage`，不落盘、不经 ImageIO。
///
/// 用 `CGImageAlphaInfo::Last`（非预乘）如实描述 `RgbaImage` 的字节含义；alpha 恒为 255 的
/// 截图里它与 PremultipliedLast 等价，但带透明像素的图不该被谎报成已预乘。
///
/// 返回 `CFRetained` 而非 `Retained`：Core Graphics 是 CF 类型，objc2 不给它 `into_inner`，
/// 而 `initWithCGImage_options` 收 `&CGImage`，`&CFRetained<CGImage>` 自动解引用即可。
fn build_cg_image(image: &RgbaImage) -> VisionResult<CFRetained<CGImage>> {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let data = CFData::from_bytes(image.as_raw());
    let provider = CGDataProvider::with_cf_data(Some(&data))
        .ok_or_else(|| VisionError::Recognition("data_provider_failed".to_string()))?;
    // `kCGColorSpaceSRGB` 是 extern 静态，读它本身就得在 unsafe 里（Rust 无法核实链接进来的常量）。
    let color_space = unsafe { CGColorSpace::with_name(Some(kCGColorSpaceSRGB)) }
        .ok_or_else(|| VisionError::Recognition("color_space_failed".to_string()))?;
    // # Safety
    // 宽高、位深与 `bytes_per_row` 三者自洽（RGBA8 = 32bpp），字节缓冲由 `image` 在整个调用期间持有，
    // `decode` 传 null 表示不做分量重映射——这是 Core Graphics 允许的取值。
    unsafe {
        CGImage::new(
            width,
            height,
            8,
            BYTES_PER_PIXEL * 8,
            width * BYTES_PER_PIXEL,
            Some(&color_space),
            CGBitmapInfo(CGImageAlphaInfo::Last.0),
            Some(&provider),
            core::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or_else(|| VisionError::Recognition("cgimage_failed".to_string()))
}

/// Vision 的 options 字典：本包不用任何辅助信息（无相机内参、无 CIContext）。
fn empty_options() -> Retained<NSDictionary<VNImageOption, AnyObject>> {
    NSDictionary::new()
}
