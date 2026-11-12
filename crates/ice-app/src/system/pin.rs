//! 贴图窗（CHG-009 REQ-003/REQ-004）。
//!
//! **这一格的地基是 2026-11-05 那晚仓外探针量出来的**（事实 17/25/26/27/29/32/33）：
//! 子 viewport 里那张纹供得出来、画的就是我们算的那块几何、无边框子窗收得到键盘且只给
//! 焦点那一张、同时钉 8 张各自出图。所以这个模块不是"猜一个形态再改"，是把读数控成代码。
//!
//! 三条从读数里出来的硬规矩，写在这里而不是散在绘制里：
//! 1. **操作条是另一扇浮空窗**（CHG-011），不画在这扇贴图窗里——用户那句"离图 3px 的圆角
//!    单独操作栏"是字面意思，图上压控件或把带子焊在窗口下缘都违背它。本模块只画图片那一块。
//! 2. **贴图窗恒等于图片**（`window_size` = 图片点尺寸，不随收放变）。实测过"画一块不透明
//!    带子却不缩窗口"那一档：那条带子不画东西、**还照吃命中**——一条隐形陷阱。所以贴图窗里
//!    再也没有"不画却吃命中"的地方；收放只决定那扇药丸窗在不在。
//! 3. **位置只从 `ViewportInfo::outer_rect` 读**，本模块 SHALL NOT 自己累计第二份坐标
//!    （拖动、系统搬窗、隐藏再回来三种情况它都跟着，事实 32）。
//!
//! `scale` 那一处除法是全局唯一出处（像素 → 点），窗口尺寸、带子重算、文字层坐标三处
//! 都读同一个结果；各自再除一次或都忘了除，就是"贴图比用户框的大一倍"那种现场（H9）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::system::{bar, pill, wake_root};
use egui::{Id, Key, Modifiers, Pos2, Rect, Response, Sense, Ui, Vec2};

/// 文字层选中块的高亮：淡蓝底 + 一档深一点的边。颜色跟应用自己的图标同一族（不新造色系）。
fn select_tint() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(0x2E, 0x6F, 0xF2, 60)
}

fn select_edge() -> egui::Stroke {
    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2E, 0x6F, 0xF2))
}

// 带子的概念已经在 CHG-011 里被**浮空药丸**取代：贴图窗恒等于图片那一块，操作条是另一扇
// 窗（`system/pill.rs`）。这里不再有 `BAND_H`，也不再有多出来的一块死区（事实 3 那条隐形
// 陷阱因此结构性消失）。药丸本体的行高在 `pill::PILL_H`，与这里无关。

/// 带子里那三颗（`ACTIONS` 去掉「关闭」），**顺序就是显示顺序**。
///
/// 单独一张表而不是在带子里 `filter`：这样"药丸上没有关闭"这件事写在类型里，
/// 谁把 Close 塞回来会少一颗，断言立刻响。
pub const BAND_ACTIONS: [PinAction; 3] = [
    PinAction::Translate,
    PinAction::CopyImage,
    PinAction::SaveImage,
];

/// 右键菜单那四项，**顺序就是显示顺序**。
///
/// 一处声明、长度锁 4：用户明说"只要四项"（Q-006 默认 翻译 / 复制图像 / 保存图像 / 关闭），
/// 带子和右键菜单读同一张表。加第五项要改这一处，而 `exactly_four_actions` 那条断言会先响。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinAction {
    Translate,
    CopyImage,
    SaveImage,
    Close,
}

/// 四颗按钮的表。带子里画它，右键菜单也列它——**不是两套清单**。
pub const ACTIONS: [PinAction; 4] = [
    PinAction::Translate,
    PinAction::CopyImage,
    PinAction::SaveImage,
    PinAction::Close,
];

impl PinAction {
    /// 按钮上那两个字的来源。文案是一次性资产（`docs/05` §6 那条），带子与菜单共用。
    pub fn label(self) -> &'static str {
        match self {
            PinAction::Translate => "翻译",
            PinAction::CopyImage => "复制",
            PinAction::SaveImage => "保存",
            PinAction::Close => "关",
        }
    }

    /// 右键菜单里要说全的名字（带子上挤成两个字，菜单里不必省）。
    pub fn menu_label(self) -> &'static str {
        match self {
            PinAction::Translate => "翻译这张图",
            PinAction::CopyImage => "复制图像",
            PinAction::SaveImage => "保存图像",
            PinAction::Close => "关闭这张贴图",
        }
    }
}

/// 一张贴图该占的窗口尺寸与那块图片矩形。**这条 REQ 唯一能被无头锁住的部分**。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinLayout {
    /// 窗口内容尺寸（点）。**恒等于图片那块**——操作条在另一扇药丸窗里，不再多出 `BAND_H`
    pub window_size: Vec2,
    /// 图片画在哪块（点，左上角是窗口原点）
    pub image_rect: Rect,
}

/// 像素尺寸 + 那张屏的 `pixels_per_point` → 窗口几何。
///
/// `image_px` 是**像素**（PNG 读出来的原尺寸），`scale` 是那张贴图所在屏的 ppp。
/// 除法只在这一处发生，产出的两块矩形全是点。收放带子改由药丸窗的显隐负责，与本函数无关。
pub fn pin_layout(image_px: Vec2, scale: f32) -> PinLayout {
    // 唯一一处除法。ppp 拿不到（0 或负）时按 1.0 兜住——那不是"截错了"，
    // 而是这一帧还没有屏信息，宁可 1:1 也不能算出 0 尺寸的窗口
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let size = image_px / scale;
    let image_rect = Rect::from_min_size(Pos2::ZERO, size);
    PinLayout {
        window_size: size,
        image_rect,
    }
}

/// 一张贴图能对外发出的**全部意图**。带子里的按钮、右键菜单里的四项、键盘那几颗、
/// 文字层那一次拖选，四处都只能落到这七种之一——矩阵一旦分叉，"点按钮有效、按键没反应"
/// 这类只对一半的缺陷就回来了（图钉那次是两条通道，本包同一条纪律：一个枚举、一处映射）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinIntent {
    Translate,
    CopyImage,
    SaveImage,
    /// 销毁这张贴图的窗口。**不动历史文件**（REQ-004 明写）
    Close,
    /// 空格：收放带子
    ToggleBand,
    /// 裸 `C`。这一层**不判**它该复制字还是图——选中集合在状态层，分派在 `app.rs`
    /// （REQ-004 末段 + REQ-005：判据依赖文字层状态，而文字层状态不归这个模块持有）。
    Copy,
    /// 文字层一次拖选收尾：选中的块 id。**空表也算一次写入**——它就是"取消选中"，
    /// 当"这帧没新信息"忽略掉的话，用户松开一个空拖之后界面还写着有选中。
    Select(Vec<u32>),
}

