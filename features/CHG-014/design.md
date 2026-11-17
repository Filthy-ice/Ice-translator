Change: CHG-014
Version: v0.2
Spec-Hash: 404ac5357b9002b47583674be4b80008bb38b55de0f7f7f0670ca15ca316ef5e

# CHG-014 · Design

## 实现方案

仓库 `ice-translator`，分支按 `codebase/repositories.json` 登记；基线 commit `45c76ba`。核心模型（用户原话 Q-003）：**冻结截图上叠一层透明 RGBA 标注位图**。工具往这层落像素；橡皮擦在这层做 destination-out；显示与三出口都把这层 alpha 合成到背景上。因为擦的是位图 alpha，部分擦除天然成立（Q-002），不需要把矢量对象拆段。

模块落点（一个新模块 + 三处接线 + 两处文档）：

| 落点 | 改什么 | 为什么在这里 |
|---|---|---|
| 新增 `system/annotate.rs` | 光栅层 `Layer{image:RgbaImage}` + 落像素纯函数（rect_stroke/line/arrow/pencil/number_badge/text/mosaic）+ `erase(layer,from,to,radius)` destination-out + 预览矢量形状 | 纯函数、可无头断言；不碰 UI/IO，几何与像素操作集中一处 |
| `system/selection.rs` | 标注层按 display+frozen_id 存 `IdTypeMap`（照 `load_selection:532` 那套）；`show` 里背景之后叠标注层纹理 + 预览矢量；工具态分派 press/drag/release；药丸 `BAR_ICONS` 扩八工具+对勾最后、`bar_width` 同源扩 | 遮罩每屏一扇、状态各窗自持是既有形状（事实 2），标注层照挂不另起炉灶 |
| `state.rs:1221 crop_selection` | 裁出背景那块后，把标注层对应像素区域 alpha 合成进裁图（唯一合成点，事实 1） | 三出口（钉/复制/保存）都经此，改一处三出口一致 |
| `app.rs:1489-1527` | 确认三兄弟把该屏标注层一并交给 crop（或 crop 自读 capture 上的标注层引用） | 标注层运行态挂在 capture/遮罩侧，确认时取用 |
| `docs/06` / `docs/07` | W-14.x 走查 + 无头 TC；新增层与不变边界登记 | 验证层次与改口关系落账 |

**关键约束**：默认无工具激活时 press/drag/release 与现状完全一致（REQ-006 停止条件 3）；换算只走 `Field::to_pixels`（事实 4）；图标全线稿、① 画"圆圈+1"不依赖字符字形（事实 3、D-5）；零新增 crate（事实 5：`image` 已在、字形光栅 `ab_glyph_rasterizer`/`owned_ttf_parser` 已在 Cargo.lock 经 ice-vision）；粗细/色板运行态不进 config。

**拖拽预览与落层分离**（事实 7）：拖动中把当前形状用 egui 矢量画在面上（不脏层），松手才调 `annotate` 落像素进层；这样橡皮擦只擦已落层内容，预览不被擦、也不留半截脏像素。单击类（序号/文字）按下即落/唤起输入。

## REQ-001 实现与验证

`Layer` 建在该屏冻结帧像素尺寸（`frame.image.width()/height()`，与 `crop_selection` 同源）。显示：`show` 里背景纹理之后，把 `Layer.image` 传成 egui 纹理（脏时才重传，照 `freeze_texture:2116` 那套按代际命中）叠画；层空时不叠（与现状逐像素同）。合成：`crop_selection` 裁出 `cropped` 后，对标注层同坐标区域逐像素 alpha-over（`out = src*a + dst*(1-a)`），只改选区内像素。层随 `end_capture`/cancel 清（运行态）。存储照 `load_selection` 按 (display, frozen_id) 存 `IdTypeMap`，关窗不残留、不串屏。
验证：无头断言空层裁图 == 现状裁图（逐像素）；落一矩形后裁图四边像素被染、内部不变；合成不越选区界。真机 W-14.1 看叠层。

## REQ-002 实现与验证

