//! 贴图窗（CHG-009 REQ-003/REQ-004）。
//!
//! **这一格的地基是 2026-11-05 那晚仓外探针量出来的**（事实 17/25/26/27/29/32/33）：
//! 子 viewport 里那张纹供得出来、画的就是我们算的那块几何、无边框子窗收得到键盘且只给
//! 焦点那一张、同时钉 8 张各自出图。所以这个模块不是"猜一个形态再改"，是把读数控成代码。
//!
//! 三条从读数里出来的硬规矩，写在这里而不是散在绘制里：
//! 1. **带子是窗口的一部分**，按钮 SHALL NOT 浮在图片上——用户那句"右下角有漂浮在图片
//!    外侧的翻译按钮"是字面意思，图上压控件就违背它。
//! 2. **收起带子 = 把窗口本身缩到图片那一块**（`ViewportCommand::InnerSize`）。实测过
//!    只画不缩的那一档：那条带子不画东西、不透明（采样 `#0800`）、**还照吃命中**——
//!    一条隐形陷阱。所以"画但不看得见"这种收法禁止出现。
//! 3. **位置只从 `ViewportInfo::outer_rect` 读**，本模块 SHALL NOT 自己累计第二份坐标
//!    （拖动、系统搬窗、隐藏再回来三种情况它都跟着，事实 32）。
//!
//! `scale` 那一处除法是全局唯一出处（像素 → 点），窗口尺寸、带子重算、文字层坐标三处
//! 都读同一个结果；各自再除一次或都忘了除，就是"贴图比用户框的大一倍"那种现场（H9）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::{Id, Key, Modifiers, Pos2, Rect, Response, Sense, Ui, Vec2};

/// 带子的高度：一颗按钮（与 E 区那排同源，同一个字号加同一套留白）加上下各 8 留白。
///
/// 为什么是常量而不是按内容量：收起时窗口要缩到**图片那一块**，展开时窗口是
/// `图片 + BAND_H`——这个差值必须是确定的数，否则"缩窗口"那一刻算出来的高度和实际
/// 摆出来的带子对不上，就会留下几像素的死区（就是第 2 条硬规矩要防的东西）。
pub const BAND_H: f32 = 34.0;

/// 带子里那四颗，**顺序就是显示顺序**。
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

/// 一张贴图该占的窗口尺寸与两块矩形。**这条 REQ 唯一能被无头锁住的部分**。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinLayout {
    /// 窗口内容尺寸（点）。收起时它等于图片那块，展开时多出 `BAND_H`
    pub window_size: Vec2,
    /// 图片画在哪块（点，左上角是窗口原点）
    pub image_rect: Rect,
    /// 带子占的那块。收起时它整块落在窗口之外，宽度为 0——**不是"在里面但没画"**
    pub band_rect: Rect,
}

/// 像素尺寸 + 开合 + 那张屏的 `pixels_per_point` → 窗口几何。
///
/// `image_px` 是**像素**（PNG 读出来的原尺寸），`scale` 是那张贴图所在屏的 ppp。
/// 除法只在这一处发生，产出的三个矩形全是点。
pub fn pin_layout(image_px: Vec2, band_open: bool, scale: f32) -> PinLayout {
    // 唯一一处除法。ppp 拿不到（0 或负）时按 1.0 兜住——那不是"截错了"，
    // 而是这一帧还没有屏信息，宁可 1:1 也不能算出 0 尺寸的窗口
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let size = image_px / scale;
    let image_rect = Rect::from_min_size(Pos2::ZERO, size);
    let window_size = if band_open {
        Vec2::new(size.x, size.y + BAND_H)
    } else {
        size
    };
    let band_rect = if band_open {
        Rect::from_min_size(Pos2::new(0.0, size.y), Vec2::new(size.x, BAND_H))
    } else {
        // 收起：整块挪到窗口之外并压成 0 宽。命中测试因此永远碰不到它（REQ-004 末句
        // "不可见就是不可点，不画灰着的假按钮"）
        Rect::from_min_size(Pos2::new(window_size.x, window_size.y), Vec2::ZERO)
    };
    PinLayout {
        window_size,
        image_rect,
        band_rect,
    }
}

/// 一张贴图能对外发出的**全部意图**。带子里的按钮、右键菜单里的四项、键盘那几颗，
/// 三处都只能落到这五种之一——矩阵一旦分叉，"点按钮有效、按键没反应"这类只对一半的
/// 缺陷就回来了（图钉那次是两条通道，本包同一条纪律：一个枚举、一处映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinIntent {
    Translate,
    CopyImage,
    SaveImage,
    /// 销毁这张贴图的窗口。**不动历史文件**（REQ-004 明写）
    Close,
    /// 空格：收放带子
    ToggleBand,
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
/// - 裸 `C` **这里不给动作**。它到底是"复制图像"还是"复制选中的字"要看那张贴图有没有
///   选中文字（REQ-004 末段 + REQ-005），那个判断依赖文字层状态，由 `app.rs` 那一层按
///   "选没选中"分支处理；这一层只认带修饰的那颗复制键。
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