impl PinIntent {
    pub fn from_action(action: PinAction) -> Self {
        match action {
            PinAction::Translate => PinIntent::Translate,
            PinAction::CopyImage => PinIntent::CopyImage,
            PinAction::SaveImage => PinIntent::SaveImage,
            PinAction::Close => PinIntent::Close,
        }
    }
}

/// 键盘 → 意图。**这是"每颗键给哪条意图"那张矩阵的唯一出处**。
///
/// 三条不显然的：
/// - **esc 无论带子开合都是关窗**。不做"先关带子再关窗"的两段式（REQ-004 末句：那是
///   会让人按第二下才发现的坑）。
/// - 裸 `C` 给 [`PinIntent::Copy`]，**不给 `CopyImage`**：到底是"复制图像"还是"复制选中的
///   字"要看这张有没有选中（REQ-004 末段 + REQ-005），那个判据在状态层，由 `app.rs` 分支。
///   在这里直接判成图像，就是"选中了字却粘出一张图"——用户报过的第 3 类缺陷的形状。
/// - 带 `Alt`/`Ctrl`(非 mac)/其他输入法组合的一律不抢，与 `system::hotkeys.rs` 同一条纪律。
pub fn key_intent(key: Key, modifiers: Modifiers) -> Option<PinIntent> {
    let copy_chord = if cfg!(target_os = "macos") {
        modifiers.command && !modifiers.alt
    } else {
        modifiers.ctrl && !modifiers.alt
    };
    match key {
        Key::Escape => Some(PinIntent::Close),
        Key::Space => Some(PinIntent::ToggleBand),
        Key::C if copy_chord => Some(PinIntent::CopyImage),
        Key::C if modifiers.is_none() => Some(PinIntent::Copy),
        _ => None,
    }
}

/// 这一帧要不要给窗口发"改尺寸"命令。
///
/// `ViewportBuilder` 的尺寸只在**创建那一帧**生效（事实 26），所以收放带子之后只能靠
/// `InnerSize`；而它 SHALL NOT 每帧发（会把用户的拖动/系统搬窗搅出一堆无谓的 resize 抖动），
/// 只在内容尺寸真的变了那一刻发一次。上一帧的尺寸从 `outer_rect` 换算，不自己攒坐标。
pub fn inner_size_command(previous: Option<Vec2>, now: Vec2) -> Option<Vec2> {
    match previous {
        Some(prev) if prev == now => None,
        Some(prev) if (prev - now).length() < 0.5 => None, // 亚像素抖动不发
        _ => Some(now),
    }
}

/// 药丸里那一排三颗画进 `ui`，按下的动作推进 `sink`（由 `pill.rs` 在药丸窗那一帧调用）。
///
/// **清单与文案仍只在这一处**（遍历 `BAND_ACTIONS` 那张表，标签是 `action.label()` 那两个字）；
/// 只是绘制从"egui 默认文本按钮（字外套一层底和框）"换成 [`bar::text_button`]（平时只有字、
/// 悬停变淡浮起，REQ-002/REQ-009 的改口）。**不加 `glyph()` 映射**——改判后贴图消费文字按钮，
/// 不消费线稿（线稿 `icon_button` 留在 `selection.rs` 只给遮罩用）。
pub fn pill_buttons(ui: &mut Ui, sink: &mut Vec<PinIntent>) {
    ui.horizontal(|ui| {
        // **药丸上没有「关闭」**（用户 2026-11-09 晚：不需要在界面上搞一个关闭的按钮）。
        // 关闭只走两条：esc 关当前这张、右键菜单那颗。右键仍读完整的 `ACTIONS` 四项。
        for action in BAND_ACTIONS {
            // 禁用口径照旧：识别/翻译进行中那颗"翻译"由调用方 `ui.add_enabled_ui` 关掉；
            // 禁用态那颗不进悬停支（REQ-009 边界，`bar::text_button` 自己判 `response.enabled()`）
            let response = bar::text_button(ui, action.label());
            if response.clicked() {
                sink.push(PinIntent::from_action(action));
            }
            response.on_hover_text(action.menu_label());
        }
    });
}

/// 把一帧里某张贴图攒下的意图塞进它的交接槽（药丸窗的按钮与 esc 走这一条，不开第二条通道）。
///
/// 与贴图窗自己那三处（带子已搬进药丸、右键、键盘）共用同一个 `intents_slot`——槽按 pin id
/// 键控，所以"按一次关掉两张"结构上不成立（design REQ-004）。
pub fn submit_intents(ctx: &egui::Context, id: u64, intents: Vec<PinIntent>) {
    if intents.is_empty() {
        return;
    }
    if let Ok(mut queue) = intents_slot(ctx, id).lock() {
        queue.extend(intents);
    }
}

/// 图片本体那一块的命中区。**左键按住拖动走 `StartDrag`**（REQ-004 第一条，见 `show` 那一处），
/// **不开 `with_movable_by_background`**——它一开无边框窗的整片内容都算背景，每一次按下都被
/// AppKit 当成拖窗口吃掉，于是按钮收不到点击、右键弹不出、窗从来没成为 key window
/// （esc/空格也进不来），用户报的"三个关法都不管用"就是这一个原因（D-29）。
///
/// `Sense::click()` 只为了右键菜单吃得着这一下。位置不自己算增量：拖到哪都只从
/// `outer_rect` 现读（硬规矩 3），第二条坐标计算不在这里出现。
pub fn image_area(ui: &mut Ui, size: Vec2) -> Response {
    ui.allocate_rect(Rect::from_min_size(Pos2::ZERO, size), Sense::click())
}

/// 文字层那一次拖选命中了哪些块（REQ-005）。**纯几何，无头可断言**。
///
/// 三条判据都在这个函数里，别处不许再算第二遍：
/// - **块框从像素换算到点**只在这一处：`image` 那块矩形是点、`document` 的 `bbox` 是像素，
///   比值由这两个数现算（不引第三个来源）。CHG-009 事实 22 那条纪律：两处各除一次或
///   都忘了除，出的就是"选中的是上面那行、复制的是下面那行"。
/// - **一个点也算命中**（拖出来的矩形退化成一个点时），所以单击能选中脚下那块，
///   而点在空白处得到空表——空表是"取消选中"，不是"什么都没发生"。
/// - 只认 `enabled` 且非空白的块：识别没出的块 SHALL NOT 选得中（不许给空字符串当选中）。
pub fn hit_blocks(document: &ice_core::document::OcrDocument, image: Rect, drag: Rect) -> Vec<u32> {
    let mut ids: Vec<u32> = document
        .blocks
        .iter()
        .filter(|block| block.enabled && !block.text.trim().is_empty())
        .filter(|block| block_rect(document, image, block.bbox).intersects(drag))
        .map(|block| block.id)
        .collect();
    ids.sort();
    ids
}