`annotate.rs` 纯函数各落一种：`rect_stroke`（四边线段、只描边）、`line`（两端段）、`arrow`（线段+终点两短段成头）、`pencil`（采样点连段）、`number_badge`（实心圆+白数字，数字由会话计数器自增）、`text`（字形光栅落层，用 ice-vision 现成光栅或 egui Fonts 光栅，不联网）、`mosaic`（拖出区域从冻结背景盒式均值打码写层）。颜色/粗细入参读 REQ-003 当前值。药丸图标线稿：方框=rect_stroke 小样、斜箭头=line+头、斜直线=line、铅笔=笔形线稿、①=圆圈+数字1、T=painter.text("T")、黑白格=2x2 交替填充、橡皮擦=parallelogram 线稿。
验证：无头断言各形状落层像素（矩形内部空四边有、箭头有头、序号自增 1→2、马赛克块内=区域均值色）。真机 W-14.1 八种画一遍、图标读得懂。

## REQ-003 实现与验证

常量一处：`THICKNESS: [f32; N]`（默认中档下标）、`PALETTE: [Color32; 6]`（红默认+黄蓝绿白黑）。药丸下拉（egui `ComboBox` 或自绘弹出）选当前下标/色，存运行态（遮罩侧 temp，不进 config）。落层与预览同源读当前值。马赛克/橡皮擦不读色。
验证：无头断言换粗细线宽变、换色落层色变、默认红。真机 W-14.1 下拉好用。

## REQ-004 实现与验证

`erase(layer, from, to, radius)`：沿 from→to 采样圆笔刷，对覆盖像素置 alpha=0（destination-out），只动 layer 不动背景。半径读当前粗细档（可另给橡皮擦一档倍率常量）。部分擦除=只清抹过处，框半截/马赛克一角/序号一部分皆可。
验证：无头断言擦一笔后路径像素 alpha=0、路径外不变；擦半条框另半条仍在；背景冻结帧像素不被改。真机 W-14.1 手感。

## REQ-005 实现与验证

`BAR_ICONS` 扩为 [Rect,Arrow,Line,Pencil,Number,Text,Mosaic,Eraser, Copy,Save, Check]（对勾最后），加分隔视觉；`bar_width` 按新颗数同源算（不铺满锚点宽）；`icon_button` 加八种线稿分支+悬停提示；"落在药丸上不算框选手势"扩到工具按钮。既有 `the_bar_has_three_icons_in_this_order` 测试改判成新顺序（对勾最后）。

复制/保存两颗图标按用户 2026-11-17 v0.2 裁定重画（现码锚点 `selection.rs:924-932` Copy、`:933-979` Save）：
- **Save**：去掉原 U 形盘的三段（左右竖＋底横），改为「一支向下箭头（竖杆 `line_segment` ＋ 两段斜线成头）＋ 箭头下方一条独立横线」。横线与箭头不相连，是通用下载读法。
- **Copy**：后框移到**右上**、前框移到**左下**，前框压住后框 >50% 面积；画法＝先 `rect_stroke` 后框，再对前框 `rect_filled`（药丸底色 `Color32::from_rgba_premultiplied(28,28,30,235)`，与 `bar` 背景同源）＋ `rect_stroke` 前框。前框实心填充遮住后框被压部分，后框线条不透过前框（现码两框皆透明描边、互相透过，正是要改掉的）。
验证：无头断言顺序对勾最后、宽按内容、点工具不产生选区手势。真机 W-14.1 排列与图标，并逐颗核对保存=箭头下一横、复制=前框不透明压住右上后框一多半。

## REQ-006 实现与验证

遮罩侧加 `active_tool: Option<Tool>` 运行态（各窗自持）。`show` 的 press/drag/release：`active_tool=None` 走既有 Selection 分支（逐手势不变）；`Some(tool)` 走 annotate 分派（拖拽落层/预览、单击落/唤起），不碰 `next.rect`。点当前工具按钮或 esc 置回 None（esc 在工具态先退工具、不取消遮罩）。工具态下选区仍可见、对勾/复制/保存仍可用。
验证：无头断言无工具手势==现状、激活后拖拽 rect 不变而层变、esc 先退工具态。真机 W-14.1 切换顺手。

## REQ-007 实现与验证

不改 CHG-009/010/011/012/013 文件/快照；冻结/抓屏/换算/认窗/选框/command+c/翻译/落盘/老化/密钥照旧；零新增 crate（`cargo tree` 比对基线逐行同）、零新增 config 字段；文案/日志不含文件名或 OCR 文本；旧证据并入合并走查一次重录（事实 6）；docs/06 加 W-14.x、docs/07 登记。
验证：`./sdd scan`、`cargo tree -p ice-app -e normal` 零增量、`cargo clippy --workspace --all-targets -- -D warnings` 全绿；登记落 T-008。