/// 带子里那一排按钮画进 `ui`，按下的动作推进 `sink`。
///
/// **带子收起时这个函数根本不会被调**（窗口没那块地方），所以这里不需要再判一次开合；
/// 反过来，如果哪天有人给"收起"的贴图也画了这几颗，`band_rect` 的宽度是 0，按钮挤不进
/// 命中区——两种防线都留着。
pub fn band_buttons(ui: &mut Ui, sink: &mut Vec<PinIntent>) {
    ui.horizontal(|ui| {
        for action in ACTIONS {
            // 「关」按下去就是把这张从状态里摘掉，别的什么都不动（历史文件照旧）
            if ui.button(action.label()).clicked() {
                sink.push(PinIntent::from_action(action));
            }
        }
    });
}

/// 图片本体那块的命中意图：**按住就能拖**（REQ-004 第一条）。
/// 图片本体那一块。
///
/// `Sense::click()` 只为了右键菜单吃得着这一下；**左键按住拖动交给系统**
/// （`with_movable_by_background(true)`，事实 29/32 实测能把无边框子窗拖着走，
/// 拖到哪 `outer_rect` 就读到哪）。自己接按下事件算增量坐标是另一条路
/// （`ViewportCommand::StartDrag`，它自己注释就写着"不保证有效，除非紧接左键按下"）——
/// **两条都留一份坐标计算迟早对不上**，所以这里只留一条，另一条不出现。
pub fn image_area(ui: &mut Ui, size: Vec2) -> Response {
    ui.allocate_rect(Rect::from_min_size(Pos2::ZERO, size), Sense::click())
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
pub fn displayed(pin: &crate::state::Pin) -> Option<Display> {
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

fn visual_key(id: u64) -> Id {
    Id::NULL.with("ice-translator.pin.visual").with(id)
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
/// `band_line` 是这一帧带子里那一行进度话（"识别中…" / "翻译中 3/7"），由帧循环从状态里
/// 算好传进来：**贴图不自己去读 `AppState`**，也不留一份全局可变的"当前进度"——
/// 六张贴图各有自己的进度，靠键去查迟早查错对象。
///
/// 首张落在哪儿没有判据：`screencapture` 只交回一个文件，不交回用户框的那块矩形。
/// 这里因此不给 `position`，让系统按默认位摆放，**用户靠拖动定位**（实测拖得动、
/// 位置读 `outer_rect` 跟得上，事实 29/32）。攒一份"我们以为它在哪"的坐标是禁止的。
pub fn show(ctx: &egui::Context, pin: &crate::state::Pin, band_line: Option<String>) {
    let id = pin.id;
    let band_open = pin.band_open;
    let Some(visual) = visual_of(ctx, pin) else {
        return;
    };
    // ppp 取当前这一帧的：挪到另一块屏上时会跟着变，下一帧的 `InnerSize` 就自纠
    let scale = ctx.pixels_per_point();
    let layout = pin_layout(visual.px, band_open, scale);
    let previous = ctx.data_mut(|d| d.get_temp::<Vec2>(size_key(id)));
    let resize = inner_size_command(previous, layout.window_size);

    let viewport = egui::ViewportId::from_hash_of(Id::NULL.with("ice-translator.pin").with(id));
    let builder = egui::ViewportBuilder::default()
        .with_inner_size(layout.window_size)
        .with_decorations(false)
        .with_transparent(false)
        .with_has_shadow(false)
        .with_always_on_top()
        // 拖动这一档实测有效（事实 29/32）：按住无边框子窗的图本体就能拖着走，
        // 拖到的位置 `outer_rect` 读得到。我们自己接住按下事件反而会让系统收不到那一下。
        .with_movable_by_background(true)
        .with_title("贴图");

    let texture = visual.texture;
    let root = ctx.clone();
    let slot = intents_slot(ctx, id);

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
            // 意图是主窗下一帧要取的：**回调里必须叫主窗**（与 Carbon 回调同一条纪律）
            root.request_repaint();
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

        // 图片本体：先占位（光标因此停在图片下缘，下一格拿到的正是 `band_rect`），再画
        let area = image_area(ui, layout.image_rect.size());
        ui.painter().image(
            texture.id(),
            layout.image_rect,
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // 右键：恰好四项，与带子同一张表
        area.context_menu(|ui| image_menu(ui, &mut local));

        if band_open {
            ui.allocate_ui_with_layout(
                Vec2::new(layout.window_size.x, BAND_H),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    band_buttons(ui, &mut local);
                    if let Some(line) = band_line.as_deref() {
                        // 同一行的右半边：这张自己的进度。六张贴图各说各的，不共用一条
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(line);
                        });
                    }
                },
            );
        }
        if !local.is_empty() {
            if let Ok(mut queue) = slot.lock() {
                queue.extend(local);
            }
            // 意图是主窗下一帧要取的：**推完必须叫主窗**（同一条纪律的第三处）
            root.request_repaint();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 把注释行与测试模块之前的部分留下：形状核查要看的是**代码里写了什么**，
    /// 不是注释里讨论了什么（本文件就专门解释过为什么不用另一条拖动路）。
    fn implementation_lines(source: &str) -> String {
        source
            .split("#[cfg(test)]")
            .next()
            .unwrap_or(source)
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn px(w: f32, h: f32) -> Vec2 {
        Vec2::new(w, h)
    }

    fn at(x: f32, y: f32) -> Pos2 {
        Pos2::new(x, y)
    }

    /// 展开：窗口 = 图片 + 一条带子，带子在图片**下面**，图片那块一个像素都没被挤
    #[test]
    fn the_open_band_sits_below_the_image_and_grows_the_window() {
        let l = pin_layout(px(512.0, 360.0), true, 1.0);
        assert_eq!(l.window_size, px(512.0, 360.0 + BAND_H));
        assert_eq!(l.image_rect.max, at(512.0, 360.0));
        assert_eq!(l.band_rect.min, at(0.0, 360.0), "带子必须紧贴图片下缘");
        assert_eq!(l.band_rect.height(), BAND_H);
        assert_eq!(l.band_rect.width(), 512.0, "带子与图片同宽");
        // 两块不许重叠：图上压控件就违背"漂浮在图片外侧"那句原话
        assert!(l.image_rect.max.y <= l.band_rect.min.y);
    }

    /// 收起：**窗口本身**缩到图片那一块，带子整块到窗口之外且宽度为 0。
    /// 这条断言挡的是"画但不看得见"那种收法——实测它会留下一条不吃绘制、照吃命中的死区。
    #[test]
    fn a_closed_band_shrinks_the_window_itself_and_leaves_no_dead_strip() {
        let open = pin_layout(px(512.0, 360.0), true, 1.0);
        let closed = pin_layout(px(512.0, 360.0), false, 1.0);
        assert_eq!(closed.window_size, px(512.0, 360.0), "收起 = 缩到图片本身");
        assert_eq!(open.window_size.y - closed.window_size.y, BAND_H);
        // 图片那块不许因为收带子而挪动或变形（往下收，不把图挪走）
        assert_eq!(closed.image_rect, open.image_rect);
        assert_eq!(closed.band_rect.width(), 0.0);
        assert!(closed.band_rect.min.x >= closed.window_size.x);
        assert!(closed.band_rect.min.y >= closed.window_size.y);
    }

    /// `scale = 2.0` 那一档：**点边长是像素的一半**，而且只在这一处除。
    /// 这条**不声称证明了真机行为**（H9 要一块 HiDPI 屏，归 W-5.7），只证明换算只有一个出处。
    #[test]
    fn the_pixel_to_point_division_happens_exactly_once() {
        let l = pin_layout(px(1000.0, 800.0), true, 2.0);
        assert_eq!(
            l.image_rect.max,
            at(500.0, 400.0),
            "2 倍屏上贴图不该有一倍大"
        );
        assert_eq!(
            l.window_size.y,
            400.0 + BAND_H,
            "带子那一段是点，不再除一次"
        );
        assert_eq!(l.band_rect.min.y, 400.0);
        assert_eq!(l.band_rect.height(), BAND_H);
    }

    /// ppp 拿不到时按 1.0 兜：宁可 1:1，也不能算出 0 尺寸或负尺寸的窗口
    #[test]
    fn a_missing_scale_falls_back_to_one_not_to_zero() {
        for bad in [0.0, -1.0, -0.0] {
            let l = pin_layout(px(200.0, 100.0), true, bad);
            assert_eq!(l.window_size, px(200.0, 100.0 + BAND_H), "ppp={bad}");
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
            let l = pin_layout(px(w, h), true, 1.0);
            assert_eq!(l.image_rect.min, Pos2::ZERO, "几何里不许出现第二个坐标源");
            assert_eq!(l.window_size, px(w, h + BAND_H));
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
        // 裸 C 不在这一层给动作：它是"图还是字"要看有没有选中文字（REQ-005）
        assert_eq!(key_intent(Key::C, none), None);
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
            show(ui.ctx(), &pin, Some("识别中…".to_string()));
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

    /// **形状核查：一条拖动路。** 自己接按下算增量坐标那条（`StartDrag`）不许同时存在——
    /// 两处各攒一份坐标迟早对不上（`outer_rect` 才是位置的唯一出处，事实 32）。
    #[test]
    fn only_one_drag_path_exists_in_this_module() {
        let source = include_str!("pin.rs");
        // 只看代码：本文件的注释里就提到过 StartDrag（说为什么不用它），拿散文当证据会误报
        let body = implementation_lines(source);
        assert!(
            !body.contains("StartDrag"),
            "本模块的实现里出现了 StartDrag：两条拖动路会各攒一份坐标"
        );
        assert_eq!(
            body.matches("with_movable_by_background").count(),
            1,
            "系统那条拖动路只许出现一处（调用点），别在两个 builder 上各设一次"
        );
    }

    /// **形状核查：一份清单。** 带子与右键菜单两处都只许遍历 `ACTIONS`，
    /// 不许各自再写一遍按钮名——那句"只要四项"要被钉住，不是被打折理解。
    #[test]
    fn both_the_band_and_the_menu_iterate_the_one_table() {
        let body = implementation_lines(include_str!("pin.rs"));
        for name in ["band_buttons", "image_menu"] {
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
}