/// 一个识别块此刻在屏上占哪一块（点坐标）。**命中判定与高亮绘制读同一个换算**，不各算一遍。
///
/// 比值由 `image`（点）与 `document.width`（像素）现算。CHG-009 事实 22 那条纪律：两处各除
/// 一次或都忘了除，出的就是"选中的是上面那行、复制出去的是下面那行"。
pub fn block_rect(
    document: &ice_core::document::OcrDocument,
    image: Rect,
    bbox: ice_core::document::PixelRect,
) -> Rect {
    if document.width == 0 || image.width() <= 0.0 {
        return Rect::NOTHING;
    }
    let scale = f64::from(document.width) / f64::from(image.width());
    let point = |x: f32, y: f32| {
        image.min + Vec2::new((f64::from(x) / scale) as f32, (f64::from(y) / scale) as f32)
    };
    Rect::from_min_max(
        point(bbox.x, bbox.y),
        point(bbox.x + bbox.w, bbox.y + bbox.h),
    )
}

/// 拖选要不要接管这一帧的鼠标。**不满足就一条都不接管**：
/// 裸按下图片仍然是"拖这扇窗"（D-29 的修法），文字层还没长出来时也不许出现第二种手势。
pub fn selection_gesture(text_ready: bool, shift: bool, primary_pressed: bool) -> bool {
    text_ready && shift && primary_pressed
}

/// 拖选期间这一帧该画的那个矩形：按下点与当前指针点的并。松手之前它只是**读数**，
/// 选中集合要等 [`hit_blocks`] 在抬起那一刻算一次——每帧都算会让"擦过"的块全进选中。
pub fn drag_rect(from: Pos2, to: Pos2) -> Rect {
    Rect::from_two_pos(from, to)
}

/// 右键菜单：画在图片本体上，**恰好四项**，与带子里那四颗同一张表（[`ACTIONS`]）。
///
/// 项数由那一张表一处决定；`exactly_four_actions_shared_by_the_band_and_the_menu` 钉住
/// "只要四项"这句原话——加第五项要改表，而断言会先响。
pub fn image_menu(ui: &mut Ui, sink: &mut Vec<PinIntent>) {
    for action in ACTIONS {
        if ui.button(action.menu_label()).clicked() {
            sink.push(PinIntent::from_action(action));
            ui.close();
        }
    }
}

/// 一张贴图在 egui 里的"活着"证据：纹理 + 像素尺寸 + 是哪一个来源。
///
/// 存进 `ctx.data` 而不是 `AppState`：状态层 SHALL NOT 拿着一份解好码的图（那是几 MB 到
/// 几十 MB 的像素，而 `AppState` 要能在无头里构造与断言）。贴图内容归本模块缓存，
/// 状态层只记"有这张、带子开着、链走到哪一步了"。
#[derive(Clone)]
struct Visual {
    texture: egui::TextureHandle,
    /// PNG 的像素尺寸（点由它除以 ppp 得到，见 [`pin_layout`]）
    px: Vec2,
    /// 缓存的是哪一格：0=原图文件、1=识别带回来的底图、2=成品。换了格就要重建纹理
    kind: u8,
    /// `kind == 0` 时是哪一个文件（换文件要重建）
    source: PathBuf,
}

/// 这张贴图**此刻屏上显示的是哪一份像素**。
///
/// 顺序是死的：成品优先（原地替换之后屏上就是它），其次识别带回的底图，最后才读文件。
/// 复制与保存 MUST 用同一份——一边显示译文版、一边把原图交出去，用户下一步粘到别处的
/// 东西和他刚看到的对不上，那比报错更糟。
pub fn displayed(pin: &crate::state::Pin) -> Option<Display<'_>> {
    if let Some(product) = pin.product.as_ref() {
        return Some(Display::Owned(product.image.clone()));
    }
    if let Some(admitted) = pin.source.as_ref() {
        return Some(Display::Borrowed(&admitted.image));
    }
    ice_vision::import::admit_path(&pin.path)
        .map(|admitted| Display::Owned(admitted.image))
        .ok()
}

/// 屏上那一份像素的两种来路：贴图自己的（借）或现读文件换来的（有）。
pub enum Display<'a> {
    Borrowed(&'a image::RgbaImage),
    Owned(image::RgbaImage),
}

impl Display<'_> {
    pub fn image(&self) -> &image::RgbaImage {
        match self {
            Display::Borrowed(image) => image,
            Display::Owned(image) => image,
        }
    }
}

/// 这张贴图的视口号。**唯一一处构造点**：`show` 用它、发起截图前收起窗也用它——
/// 两边各算一次 hash，一旦算法改动就会"收起了另一扇窗"，而那在测试里看不出来
pub fn viewport_id(id: u64) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(Id::NULL.with("ice-translator.pin").with(id))
}

/// 这扇贴图原生窗的标题：带号，好让 `system::attach` 按标题把它和**它自己的**药丸配成对。
/// 无边框窗标题不上屏，只给 AppKit 认。
pub fn window_title(id: u64) -> String {
    format!("贴图 {id}")
}

fn visual_key(id: u64) -> Id {
    Id::NULL.with("ice-translator.pin.visual").with(id)
}

/// 拖选起点（点坐标，本窗内）。存在这个窗自己的 egui 数据里，**不进 `AppState`**：
/// 它是一帧级的中间量，抬起那一瞬就换成 `Pin::selected`，留着它等于让状态层多一份会过期的坐标。
fn sel_start_key(id: u64) -> Id {
    Id::NULL.with("ice-translator.pin.sel_start").with(id)
}

fn size_key(id: u64) -> Id {
    Id::NULL.with("ice-translator.pin.window_size").with(id)
}

/// 建/取这一帧要用的纹理。同一格（同来源、同文件）只解一次码。
///
/// 解不开就返回 `None`：这一帧**什么都不画**，而不是画一块白——白块会被当成"图坏过"，
/// 而贴图的存在与否由状态层说了算，这里画不出来也不该把那张从状态里抹掉。
fn visual_of(ctx: &egui::Context, pin: &crate::state::Pin) -> Option<Visual> {
    // 换了来源（原图 → 底图 → 成品）就要重建纹理，所以来源本身是缓存键的一部分
    let kind = if pin.product.is_some() {
        2
    } else if pin.source.is_some() {
        1
    } else {
        0
    };
    let cached = ctx.data_mut(|d| d.get_temp::<Visual>(visual_key(pin.id)));
    if let Some(visual) = cached {
        if visual.kind == kind && visual.source == pin.path {
            return Some(visual);
        }
    }
    let image = displayed(pin)?.image().clone();
    let px = Vec2::new(image.width() as f32, image.height() as f32);
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    let texture = ctx.load_texture(
        format!("ice-translator-pin-{}-{kind}", pin.id),
        color,
        // LINEAR：这张 PNG 是物理像素，落在 2 倍屏上要按点缩小显示，NEAREST 会把文字
        // 边缘啃出锯齿；1:1 那一档两种取样看不出差别，所以不必为它分两套路径
        egui::TextureOptions::LINEAR,
    );
    let visual = Visual {
        texture,
        px,
        kind,
        source: pin.path.clone(),
    };
    ctx.data_mut(|d| d.insert_temp(visual_key(pin.id), visual.clone()));
    Some(visual)
}

fn intent_key(id: u64) -> Id {
    Id::NULL.with("ice-translator.pin.intents").with(id)
}

fn intents_slot(ctx: &egui::Context, id: u64) -> Arc<Mutex<Vec<PinIntent>>> {
    ctx.data_mut(|d| {
        if let Some(existing) = d.get_temp::<Arc<Mutex<Vec<PinIntent>>>>(intent_key(id)) {
            return existing;
        }
        let fresh = Arc::new(Mutex::new(Vec::new()));
        d.insert_temp(intent_key(id), fresh.clone());
        fresh
    })
}

/// 取走这张贴图攒下的意图（带子按钮、右键菜单、键盘都走这一条）。
///
/// **为什么不是"画完当场就交出来"**：`show_viewport_deferred` 只是把回调登记下来，真正
/// 执行是在**那个窗口自己那一帧**（egui 把回调交给平台后端按视口调度）。所以在主窗这一帧
/// 里直接读回调里的本地变量必然读不到东西——症状会是"点了按钮要等下一次动鼠标才生效"。
/// 槽放在主 `Context` 的 `data` 里，主窗下一帧取，晚一帧（约 16ms）而次序不会乱。
pub fn take_intents(ctx: &egui::Context, id: u64) -> Vec<PinIntent> {
    let Some(slot) = ctx.data_mut(|d| d.get_temp::<Arc<Mutex<Vec<PinIntent>>>>(intent_key(id)))
    else {
        return Vec::new();
    };
    match slot.lock() {
        Ok(mut queue) => std::mem::take(&mut *queue),
        // 锁只在有人 poison 时失败，而那意味着别处已经 panic 过：这里不二次 panic，
        // 当成"没有意图"处理，界面上这一帧什么都不做，下一帧照旧
        Err(_) => Vec::new(),
    }
}

/// 把一张贴图**这一帧**画出来：`show_viewport_deferred` 是每帧的 show 调用。
///
/// 只在第一帧调一次的写法实测会翻车（事实 25）：窗口真的建出来了，下一帧末尾就被 egui
/// 以 `it was never used this pass` 删掉——症状是"贴图闪一下就没了"。所以调用方 SHALL
/// 每帧为集合里每一张活着的贴图各调一次本函数，关闭的实现是**那一帧起不再调它**。
///
/// 本函数只画**图片那一块**（带子已换成另一扇药丸窗，见 `system/pill.rs`）。贴图窗恒等于
/// 图片，`window_size` 只随 ppp 变（挪到另一块屏那一刻自纠），不再随收放变。带子展开时
/// 这一帧的回调顺带把自己所在屏的位置推给药丸窗（`pill::follow`，**不绕主窗**）。
///
/// 首张落在哪儿：**由状态层给**（`pin.at` = 确认那一刻选区的全局左上角），本模块不猜、
/// 也不攒坐标。给不出（「打开上一次」、满了之后 F3 补钉那两张）就照旧让系统按默认位摆，
/// 用户靠拖动定位（实测拖得动、位置读 `outer_rect` 跟得上，事实 29/32）。
///
/// 这一格兑现的是 CHG-009 REQ-001 那句"选完**原地**钉成一张浮在屏幕上的图"：系统框选那个
/// 年代拿不到那块矩形（`screencapture -i` 只交回一个文件），当时只能交给默认位；
/// 自绘遮罩（CHG-010）交得出矩形，就该钉回用户框住的那一块。
/// 每帧传的仍是同一个值——egui 只在 builder 的 `position` **变了**的时候才发位置命令，
/// 所以用户拖动之后窗不会被弹回原地（攒一份"我们以为它在哪"的坐标仍然是禁止的）。
pub fn show(ctx: &egui::Context, pin: &crate::state::Pin, hint: Option<bool>) {
    let id = pin.id;
    let band_open = pin.band_open;
    // 这一帧这张的药丸带不带首用提示（REQ-005）：只影响药丸窗按多大的可见物体去量翻转/落点。
    // 提示本体画在药丸窗里（`pill::show`），贴图窗自己恒等于图片，不受它影响。
    let hint_present = hint.is_some();
    let Some(visual) = visual_of(ctx, pin) else {
        return;
    };
    // ppp 取当前这一帧的：挪到另一块屏上时会跟着变，下一帧的 `InnerSize` 就自纠
    let scale = ctx.pixels_per_point();
    let layout = pin_layout(visual.px, scale);
    let previous = ctx.data_mut(|d| d.get_temp::<Vec2>(size_key(id)));
    let resize = inner_size_command(previous, layout.window_size);

    let viewport = viewport_id(id);
    let mut builder = egui::ViewportBuilder::default()
        .with_inner_size(layout.window_size)
        .with_decorations(false)
        .with_transparent(false)
        .with_has_shadow(false)
        .with_always_on_top()
        // **不开 `movable_by_background`**。它看着是"实测能拖"的那条路，但无边框窗的整片
        // 内容都算背景：每一次按下都被 AppKit 当成拖窗口吃掉，于是带子按钮收不到点击、
        // 右键菜单弹不出来，窗口也从来没成为 key window——键盘事件（esc、空格）同样进不来。
        // 用户报的"三个关法都不管用"就是这一个原因。拖动改按下面那一处 `StartDrag`。
        // 标题带上这张的号：原生子窗绑定（`system::attach`）要按标题把"这扇贴图"和"它自己的药丸"配成对。
        .with_title(window_title(id));
    if let Some(at) = pin.at {
        builder = builder.with_position(at);
    }

    let texture = visual.texture;
    let root = ctx.clone();
    let slot = intents_slot(ctx, id);
    // 闭包要 `'static`，所以这一帧的三份读数**按值**带走：文档、选中集合、文字层是否长出来。
    // 带 `&Pin` 进去是编译不过的，而带一份 `Arc<Pin>` 进去等于让状态层多一条生命周期——
    // 下一帧重新取，本来就该是这一帧的快照。
    let document = pin.document.clone();
    let selected = pin.selected.clone();
    let text_ready = pin.text_ready();

    ctx.show_viewport_deferred(viewport, builder, move |ui, _class| {
        let child = ui.ctx().clone();
        // 键盘只落在有焦点的那一张上（事实 27）：子 viewport 里的 `input` 就是这个窗的事件
        let fired: Vec<PinIntent> = child.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => key_intent(*key, *modifiers),
                    _ => None,
                })
                .collect()
        });
        if !fired.is_empty() {
            if let Ok(mut queue) = slot.lock() {
                queue.extend(fired);
            }
            // 意图是主窗下一帧要取的：**回调里必须点名叫主窗那一扇视口**。
            // 先前这里写的是 `root.request_repaint()`——egui 按当前视口 id 派发，那句只会
            // 重画这张贴图自己，主窗一次都不醒。真机读数就是用户报的那句"点关闭要么没反应、
            // 要么二十来秒才关上"：那二十秒是主窗下一次恰好重画的时刻，不是关闭慢。
            wake_root(&root);
        }
        // 收放带子要改的是**窗口本身**的尺寸（事实 26），只在真的变了那一刻发一次
        if let Some(size) = resize {
            child.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            root.data_mut(|d| d.insert_temp(size_key(id), size));
        } else if previous.is_none() {
            root.data_mut(|d| d.insert_temp(size_key(id), layout.window_size));
        }

        // 这一帧这张攒下的意图（带子、菜单、键盘三处都往这儿放，最后一起交给交接槽）
        let mut local: Vec<PinIntent> = Vec::new();

        // 图片本体：先占位（光标停在图片这一块，整窗就是它），再画
        let area = image_area(ui, layout.image_rect.size());
        ui.painter().image(
            texture.id(),
            layout.image_rect,
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // 拖动：只在**图片本体上按下左键**的那一下交给系统去拖窗口。
        // `StartDrag` 的约束是"必须紧接在左键按下之后"，所以判据用按下而不是抬起；
        // 带子那几颗不在这块矩形里，它们的按下不会被抢走。
        // `contains_pointer()` 是这一帧这块矩形上有没有指针（egui 自己算好的），
        // 再配上当前的左键按下——两个条件同时成立才把这一次按下交给系统去拖窗口
        // 按下那一下归谁：**默认仍是拖这扇窗**（D-29 的修法不许回退），只有"文字层已长出
        // 来 + 按住 Shift"才改由本模块接管成拖选。裸按下就变成选字的话，用户昨天报的
        // "关不掉/点不动"那一族会以另一种形式回来，而且这一次连拖动都没了。
        let pressed = area.contains_pointer()
            && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let shift = ui.input(|i| i.modifiers.shift);
        let start = child.data(|d| d.get_temp::<Pos2>(sel_start_key(id)));
        if selection_gesture(text_ready, shift, pressed) {
            if let Some(at) = ui.input(|i| i.pointer.interact_pos()) {
                child.data_mut(|d| d.insert_temp(sel_start_key(id), at));
            }
        } else if pressed {
            child.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        // 文字层：选中那几块各涂一层淡高亮，正在拖的那一下画个白框。
        // **绘制与选中集合是两件事**：高亮读的是状态层那份（上一帧抬起时算出来的），
        // 拖框只是这一帧的读数——每帧都算命中会让"擦过去"的块全进选中。
        if let Some(document) = document.as_ref() {
            for block in &document.blocks {
                if selected.contains(&block.id) {
                    let rect = block_rect(document, layout.image_rect, block.bbox);
                    ui.painter().rect_filled(rect, 0.0, select_tint());
                    ui.painter()
                        .rect_stroke(rect, 0.0, select_edge(), egui::StrokeKind::Inside);
                }
            }
            let pointer = ui.input(|i| i.pointer.interact_pos());
            if let (Some(from), Some(to)) = (start, pointer) {
                let rect = drag_rect(from, to);
                if !start.map(|p| p == to).unwrap_or(false) {
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(1.0, egui::Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            // 抬起那一下才算一次命中（**抬起在图片外也算**——拖出去松手是常态）
            let released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
            if released {
                if let (Some(from), Some(to)) = (start, pointer) {
                    let ids = hit_blocks(document, layout.image_rect, drag_rect(from, to));
                    local.push(PinIntent::Select(ids));
                }
                if start.is_some() {
                    child.data_mut(|d| d.remove::<Pos2>(sel_start_key(id)));
                }
            }
        } else if let Some(from) = start {
            // 文档没了（识别失败、或这张换成了成品又重识别中）：把手上那个起点擦掉，
            // 不许留一个"下一次按下接着上一次"的幽灵拖框
            let _ = from;
            child.data_mut(|d| d.remove::<Pos2>(sel_start_key(id)));
        }
        // 右键：恰好四项，与药丸同一张表
        area.context_menu(|ui| image_menu(ui, &mut local));

        // 文字层那句手势说明留在**图片本体**的悬停里（REQ-002：不进药丸）
        area.on_hover_text(TEXT_LAYER_TIP);

        // 带子展开、且这扇药丸**还没被系统绑成子窗**时：把自己这一帧的 `outer_rect` 现推给
        // 药丸窗（不绕主窗、只在真变了那一刻发）。绑上之后位置由系统按固定偏移带着走，这里不再推。
        if band_open && !pill::is_attached(&root, id) {
            if let (Some(screen), Some(outer)) = (
                pill::screen_of(&root),
                child.input(|i| i.viewport().outer_rect),
            ) {
                pill::follow(&root, id, outer, screen, hint_present);
            }
        }
        if !local.is_empty() {
            if let Ok(mut queue) = slot.lock() {
                queue.extend(local);
            }
            // 意图是主窗下一帧要取的：**推完必须点名叫主窗那一扇视口**（同一条纪律的第三处，
            // 理由见上面键盘那一处：`request_repaint()` 在子窗闭包里只叫得醒子窗自己）
            wake_root(&root);
        }
    });
}

/// 保存面板的默认名：`贴图-N.png`，N 是这张贴图的号。
///
/// 这里刻意不做日期换算，也不引日期库（V-006 零依赖增量）：我试过一次手写公历换算，
/// 结果在 1900 这个非闰年上算出了一个不存在的 `2 月 29 日`——为一个默认文件名养一份
/// 自己错的日历不值得。号只往前走不复用，同一批里就不撞名。
pub fn default_save_name(id: u64) -> String {
    format!("\u{8d34}\u{56fe}-{id}.png")
}

/// 悬停在那句进度上时给的那句手势说明。
const TEXT_LAYER_TIP: &str = "按住 Shift 在字上拖可多选几行；在空白处拖一下则取消选中";

/// 测试夹具：一块 1000×400 像素的文档，上下两行可读文字（id 0 与 1）。
/// 贴图文字层的用例（本模块与 `app.rs`）都从这里造，不各写一份字段。
#[cfg(test)]
pub fn probe_document() -> ice_core::document::OcrDocument {
    use ice_core::document::{OcrDocument, PixelRect, TextBlock};
    let mut document = OcrDocument::empty(1000, 400);
    document.blocks = vec![
        TextBlock::new(
            0,
            "上面那一行",
            PixelRect::new(10.0, 10.0, 400.0, 30.0),
            0.9,
        ),
        TextBlock::new(
            1,
            "下面那一行",
            PixelRect::new(10.0, 200.0, 400.0, 30.0),
            0.9,
        ),
    ];
    document
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::implementation_lines;

    fn px(w: f32, h: f32) -> Vec2 {
        Vec2::new(w, h)
    }

    fn at(x: f32, y: f32) -> Pos2 {
        Pos2::new(x, y)
    }

    /// 贴图窗**恒等于图片那一块**——操作条改成了另一扇浮空药丸窗（CHG-011），窗口本身不再
    /// 随收放变高。这一条**替换**了旧的两条编码"带子是窗口一部分"的断言：契约换了，不是迁就实现。
    #[test]
    fn the_pin_window_is_always_the_image_itself() {
        let l = pin_layout(px(512.0, 360.0), 1.0);
        assert_eq!(
            l.window_size,
            px(512.0, 360.0),
            "窗口 = 图片，不多出一条带子"
        );
        assert_eq!(l.image_rect.max, at(512.0, 360.0));
        // 死区被结构性消除：贴图窗里再也没有"画了却不属于图片"的那一块
        assert_eq!(l.window_size, l.image_rect.size(), "窗与图同一块");
    }

    /// 收放带子不再改贴图窗尺寸：开合只决定那扇药丸窗在不在，`window_size` 恒等图片。
    /// （旧的那条"收起=缩窗口"断言因此作废——现在根本没有会缩的那段，`pin_layout` 也不再收 `band_open`。）
    #[test]
    fn band_toggle_does_not_resize_the_pin_window() {
        let l = pin_layout(px(512.0, 360.0), 1.0);
        assert_eq!(l.window_size.y, 360.0, "不再有 BAND_H 那一段");
    }

    /// `scale = 2.0` 那一档：**点边长是像素的一半**，而且只在这一处除。
    /// 这条**不声称证明了真机行为**（H9 要一块 HiDPI 屏，归 W-5.7），只证明换算只有一个出处。
    #[test]
    fn the_pixel_to_point_division_happens_exactly_once() {
        let l = pin_layout(px(1000.0, 800.0), 2.0);
        assert_eq!(
            l.image_rect.max,
            at(500.0, 400.0),
            "2 倍屏上贴图不该有一倍大"
        );
        assert_eq!(
            l.window_size,
            px(500.0, 400.0),
            "窗口 = 换算后的图片，不再多一段带子（那一段是点也曾经不再除一次）"
        );
    }

    /// ppp 拿不到时按 1.0 兜：宁可 1:1，也不能算出 0 尺寸或负尺寸的窗口
    #[test]
    fn a_missing_scale_falls_back_to_one_not_to_zero() {
        for bad in [0.0, -1.0, -0.0] {
            let l = pin_layout(px(200.0, 100.0), bad);
            assert_eq!(l.window_size, px(200.0, 100.0), "ppp={bad}");
            assert!(l.window_size.x > 0.0 && l.window_size.y > 0.0);
        }
    }

    /// **右键菜单与带子读同一张表，而且恰好四项**（用户明说"只要四项"，
    /// 加第五项要改这一处，而这条断言先响）。
    #[test]
    fn exactly_four_actions_shared_by_the_band_and_the_menu() {
        assert_eq!(ACTIONS.len(), 4, "带子与菜单的动作表被改长了");
        let labels: Vec<&str> = ACTIONS.iter().map(|a| a.label()).collect();
        let menus: Vec<&str> = ACTIONS.iter().map(|a| a.menu_label()).collect();
        for l in &labels {
            assert!(
                labels.iter().filter(|x| *x == l).count() == 1,
                "带子上有两颗同名：{l}"
            );
        }
        for m in &menus {
            assert!(
                menus.iter().filter(|x| *x == m).count() == 1,
                "菜单里有两项同名：{m}"
            );
        }
        // 四件事各归一件，不许有第五件（某截图工具 那种十几项不是他要的）
        assert_eq!(
            labels,
            vec!["翻译", "复制", "保存", "关"],
            "带子上的字改了要顺着文案四条线一起改（docs/05 §6）"
        );
    }

    /// 「关」只是这张贴图的事：它的名字里不许混进"退出应用"那种含义。
    #[test]
    fn the_close_action_is_only_about_this_pin() {
        assert_eq!(PinAction::Close.menu_label(), "关闭这张贴图");
        assert!(!PinAction::Close.menu_label().contains("退出"));
    }

    /// 窗口尺寸随图片尺寸线性走，且**左上角永远是原点**：位置不由这里算，
    /// 只能从 `ViewportInfo::outer_rect` 读（本模块出现第二份坐标累计就是缺陷）。
    #[test]
    fn the_layout_never_invents_a_position() {
        for (w, h) in [(100.0, 50.0), (512.0, 360.0), (1920.0, 1080.0)] {
            let l = pin_layout(px(w, h), 1.0);
            assert_eq!(l.image_rect.min, Pos2::ZERO, "几何里不许出现第二个坐标源");
            assert_eq!(l.window_size, px(w, h));
        }
    }

    /// **意图矩阵**：每颗键只能落到那一种意图上，多一颗少一颗都算改坏了形态。
    #[test]
    fn the_key_matrix_maps_each_key_to_exactly_one_intent() {
        let none = Modifiers::NONE;
        assert_eq!(key_intent(Key::Escape, none), Some(PinIntent::Close));
        assert_eq!(key_intent(Key::Space, none), Some(PinIntent::ToggleBand));
        // esc 两段式（先关带子再关窗）不许出现：收起状态下的 esc 还是同一个意图
        assert_eq!(key_intent(Key::Escape, none), Some(PinIntent::Close));
        // 复制图像带修饰键（mac 是 ⌘，其他平台是 Ctrl）
        let cmd = Modifiers {
            command: true,
            ..Modifiers::NONE
        };
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        assert_eq!(
            key_intent(Key::C, if cfg!(target_os = "macos") { cmd } else { ctrl }),
            Some(PinIntent::CopyImage)
        );
        // 裸 C 给的是 **Copy**（那一条分派），不是 CopyImage：图还是字要看有没有选中（REQ-005）
        assert_eq!(key_intent(Key::C, none), Some(PinIntent::Copy));
        // Shift+Shift 之外的组合键（⌥C 一类）不抢：留给别人，也免得与输入法打架
        assert_eq!(
            key_intent(
                Key::C,
                Modifiers {
                    shift: true,
                    ..Modifiers::NONE
                }
            ),
            None
        );
        // 输入法/Alt 组合一律不抢
        assert_eq!(
            key_intent(
                Key::C,
                Modifiers {
                    command: true,
                    alt: true,
                    ..Modifiers::NONE
                }
            ),
            None
        );
        assert_eq!(key_intent(Key::A, none), None);
    }

    /// 带子四颗与右键菜单四项**发same的意图**：一条 `from_action` 映射，两处不各写一遍。
    /// 带子三颗、右键四项：**只差「关闭」这一颗**，且带子上那颗必须没有。
    /// 这条钉的是用户那句"不需要在界面上搞一个关闭的按钮，右键的可以留下"。
    #[test]
    fn the_band_drops_only_the_close_action() {
        assert_eq!(BAND_ACTIONS.len(), 3);
        assert!(
            !BAND_ACTIONS.contains(&PinAction::Close),
            "带子上不许有「关闭」"
        );
        assert_eq!(ACTIONS.len(), 4, "右键菜单仍要四项齐全");
        let missing: Vec<PinAction> = ACTIONS
            .iter()
            .copied()
            .filter(|a| !BAND_ACTIONS.contains(a))
            .collect();
        assert_eq!(missing, vec![PinAction::Close]);
        // 两处仍读同一份 from_action 映射，不各写一套
        for action in ACTIONS {
            let _ = PinIntent::from_action(action);
        }
    }

    #[test]
    fn the_band_and_the_menu_emit_the_same_four_intents() {
        let from_band: Vec<PinIntent> = ACTIONS
            .iter()
            .copied()
            .map(PinIntent::from_action)
            .collect();
        assert_eq!(
            from_band,
            vec![
                PinIntent::Translate,
                PinIntent::CopyImage,
                PinIntent::SaveImage,
                PinIntent::Close
            ]
        );
        // ToggleBand 只能来自空格，带子上没有第五颗按钮能干这件事
        assert!(!from_band.contains(&PinIntent::ToggleBand));
    }

    /// 尺寸命令只在真的变了那一刻发：收起那一下发，之后每帧不再发。
    /// 每帧都发会搅出一堆无谓 resize，也会把"用户刚拖到哪儿"这件事搅浑。
    #[test]
    fn the_resize_command_fires_once_per_change() {
        let open = px(512.0, 394.0);
        let closed = px(512.0, 360.0);
        assert_eq!(
            inner_size_command(None, open),
            Some(open),
            "创建之后第一帧要报到"
        );
        assert_eq!(inner_size_command(Some(open), open), None);
        assert_eq!(inner_size_command(Some(open), closed), Some(closed));
        assert_eq!(inner_size_command(Some(closed), closed), None);
        // 亚像素的浮点抖动不发命令
        assert_eq!(
            inner_size_command(Some(open), px(512.0, 394.0 + 0.2)),
            None,
            "0.2 点的抖动不该发一次 resize"
        );
    }

    /// 跑一帧并把纹理增量吃掉。
    ///
    /// 真宿主（eframe）每帧都消费 `FullOutput.textures`；测试里不管它就会在 drop 时 panic
    /// （`Dropped TexturesDelta with N unapplied deltas`）——那是测试的锅，不是贴图的锅。
    fn pass(ctx: &egui::Context, input: egui::RawInput, draw: impl FnMut(&mut egui::Ui)) {
        ctx.run_ui(input, draw).drop_without_applying_deltas();
    }

    /// 端到端走一遍真 `show`：**esc 按下要变成一条 `Close` 意图**。
    ///
    /// 只测 `key_intent` 那条纯映射不够——它证明不了"事件真的到了这张贴图那一帧"。
    /// 这里用真实 `Context`（embed 模式下 deferred 回调会在同一帧内跑）喂一条按键事件，
    /// 看的是 `take_intents` 取回来什么。
    #[test]
    fn a_real_pass_turns_escape_into_a_close_intent() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("pin.png");
        image::RgbImage::from_pixel(64, 40, image::Rgb([9, 9, 9]))
            .save(&png)
            .unwrap();

        let ctx = egui::Context::default();
        // eframe 在支持的平台上会关掉它；默认是 true，这里显式写出来
        // 是为了让这三条用例在任何宿主下都走同一条 embed 分支
        ctx.set_embed_viewports(true);

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        pass(&ctx, input, |ui| {
            show(ui.ctx(), &crate::state::Pin::for_test(1, png.clone()), None);
        });

        assert_eq!(take_intents(&ctx, 1), vec![PinIntent::Close]);
        // 取过一次就该空：同一条 esc 不许被处理两遍
        assert!(take_intents(&ctx, 1).is_empty());
    }

    /// 空格那一格走的是同一条路，但落点是 `ToggleBand`——矩阵在真帧里也得对得上。
    #[test]
    fn a_real_pass_turns_space_into_a_band_toggle() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("pin2.png");
        image::RgbImage::from_pixel(30, 20, image::Rgb([1, 2, 3]))
            .save(&png)
            .unwrap();
        let ctx = egui::Context::default();
        // eframe 在支持的平台上会关掉它；默认是 true，这里显式写出来
        // 是为了让这三条用例在任何宿主下都走同一条 embed 分支
        ctx.set_embed_viewports(true);

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        pass(&ctx, input, |ui| {
            let mut pin = crate::state::Pin::for_test(2, png.clone());
            pin.band_open = false;
            show(ui.ctx(), &pin, None);
        });
        assert_eq!(take_intents(&ctx, 2), vec![PinIntent::ToggleBand]);
    }

    /// 文件解不开（被删了、坏了）：**这一帧什么都不画，也不 panic**，更不把那张从状态里抹掉。
    #[test]
    fn an_unreadable_file_paints_nothing_instead_of_a_white_block() {
        let ctx = egui::Context::default();
        // eframe 在支持的平台上会关掉它；默认是 true，这里显式写出来
        // 是为了让这三条用例在任何宿主下都走同一条 embed 分支
        ctx.set_embed_viewports(true);
        let missing = std::path::PathBuf::from("/tmp/definitely-not-a-screenshot.png");
        pass(&ctx, egui::RawInput::default(), |ui| {
            show(
                ui.ctx(),
                &crate::state::Pin::for_test(3, missing.clone()),
                None,
            );
        });
        assert!(take_intents(&ctx, 3).is_empty());
    }

    /// 保存面板的默认名**不许用历史目录里那个内部编号**当样子（那是我们的存储结构），
    /// 也不能带用户可能截到的任何内容。
    #[test]
    fn the_save_name_is_a_readable_name_not_our_internal_id() {
        let name = default_save_name(7);
        assert!(name.ends_with("-7.png"), "默认名该带这张的号：{name}");
        assert!(
            name.starts_with("\u{8d34}\u{56fe}"),
            "默认名该以「贴图」开头：{name}"
        );
        // 不带历史目录里那个内部编号形态（pid-自增-epoch），也不带文件名分隔符歧义
        assert!(!name.contains("1730000000"), "把 epoch 当名字了");
        assert_ne!(name, default_save_name(8), "两张的默认名不该撞");
    }

    /// **形状核查：只有一条拖动路，而且不自己攒坐标。**
    ///
    /// 这一条改过一次，理由是实测：先前用的是 `with_movable_by_background(true)`，
    /// 无边框窗整片内容都算"背景"，于是每一次按下都被 AppKit 当成拖窗口吃掉——带子按钮
    /// 收不到点击、右键菜单弹不出来、窗口从没成为 key window 所以 esc 与空格也进不来
    /// （用户报的"三个关法都不管用"就是这一个原因）。现在换成图片本体按下时发 `StartDrag`，
    /// 只有一条路：另一条不许同时存在，位置也不许自己算（只从 `outer_rect` 读）。
    #[test]
    fn only_one_drag_path_exists_in_this_module() {
        let body = implementation_lines(include_str!("pin.rs"));
        assert!(
            !body.contains("movable_by_background"),
            "背景拖动不许回来：它会吞掉这一扇窗的所有点击与键盘"
        );
        assert_eq!(
            body.matches("ViewportCommand::StartDrag").count(),
            1,
            "拖动只许有一处发起点"
        );
        // 位置不许有第二个来源：本模块既不发 OuterPosition，也不存坐标字段
        assert!(
            !body.contains("OuterPosition"),
            "不许由我们往窗口发位置命令：位置只从 outer_rect 读"
        );
    }

    /// **形状核查：推意图的两处出口都必须点名叫主窗那一扇视口。**
    ///
    /// `ctx.request_repaint()` 在子窗闭包里按"当前视口 id"派发，只会重画这张贴图自己；
    /// 主窗不醒，槽里那份意图就一直没人取。真机上的表现是用户报的那句
    /// "点关闭要么没反应，要么二十来秒才关上"——二十秒是主窗下一次恰好重画的时刻。
    #[test]
    fn intents_are_delivered_by_waking_the_root_viewport() {
        let body = implementation_lines(include_str!("pin.rs"));
        assert!(
            !body.contains("root.request_repaint()"),
            "拿主窗的 ctx 调 request_repaint 只会叫醒当前这扇子窗，是个假动作"
        );
        assert_eq!(
            body.matches("wake_root(&root)").count(),
            2,
            "键盘与带子按钮两个推意图的出口，一个不多一个不少"
        );
    }

    /// **形状核查：一份清单。** 药丸那一排与右键菜单两处都只许遍历那张 `*_ACTIONS`，
    /// 不许各自再写一遍按钮名——那句"只要四项"要被钉住，不是被打折理解。
    #[test]
    fn both_the_band_and_the_menu_iterate_the_one_table() {
        let body = implementation_lines(include_str!("pin.rs"));
        for name in ["pill_buttons", "image_menu"] {
            let start = body
                .find(&format!("pub fn {name}"))
                .unwrap_or_else(|| panic!("找不到 {name}"));
            let text =
                &body[start..body[start..].find("\n}").unwrap_or(body.len() - start) + start];
            assert!(
                text.contains("ACTIONS"),
                "{name} 不再遍历 ACTIONS 了，等于私自加了第二份清单"
            );
        }
        // 按钮名只出现在 PinAction 的 match 里（一处），不在两个绘制函数里各写一遍
        assert_eq!(
            body.matches("\"翻译\"").count(),
            1,
            "「翻译」这两个字出现了第二处：文案该由那张表一处给"
        );
    }

    /// 命中判定只认与拖框相交的块：**擦过另一行不算选中**（选中的是上面那行、
    /// 复制出去的是下面那行，是这一格最容易出的错）。
    #[test]
    fn a_drag_only_collects_the_blocks_it_touches() {
        let doc = probe_document();
        let image = Rect::from_min_size(at(0.0, 0.0), px(500.0, 200.0)); // 2 倍：500 点 = 1000 像素
        let one = Rect::from_min_max(at(20.0, 10.0), at(180.0, 20.0));
        assert_eq!(hit_blocks(&doc, image, one), vec![0]);
        let other = Rect::from_min_max(at(20.0, 100.0), at(180.0, 110.0));
        assert_eq!(hit_blocks(&doc, image, other), vec![1]);
        let both = Rect::from_min_max(at(0.0, 0.0), at(300.0, 200.0));
        assert_eq!(hit_blocks(&doc, image, both), vec![0, 1]);
    }

    /// 点在空白处 = **空表**，而空表要被当作"取消选中"写下去（REQ-005：不许给一个
    /// 空字符串当选中，也不许"什么都没发生"地留着上一次的选中）。
    #[test]
    fn a_click_on_blank_space_selects_nothing() {
        let doc = probe_document();
        let image = Rect::from_min_size(at(0.0, 0.0), px(500.0, 200.0));
        let point = Rect::from_min_max(at(300.0, 60.0), at(300.0, 60.0));
        assert_eq!(hit_blocks(&doc, image, point), Vec::<u32>::new());
        // 同一个换算下，点在块上就是选中那块（单击与拖选共用一条判据）
        let on_it = Rect::from_min_max(at(20.0, 10.0), at(20.0, 10.0));
        assert_eq!(hit_blocks(&doc, image, on_it), vec![0]);
    }

    /// 命中换算与绘制换算必须是**同一个**：这里把两处都指到同一个函数上，
    /// 一旦有人再写一份除法，这两条断言就会各说一套。
    #[test]
    fn the_hit_rect_and_the_painted_rect_use_one_conversion() {
        let doc = probe_document();
        let image = Rect::from_min_size(at(0.0, 0.0), px(500.0, 200.0));
        let painted = block_rect(&doc, image, doc.blocks[1].bbox);
        assert_eq!(painted.min, at(5.0, 100.0), "200 像素 ÷ 2 = 100 点");
        assert_eq!(painted.max, at(205.0, 115.0));
        // 点在它的右下角上就该命中这一块（说明命中用的确实是这一块矩形）
        assert_eq!(
            hit_blocks(
                &doc,
                image,
                Rect::from_min_max(at(205.0, 115.0), at(205.0, 115.0))
            ),
            vec![1]
        );
    }

    /// **裸按下仍然是拖这扇窗**（D-29 的修法）：只有"文字层长出来了 + 按住 Shift"
    /// 才把这一帧的鼠标交给拖选。三个条件少一个都不接管。
    #[test]
    fn a_plain_press_still_belongs_to_the_window_drag() {
        assert!(selection_gesture(true, true, true));
        assert!(
            !selection_gesture(true, false, true),
            "没按 Shift 就还是拖窗"
        );
        assert!(
            !selection_gesture(false, true, true),
            "文字层没长出来，Shift 也不该冒出第二种手势"
        );
        assert!(!selection_gesture(true, true, false));
    }

    /// **形状核查：贴图的落点只有一个来源——状态给的那一个。**
    ///
    /// 本模块不许自己算位置（`the_layout_never_invents_a_position` 钉住了 `pin_layout`
    /// 那一半），也不许在拖动之后把窗弹回原地：egui 只在 builder 的 `position` **变了**
    /// 的时候才发位置命令，所以每帧传的都是 `pin.at` 这一个定值，绝不传 `outer_rect` 读回来的值。
    #[test]
    fn the_place_comes_from_the_state_and_never_from_our_own_guess() {
        let body = implementation_lines(include_str!("pin.rs"));
        assert_eq!(
            body.matches("with_position").count(),
            1,
            "建窗那一处，只此一处"
        );
        assert!(
            body.contains("if let Some(at) = pin.at"),
            "落点只认状态给的那一格；没有就交给系统摆"
        );
    }
}
