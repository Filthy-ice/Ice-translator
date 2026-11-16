//! 区域截图（FR-2.10、CHG-009 REQ-001/002/009）。
//!
//! **唯一的"这台机器能不能截、截成什么样"的出处**：视图只问 [`capability()`] 与
//! [`explain()`]，不自己判断平台、不自己拼参数、不自己解释退出码——那三件事一旦散写，
//! "按钮灰不灰"和"这句话对不对"就会各自漂移（追踪表 V-004/V-005 锁的就是这个）。
//!
//! 为什么交给系统二进制而不是自己抓屏：macOS 上任何第三方拿屏幕像素都要走同一套系统接口，
//! 而"框选"这件事系统本来就有一个交互层，它认多屏、认缩放、认菜单栏让位，自己重写一份
//! 只会把这些边角重新踩一遍。代价是它**阻塞到用户画完为止**（可能两秒，也可能两分钟），
//! 所以调用方 SHALL 在 `spawn_blocking` 里跑 [`capture`]，SHALL NOT 在帧循环里等（事实 12）。
//!
//! ## 判据：为什么不是退出码（事实 14，2026-11-05 真机两条分支各跑一次）
//!
//! `esc` 取消与成功选区**两条路的退出码都是 0**，所以退出码在这条链上没有区分能力。
//! 能分开的只有"指定路径是否出现文件"，于是 [`classify`] 是一条**有序**判定，顺序不许调换：
//!
//! 1. 文件出现且解得出图像 ⇒ 成功；
//! 2. 文件出现但解不出图像（0 字节 / 坏 PNG）⇒ 失败，SHALL NOT 当成"取消"放过；
//! 3. 文件没出现，但退出码非 0 **或** stderr 非空 ⇒ 失败——权限那一档就是这个形状
//!    （本仓 D-28 那晚实测到 `could not create image from display`，它**不写文件**，
//!    所以只看"没文件"会把它误判成取消）；
//! 4. 文件没出现、退出码 0、stderr 空 ⇒ 取消，什么都不做。
//!
//! 另一条前置：交互层**不会**提前校验落盘路径（事实 24：目录不存在它也照样弹框选界面，
//! 等用户画完才失败——那是最坏的报错时机），所以 [`capture`] 在发起前先 `create_dir_all`。

use std::path::{Path, PathBuf};

/// 系统那个交互框选二进制。**这条绝对路径在本仓只许出现在这一行**：它就是"这台机器上
/// 截图这件事的入口"，出现第二处意味着有两个地方在各自决定怎么截（V-005 的静态核查数它）。
#[cfg(target_os = "macos")]
const TOOL: &str = "/usr/sbin/screencapture";

/// 四个动作各自的能力，**按动作分**而不是按平台分——非 mac 上"截得下来但认不出字"
/// 与"截不下来"是两种不同的界面话（REQ-009 的表格也按这四格记）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    /// 交互式框选能不能用
    pub capture: bool,
    /// 钉在屏幕上（无边框置顶子窗口）——egui 的 viewport API 三平台都有，但 Wayland 下
    /// 窗口层级由合成器说了算，这一格按 Linux 那档记"受阻"
    pub pin: bool,
    /// 全局热键：mac 走 Carbon，Windows 走 `RegisterHotKey`，Wayland 结构性无解
    pub hotkey: bool,
    /// 本地 OCR（唯一出处仍是 `ice_vision::native_ocr_supported()`，这里只是转发，
    /// 免得第二处自己写 `cfg(target_os)`）
    pub ocr: bool,
}

/// 这台机器的四格能力。**唯一判定处**：视图里出现 `#[cfg(target_os)]` 就是缺陷。
pub fn capability() -> Capability {
    Capability {
        capture: cfg!(target_os = "macos"),
        // 贴图窗用的 API（`show_viewport_deferred` + `with_always_on_top`）在 X11 有效；
        // Wayland 下置顶与定位都由合成器决定，本包不假装能做（NFR-7 那格记的是"受阻"）。
        pin: cfg!(target_os = "macos") || cfg!(target_os = "windows"),
        hotkey: cfg!(target_os = "macos") || cfg!(target_os = "windows"),
        ocr: ice_vision::native_ocr_supported(),
    }
}

/// 失败的那几类，只到"类别"为止——具体 stderr 与路径 SHALL NOT 往界面和日志里传
/// （REQ-008 那条边界：截图内容留在盘上已经是有意的例外，文案再带上文件名就是第二份暴露）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// 历史目录建不起来或写不进去（发起前就该发现）
    Unwritable,
    /// 那个系统二进制不存在或起不来
    ToolMissing,
    /// 交互层报了"拿不到画面"（屏幕录制权限那一档）
    NoPixels,
    /// 文件出现了但解不出图像（0 字节 / 坏 PNG）
    Corrupt,
    /// 其余非 0 退出
    Unknown,
}

/// 历史的两条上限（Q-007 默认档，7 天 + 200 张双上限）：**这两个数只在这一处定义**，
/// 老化、设置页那句说明、清除那一颗都读它们，写两处就会有一天一边说 7 天一边说 30 天。
pub const HISTORY_DAYS: u64 = 7;
pub const HISTORY_MAX: usize = 200;

/// 一次框选的三种真实结局。**取消是独立一档，不是失败**：`Cancelled` 那一支不建贴图、
/// 不改图片页任何内容、不落提示条（与 `system::panel` 的 `Reply::Cancelled` 同一条纪律）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Captured(PathBuf),
    Cancelled,
    Failed(Failure),
    /// 这台机器没有这件事（非 mac），调用方 SHALL 连子进程都不起。
    /// mac 构建里它永远不被构造——那是 `capture()` 非 mac 那一支的返回值，不是漏了分支
    #[allow(dead_code)]
    Unsupported,
}

impl Outcome {
    /// 给状态行与提示条的那句话。三类失败共用同一句是**有意的**：退路（系统截图
    /// `Cmd+Shift+4` 之后点「粘贴」）是既有且真能走通的一条（事实 9/10），
    /// 而"权限被拒"与"工具不在"对用户来说下一步是同一件事——分得更细只会多一句看不懂的术语。
    pub fn explain(&self) -> Option<&'static str> {
        match self {
            Outcome::Captured(_) | Outcome::Cancelled => None,
            Outcome::Unsupported => {
                Some("这台机器上没有可用的框选截图；可以先用系统截图再点「粘贴」")
            }
            Outcome::Failed(kind) => Some(match kind {
                Failure::Unwritable => "截图存不下：那个目录建不起来或写不进去；先把历史清一次再试",
                Failure::ToolMissing | Failure::NoPixels | Failure::Corrupt | Failure::Unknown => {
                    "这台机器上截不出来；可以先用系统截图（Cmd+Shift+4）再点「粘贴」"
                }
            }),
        }
    }
}

/// 历史里那张图的文件名：`<pid>-<自增>-<epoch 秒>.png`（REQ-007 定这一种形状，
/// 没有别的写法——目录本身就是专有的，名字再加前缀只会让人以为里面有第二套规则）。
///
/// 三段都不可省：只用自增，两个实例会在同一秒写出同一个名字、后一张覆盖前一张（用户
/// 抱怨过的"我截的图呢"）；只用 pid，同一个实例里连点两次会撞；只加时间戳，同一秒内的
/// 两次连点照样撞。纯函数，好把这三条都锁住。
pub fn file_name(pid: u32, seq: u64, epoch_secs: u64) -> String {
    format!("{pid}-{seq}-{epoch_secs}.png")
}

/// [`file_name`] 拼成完整落点
pub fn target_path(dir: &Path, pid: u32, seq: u64, epoch_secs: u64) -> PathBuf {
    dir.join(file_name(pid, seq, epoch_secs))
}

/// 交给交互层的参数：`-i` 框选、`-x` 不出快门声、`-o` 不带窗口阴影、`-t png` 定格式，
/// 末尾那个位置参数是**我们指定的落盘路径**（事实 1，`screencapture -h` 原文）。
pub fn tool_args(path: &Path) -> Vec<String> {
    vec![
        "-i".into(),
        "-x".into(),
        "-o".into(),
        "-t".into(),
        "png".into(),
        path.display().to_string(),
    ]
}

/// 判"这张文件是不是解得开"只看头部。
///
/// 为什么不全解：全解要等识别链那一步去做（`ice_vision::import::admit_path`），
/// 一张图在这里解一次、识别时再解一次纯属白花；而 0 字节与坏 PNG 这两种真实状态
/// 在头部就会失败，判据要的恰好只有这两档。
fn decodable(path: &Path) -> bool {
    image::image_dimensions(path).is_ok()
}

/// 上面那 1~4 的有序判定。把"退出码、stderr、文件在不在"三样实际观测交进来，
/// 是这条判据能被无头测的原因——真机跑一次只能给一个样本，而这里能把四路各锁一条。
pub fn classify(path: &Path, exit_zero: bool, stderr: &str) -> Outcome {
    if path.exists() {
        return if decodable(path) {
            Outcome::Captured(path.to_path_buf())
        } else {
            // 第 2 路：写了个打不开的东西。不许因为"文件在"就当成功，也不许因为
            // "退出码是 0"就当取消。
            Outcome::Failed(Failure::Corrupt)
        };
    }
    if exit_zero && stderr.trim().is_empty() {
        return Outcome::Cancelled;
    }
    // 第 3 路：没文件但报了东西。`could not create image from display` 是权限/拿不到画面
    // 那一档的实测指纹（D-28），单独立一类只为了日志能分清，界面文案仍与其余同类共用一句。
    if stderr.contains("could not create image") {
        return Outcome::Failed(Failure::NoPixels);
    }
    Outcome::Failed(Failure::Unknown)
}

/// 静默抓**一块屏**（CHG-010 REQ-001 的"先冻结"）。
///
/// 三条实测决定它的形状：
/// - 不带 `-D` 只给主屏（本机实测产出一张 1920×1080，左边那块 1920×1200 不在里面），
///   所以"每块屏各一扇遮罩"必须**按屏点名**，序号从 1 起；
/// - 区域那条路（`-R`）**不接受负坐标**（实测报 `-R requires a valid rect`），而本机第二块屏
///   就挂在 `(-1920,-120)`，所以不能拿矩形切屏，只能 `-D`；
/// - 它**没有"取消"这一档**：静默抓图不等用户画，所以 [`classify_grab`] 与 [`classify`] 不同——
///   这里"没有文件"只有失败一种读法。把老那套"没文件 + 退出码 0 = 取消"抄过来，
///   出的就是"抓屏明明失败了，界面却说用户取消了"（REQ-106 说的那条判据随系统工具退役）。
///
/// 调用方同样 SHALL 把它放在 `spawn_blocking` 里：一次约 0.08~0.22 秒，帧循环里等它就是一卡。
#[cfg(target_os = "macos")]
pub fn grab_screen(dir: &Path, seq: u64, display: u8) -> Outcome {
    let path = target_path(dir, std::process::id(), now_secs(), seq);
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("截图目录建不起来：{e}");
        return Outcome::Failed(Failure::Unwritable);
    }
    let output = match std::process::Command::new(TOOL)
        .args(grab_args(&path, display))
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("抓屏工具起不来：{e}");
            return Outcome::Failed(Failure::ToolMissing);
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    classify_grab(&path, output.status.success(), &stderr)
}

/// 非 mac：同签名，恒 `Unsupported`（与 [`capture`] 一条形状，不写第二套）。
#[cfg(not(target_os = "macos"))]
pub fn grab_screen(_dir: &Path, _seq: u64, _display: u8) -> Outcome {
    Outcome::Unsupported
}

/// 静默抓屏的参数。**序号 1 起是系统的读法**（`-D1` = 主屏），传 0 会被当成"没点名"，
/// 所以这里先夹一次，免得序号从 0 起时永远抓回主屏而第二块屏的画面被当成错误。
pub fn grab_args(path: &Path, display: u8) -> Vec<String> {
    vec![
        "-x".into(),
        "-o".into(),
        "-t".into(),
        "png".into(),
        format!("-D{}", display.max(1)),
        path.display().to_string(),
    ]
}

/// 抓屏那一档的有序判定：**没有"取消"这个出口**。
pub fn classify_grab(path: &Path, exit_zero: bool, stderr: &str) -> Outcome {
    if path.exists() {
        return if decodable(path) {
            Outcome::Captured(path.to_path_buf())
        } else {
            Outcome::Failed(Failure::Corrupt)
        };
    }
    if stderr.contains("could not create image") {
        return Outcome::Failed(Failure::NoPixels);
    }
    // 没文件、退出码 0、stderr 空：静默抓图里这是"什么都没拿到"，不是用户放弃。
    // 序号越界（例如只有两块屏却点名第三块）就是这个形状。
    let _ = exit_zero;
    Outcome::Failed(Failure::NoPixels)
}

/// 跑一次框选。**调用方负责把它放在 `spawn_blocking` 里**（它会阻塞到用户画完为止）。
///
/// 先建目录再发起（事实 24），建不起来就直接失败——此时交互层还没弹出来，用户什么都没画，
/// 报错的时机是"还没开始"而不是"画完了才发现存不下"。
#[cfg(target_os = "macos")]
pub fn capture(dir: &Path, seq: u64) -> Outcome {
    let path = target_path(dir, std::process::id(), now_secs(), seq);
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("截图目录建不起来：{e}");
        return Outcome::Failed(Failure::Unwritable);
    }
    let output = match std::process::Command::new(TOOL)
        .args(tool_args(&path))
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("框选工具起不来：{e}");
            return Outcome::Failed(Failure::ToolMissing);
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    classify(&path, output.status.success(), &stderr)
}

/// 非 mac：同签名，恒 `Unsupported`——**不写第二套形状**（REQ-009 的编译面纪律）。
#[cfg(not(target_os = "macos"))]
pub fn capture(_dir: &Path, _seq: u64) -> Outcome {
    Outcome::Unsupported
}

/// 把一张**内存里的图**写进历史目录，并按双上限老化。**这是"确认之后才落盘"的唯一入口**
/// （CHG-010 REQ-102/104：一次没确认的框选，盘上不许留任何东西）。
///
/// 为什么由我们写而不是让工具写：工具的 `-i` 那一档是"它自己挑路径写文件"，而自绘遮罩拿到的
/// 是内存里的一块像素，落盘点必须由我们定名——顺便把"文件名里没有用户内容"这条老纪律
/// （REQ-008，日志与文案都不许出现文件名）继续锁在 `file_name` 那一处。
///
/// 顺序是有意义的：**先建目录、再算名字、才写**。目录建不起来时先失败，免得白拿一个名字
/// 却什么都没落。**老化不在这里**——它已经有唯一的出口（帧循环里那一处，跑的是同一对上限
/// 常量与同一个 `retain`），这里再来一遍就是"两个地方各自删用户的文件"。
pub fn save_shot(dir: &Path, image: &image::RgbaImage, seq: u64) -> Result<PathBuf, Failure> {
    std::fs::create_dir_all(dir).map_err(|_| Failure::Unwritable)?;
    let path = target_path(dir, std::process::id(), now_secs(), seq);
    if let Err(e) = image.save(&path) {
        tracing::warn!("截图写不出去：{e}");
        return Err(Failure::Unwritable);
    }
    Ok(path)
}

/// 抓屏用的**临时**落点：静默抓图也要经一次磁盘（那是系统二进制的接口形状），
/// 但那份文件不是历史、也不许进历史目录——它只活到解码完成那一瞬（[`purge_temp`] 删掉）。
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("ice-translator-frozen-{}", std::process::id()))
}

/// 删掉这次抓屏留下的临时文件（**每屏一份，全删**；删不掉不报错——那是别人的进程在用的名字，
/// 我们无权判断，也不能因为一个删不掉的临时文件就说"截图失败"）。
pub fn purge_temp(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        std::fs::remove_file(entry.path()).ok();
    }
}

/// 历史目录的名字。**这个路径不是配置项**：能配就等于能把用户截到的东西配进仓库里，
/// 隐私边界交出去了（REQ-008）。
pub const SHOTS_DIR: &str = "shots";

/// 一张留在盘上的截图：路径加它的修改时间（老化与「最近」都只读这两样）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub path: PathBuf,
    /// epoch 秒
    pub mtime: u64,
}

/// 历史目录（`app_data_dir` 下那一格）。拿不到系统数据目录时按"存不下"报，
/// 与 [`Failure::Unwritable`] 同一档——截图这件事的失败面只有"存不下"和"截不出"两种。
pub fn shots_dir() -> Result<PathBuf, Failure> {
    ice_core::config::Config::app_data_dir()
        .map(|p| p.join(SHOTS_DIR))
        .map_err(|_| Failure::Unwritable)
}

/// 「最近」与老化 SHALL 共用这一个排序：**新→旧，同分按路径字典序**。
///
/// 为什么要写死同一个出处：F2「打开上一次」的"最近"和老化里"最老的那几张"是同一个序的
/// 两端，各排一次就会出现"刚截的图被当成最老的删掉"这类只对一半的行为。同分再按路径排
/// 是为了确定性——同一秒落两张是合法情况（连点），没有第二序就会随机。
pub fn sorted(shots: &[Shot]) -> Vec<&Shot> {
    let mut order: Vec<&Shot> = shots.iter().collect();
    order.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
    order
}

/// F2 队列口径（2026-11-16 用户改判 CHG-012 REQ-001/002/003：游标作废）的兜底选图：
/// 历史里**不在屏幕上**的最新一张。纯函数，不碰盘——读盘留在 `app.rs` 帧末那一层
/// （CHG-009 REQ-005 分层纪律）。排序与老化共用 [`sorted`]（事实 1），"从新到旧补"和
/// "最老先删"读的是同一个序。全都在屏幕上（或空历史）＝`None`，调用方提示"没有更多截图了"。
/// 关掉的图不走这里：它们由状态层的回队簿记优先补（见 `AppState::next_browse`）。
pub fn next_unpinned<'a>(shots: &'a [Shot], pinned: &[PathBuf]) -> Option<&'a Shot> {
    sorted(shots)
        .into_iter()
        .find(|shot| !pinned.contains(&shot.path))
}

/// 双上限老化，纯函数：**返回该删的那些路径**，删的动作在调用方。
///
/// 两个边界各钉一条（REQ-008）：`mtime` 恰好落在第 7 天那一刻的那条**不删**（"7 天内"的
/// 字面读法，删法是 `mtime < cutoff`）；张数超上限时从**最老的**开始删，而不是按名字或
/// 随机。先按时间筛、再按张数裁，两刀叠在同一份序上。
///
/// 为什么不开定时器跑它：老化发生在"用户正要截新的一张"那一刻最自然，也省掉帧循环里
/// 多一件事（没人看的时候不需要老化，`docs/05` §6 那条"帧循环里少干活的"同源）。
pub fn retain(shots: &[Shot], now: u64, days: u64, max: usize) -> Vec<PathBuf> {
    let cutoff = now.saturating_sub(days.saturating_mul(86_400));
    let order = sorted(shots);
    let mut dead: Vec<PathBuf> = Vec::new();
    let mut alive: Vec<&Shot> = Vec::new();
    for shot in order {
        if shot.mtime < cutoff {
            dead.push(shot.path.clone());
        } else {
            alive.push(shot);
        }
    }
    if alive.len() > max {
        dead.extend(alive[max..].iter().map(|shot| shot.path.clone()));
    }
    dead
}

/// 「清除所有历史」：删掉这个目录里的图片文件，**返回真的删掉了几个**。
///
/// 张数要交回给提示条——"清了"与"清了 37 张"是两句话，后者用户才知道它原来攒了多少。
/// 直接删而不是标记（REQ-008：留标记等于把内容留在原地，只是看不见了）。
pub fn clear_all(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 把目录里留下的图读成 [`Shot`] 列表（老化与 F2 的输入）。读不到的那一条跳过——
/// 一个 stat 都拿不到的条目本来也不值得为它报错。
pub fn list(dir: &Path) -> Vec<Shot> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())?;
            Some(Shot { path, mtime })
        })
        .collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

// ---- 截图收尾只叫醒帧循环，不再收起/还原自家窗（CHG-013 改口 CHG-009 REQ-010） ----

/// 收尾那一步要发哪两条命令，**按顺序**。抽成纯函数是因为隐藏态那一次是量出来的：
/// 只喊重画唤不出 UI 帧，得再对主窗发一次"隐藏中也画一帧"（事实 30/31）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// 让停摆的帧循环跑一帧（隐藏中也画）
    PaintWhileHidden,
    /// 叫一次普通重绘
    Repaint,
}

/// 收尾时**两条都要发**，不分"主窗发起前开没开"。
///
/// 这条唤醒是 CHG-009 REQ-006 的命脉，CHG-013 去掉截图的收起/还原后仍原样保留：
/// 托盘里按热键起贴图时主窗本就收着，子进程回来要靠 `RequestPaintWhileHidden` 那一帧
/// 才建得出遮罩/贴图（事实 30/31，"窗收着时只喊重画唤不出那一帧"）。主窗本就可见时
/// 这一帧照常画、无副作用，所以两种情形统一保留唤醒，不按"主窗开没开"分流（那正是
/// 被事实 5 否掉的旧判据 `root_was_hidden`）。`PaintWhileHidden` 只管一帧，顺序是
/// "先买一帧、再叫重绘"。
pub fn wake_commands() -> Vec<Wake> {
    vec![Wake::PaintWhileHidden, Wake::Repaint]
}

/// 子进程结束后叫醒帧循环（建遮罩/钉图仍要那一帧）。**不再对自家窗发 `Visible(true)`**：
/// CHG-013 起截图全程不收起自家窗（用户 2026-11-13"不用将我们的软件收起/透明"），主窗
/// 本就可见，`request_repaint` 即可；托盘隐藏态那一次靠 `RequestPaintWhileHidden` 买一帧。
/// 由**子进程完成那条线程**调用（不是帧循环）：隐藏态时帧循环根本不跑，挂在帧循环上
/// 的叫醒永远不会被执行。
pub fn wake_after_capture(ctx: &egui::Context) {
    for command in wake_commands() {
        match command {
            Wake::PaintWhileHidden => ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::RequestPaintWhileHidden,
            ),
            Wake::Repaint => ctx.request_repaint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// ①文件在且解得出 ⇒ 成功；②文件在但解不出 ⇒ 失败（而且**不是取消**）
    #[test]
    fn a_file_that_decodes_is_success_and_one_that_does_not_is_failure_not_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.png");
        let img = image::RgbImage::from_pixel(4, 3, image::Rgb([10, 20, 30]));
        img.save(&good).unwrap();
        assert_eq!(classify(&good, true, ""), Outcome::Captured(good.clone()));

        let empty = dir.path().join("zero.png");
        std::fs::File::create(&empty)
            .unwrap()
            .write_all(b"")
            .unwrap();
        assert_eq!(
            classify(&empty, true, ""),
            Outcome::Failed(Failure::Corrupt)
        );

        let junk = dir.path().join("junk.png");
        std::fs::File::create(&junk)
            .unwrap()
            .write_all(b"not a png at all")
            .unwrap();
        assert_eq!(classify(&junk, true, ""), Outcome::Failed(Failure::Corrupt));
    }

    /// ④没文件 + 退出码 0 + stderr 空 ⇒ 取消。**这条就是"退出码没有区分能力"那半边**：
    /// 成功路当年实测也是 0，所以 0 在这里只能配上"没有文件"才读成取消。
    #[test]
    fn no_file_and_a_clean_exit_is_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.png");
        assert_eq!(classify(&missing, true, ""), Outcome::Cancelled);
        // 只有空白也算"没报东西"：别把 "\n" 当成失败，用户会莫名其妙
        assert_eq!(classify(&missing, true, "\n "), Outcome::Cancelled);
    }

    /// ③没文件但报了东西 ⇒ 失败，而且**不许**滑进取消那一档
    #[test]
    fn no_file_with_a_complaint_is_failure_never_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.png");
        // 权限那一档的实测指纹（D-28）：不写文件
        let denied = "could not create image from display";
        assert_eq!(
            classify(&missing, false, denied),
            Outcome::Failed(Failure::NoPixels)
        );
        // 同一句话哪怕退出码是 0 也不算取消——判据是"报了东西"
        assert_eq!(
            classify(&missing, true, denied),
            Outcome::Failed(Failure::NoPixels)
        );
        // 非 0 退出、stderr 空：仍是失败，不是取消
        assert_eq!(
            classify(&missing, false, ""),
            Outcome::Failed(Failure::Unknown)
        );
    }

    /// 静默抓屏的参数：**点名那块屏**，序号从 1 起（实测 `-D1`→主屏、`-D2`→副屏）。
    /// 没有 `-i`：那条是"弹系统交互层等人画"，本包要的是"先冻住"。
    #[test]
    fn a_silent_grab_names_the_display_and_never_opens_the_interactive_layer() {
        let args = grab_args(Path::new("/tmp/f.png"), 2);
        assert!(
            !args.iter().any(|a| a == "-i"),
            "静默抓屏不许带交互层那一条：{args:?}"
        );
        assert!(args.windows(2).any(|w| w == ["-D2"]) || args.contains(&"-D2".to_string()));
        assert_eq!(args.last().unwrap(), "/tmp/f.png");
        // 序号 0 不是合法读法：夹成 1，免得副屏的序号从 0 起时永远抓回主屏
        assert!(grab_args(Path::new("/x.png"), 0).contains(&"-D1".to_string()));
    }

    /// **静默抓屏没有"取消"这一档**：没拿到东西只有失败一种读法。
    /// 把 [`classify`] 那套"没文件 + 退出码 0 + stderr 空 = 取消"抄到这里，
    /// 就会把"点名了一块不存在的屏"说成"你取消了"——REQ-106 说要随系统工具退役的正是那套判据。
    #[test]
    fn a_silent_grab_that_wrote_nothing_is_a_failure_not_a_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.png");
        assert_eq!(
            classify_grab(&missing, true, ""),
            Outcome::Failed(Failure::NoPixels),
            "退出码 0 也不算取消"
        );
        assert_eq!(
            classify_grab(&missing, false, "could not create image from display"),
            Outcome::Failed(Failure::NoPixels)
        );
        // 写了但解不开：与框选那一路同一档
        let junk = dir.path().join("junk.png");
        std::fs::File::create(&junk)
            .unwrap()
            .write_all(b"nope")
            .unwrap();
        assert_eq!(
            classify_grab(&junk, true, ""),
            Outcome::Failed(Failure::Corrupt)
        );
        // 正常抓到：与框选那一路同一个 Outcome，下游识别链不用分叉
        let good = dir.path().join("good.png");
        image::RgbImage::from_pixel(3, 2, image::Rgb([1, 2, 3]))
            .save(&good)
            .unwrap();
        assert_eq!(
            classify_grab(&good, true, ""),
            Outcome::Captured(good.clone())
        );
        assert_eq!(classify(&good, true, ""), Outcome::Captured(good));
    }

    /// 抓屏失败给的那句话与框选失败同一档（同一份退路），而 `Cancelled` 在这条路上
    /// **不该被产出来**——上面那条锁的是判定，这条锁的是"界面上会不会出现一句假话"。
    #[test]
    fn a_silent_grab_never_reports_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        // 一个不可能存在的屏序号：真跑一次，看它落成什么
        let outcome = grab_screen(dir.path(), 1, 99);
        if cfg!(target_os = "macos") {
            assert!(
                !matches!(outcome, Outcome::Cancelled),
                "静默抓屏报出取消就是判据用错了地方"
            );
            assert!(matches!(outcome, Outcome::Captured(_) | Outcome::Failed(_)));
        } else {
            assert_eq!(outcome, Outcome::Unsupported);
        }
    }

    /// 确认之后才落盘：**只有这一条路**会往历史目录写文件，写完立刻按双上限老化。
    #[test]
    fn a_confirmed_capture_is_the_only_way_into_the_history_directory() {
        let dir = tempfile::tempdir().unwrap();
        let img = image::RgbaImage::from_pixel(6, 4, image::Rgba([9, 9, 9, 255]));
        let path = save_shot(dir.path(), &img, 1).expect("能写进去");
        assert!(path.exists(), "说写完了却没文件");
        assert_eq!(list(dir.path()).len(), 1);
        // 名字仍是那三段（pid-自增-epoch），不含用户内容
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-1.png")
        );

        // 写第二张不会动第一张（老化不是这里的活）
        let img2 = image::RgbaImage::from_pixel(6, 4, image::Rgba([1, 1, 1, 255]));
        let second = save_shot(dir.path(), &img2, 2).unwrap();
        assert!(second.exists());
        assert_eq!(list(dir.path()).len(), 2, "写入这一处不许顺手删别人的截图");
    }

    /// 目录建不起来（父路径是个普通文件）时报的是**"存不下"**，不是"取消了"也不是"截不出"：
    /// 那两档的界面话给的是不同的退路，说错了用户会照着做一遍没用的动作。
    #[test]
    fn an_unwritable_history_directory_says_so_instead_of_blaming_the_screen() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("a-file").join("shots");
        std::fs::File::create(dir.path().join("a-file")).unwrap();
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        assert_eq!(save_shot(&blocked, &img, 1), Err(Failure::Unwritable),);
    }

    /// 临时落点按 pid 分开：两个实例同时截图不许互相删对方的文件。
    #[test]
    fn the_freeze_temp_directory_is_per_process() {
        let dir = temp_dir();
        assert!(dir.starts_with(std::env::temp_dir()));
        assert!(
            dir.to_string_lossy()
                .contains(&std::process::id().to_string()),
            "临时目录没带上 pid，两个实例会互删：{dir:?}"
        );
        // 不存在时不炸（用户可能一次都没截过就退出）
        purge_temp(&dir.join("never-created"));
    }

    /// 文件名三段都要有：两个实例（pid 不同）在同一秒同一序号也不许撞；
    /// 同一个实例连点两次（时间戳同、pid 同）靠序号不撞。
    #[test]
    fn the_file_name_survives_two_instances_and_double_clicks() {
        let a = file_name(111, 1, 1_700_000_000);
        let b = file_name(222, 1, 1_700_000_000); // 另一个实例，同一秒同一序号
        assert_ne!(a, b, "两个实例撞名就是后一张覆盖前一张");
        let c = file_name(111, 2, 1_700_000_000); // 同一实例连点第二下，还在同一秒
        assert_ne!(a, c, "同一秒内的两次连点撞名");
        let d = file_name(111, 1, 1_700_000_001); // 隔一秒再点
        assert_ne!(a, d);
        assert!(
            a.ends_with(".png"),
            "历史目录里只放图片文件（W-5.6 要 `ls` 看的就是这一条）"
        );
    }

    /// 落点由目录 + 名字拼出，且名字里没有用户内容（REQ-008：日志与提示条不许出现文件名，
    /// 所以路径只能由这一处生成、由识别链自己读，不进文案）
    #[test]
    fn the_target_path_is_the_directory_plus_that_name() {
        let p = target_path(Path::new("/tmp/x"), 7, 3, 99);
        assert_eq!(
            p.file_name().unwrap().to_string_lossy(),
            file_name(7, 3, 99)
        );
        assert_eq!(p.parent().unwrap(), Path::new("/tmp/x"));
    }

    /// 参数就是实测那一条，顺序也是（`-i` 必须在，落盘路径在最后作为位置参数）
    #[test]
    fn the_arguments_are_the_ones_measured_on_the_machine() {
        let args = tool_args(Path::new("/tmp/shot.png"));
        assert_eq!(args[0], "-i");
        assert_eq!(args.last().unwrap(), "/tmp/shot.png");
        assert!(args.windows(2).any(|w| w == ["-t", "png"]));
    }

    /// 三类失败共用那一句能行动的话——**分得更细只是多一句看不懂的术语**，
    /// 而那句话里必须真的给出退路（系统截图 + 粘贴），不是"出错了"。
    #[test]
    fn failures_share_one_actionable_sentence() {
        for k in [
            Failure::ToolMissing,
            Failure::NoPixels,
            Failure::Corrupt,
            Failure::Unknown,
        ] {
            let s = Outcome::Failed(k).explain().unwrap();
            assert!(s.contains("Cmd+Shift+4"), "{k:?} 那句没给退路");
            assert!(s.contains("粘贴"), "{k:?} 那句没指既有入口");
        }
        // 目录那一类给的是另一件事：先清历史，因为它不是"截不出来"而是"存不下去"
        let s = Outcome::Failed(Failure::Unwritable).explain().unwrap();
        assert!(s.contains("目录"));
        // 成功与取消没有话——取消不落提示条是 REQ-002 明写的
        assert_eq!(Outcome::Cancelled.explain(), None);
        assert_eq!(Outcome::Captured(PathBuf::from("/x")).explain(), None);
    }

    /// 非 mac 那一边界：这台机器连子进程都不该起，文案也不能是失败那一句
    #[test]
    fn unsupported_is_a_fourth_state_with_its_own_sentence() {
        let s = Outcome::Unsupported.explain().unwrap();
        assert!(s.contains("框选"));
        assert_ne!(s, Outcome::Failed(Failure::Unknown).explain().unwrap());
    }

    /// 能力判定只有一处，而且四格互不相同——**截图与识别在 mac 都真，Linux 全假**，
    /// 这条断言锁的是"这一处真的在按动作分档"，不是把 `cfg!` 抄了四遍。
    #[test]
    fn the_capability_answer_is_per_action_not_a_single_flag() {
        let c = capability();
        if cfg!(target_os = "macos") {
            assert!(c.capture && c.pin && c.hotkey && c.ocr);
        } else {
            assert!(!c.capture, "非 mac 不该报\"能截\"");
        }
    }

    /// `create_dir_all` 走不通时给的是 `Unwritable` 而不是"取消"：这条链最坏的失败时机
    /// 是用户画完才发现存不下（事实 24），所以这一档必须与取消分得开。
    #[test]
    fn an_unwritable_directory_is_reported_as_failure_not_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("a-file").join("sub");
        std::fs::File::create(dir.path().join("a-file")).unwrap(); // 父是个普通文件
        let outcome = capture(&blocked, 1);
        // mac 上会真的建不起来；非 mac 恒 Unsupported（这一格不在本包验收面）
        if cfg!(target_os = "macos") {
            assert_eq!(outcome, Outcome::Failed(Failure::Unwritable));
        } else {
            assert_eq!(outcome, Outcome::Unsupported);
        }
    }
    fn shot(name: &str, mtime: u64) -> Shot {
        Shot {
            path: PathBuf::from(name),
            mtime,
        }
    }

    /// 还原那一步**永远**先买一帧再叫重画：窗在这一刻就是收着的，只喊重画醒不过来
    /// （事实 30/31 + 2026-11-11 截图时间轴里那 779 毫秒的空档）。
    #[test]
    fn waking_the_loop_from_hidden_buys_exactly_one_painted_frame() {
        assert_eq!(
            wake_commands(),
            vec![Wake::PaintWhileHidden, Wake::Repaint],
            "顺序反了就等于没买那一帧：重画先落地，窗还收着，那一帧照样醒不过来"
        );
    }

    /// 老化的第一条边界：**恰好第 7 天那一条不删**（"7 天内"按字面读），过一秒才删。
    /// 这一条决定的是用户的口头预期——他说 7 天，就该是 7 天，不是"7 天减一个边界"。
    #[test]
    fn the_seventh_day_entry_survives_the_expiry_cut() {
        let day = 86_400;
        let now = 100 * day;
        let exact = shot("a", now - 7 * day); // 正好第七天
        let one_more = shot("b", now - 7 * day - 1); // 过了一秒
        let fresh = shot("c", now - day);
        let dead = retain(
            &[exact.clone(), one_more.clone(), fresh.clone()],
            now,
            7,
            200,
        );
        assert_eq!(dead, vec![one_more.path], "只该删过期的那一条");
        assert!(!dead.contains(&exact.path));
        assert!(!dead.contains(&fresh.path));
    }

    /// 第二条边界：**第 201 张进来，最老的那一条被删**（不是按名字、不是随机）。
    /// 两个上限叠在一起时先按时间筛、再按张数裁，剩下的那批里最老的先走。
    #[test]
    fn the_two_hundred_and_first_entry_pushes_out_the_oldest() {
        let now = 1_000_000;
        // 201 张，都在 7 天之内（时间那一刀一条都不该切掉），mtime 依次变新
        let shots: Vec<Shot> = (0..201)
            .map(|i| shot(&format!("{i:03}.png"), now + i))
            .collect();
        let dead = retain(&shots, now + 400, 7, 200);
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0], shots[0].path, "该挤掉的是最老的那一张");

        // 时间那一刀先走：3 张里 2 张过期、剩 1 张不超上限 ⇒ 只删那 2 张
        let mixed = vec![shot("old1", 0), shot("old2", 1), shot("new", now)];
        let dead = retain(&mixed, now, 7, 200);
        assert_eq!(dead.len(), 2);

        // 同一秒两张（连点是合法情况）：按路径序裁，结果确定，不靠运气
        let tie = vec![shot("b", 5), shot("a", 5), shot("c", 9)];
        let dead = retain(&tie, 9, 7, 2);
        assert_eq!(dead, vec![PathBuf::from("b")], "同分里路径靠后的那张算最老");
    }

    /// 「F2 补的那张」与老化 SHALL 读同一个序：不在屏上时补的最新一张，与"最老先删"里
    /// 最年轻的那一端，必须来自同一次排序。这条锁的是"两个定义各排一次"这种错法。
    #[test]
    fn the_f2_pick_and_the_aging_order_read_the_same_sort() {
        let shots = vec![shot("mid", 50), shot("newest", 80), shot("oldest", 10)];
        let order = sorted(&shots);
        assert_eq!(order[0].path, PathBuf::from("newest"));
        assert_eq!(order.last().unwrap().path, PathBuf::from("oldest"));
        assert_eq!(
            next_unpinned(&shots, &[]).unwrap().path,
            PathBuf::from("newest")
        );
        // 上限取 2 ⇒ 该删的正是排序尾部那一条，与 F2 先补的不是同一张
        let dead = retain(&shots, 80, 7, 2);
        assert_eq!(
            dead,
            vec![PathBuf::from("oldest")],
            "超出上限时最老的那一张先走"
        );
        assert_eq!(
            next_unpinned(&shots, &[]).unwrap().path,
            PathBuf::from("newest")
        );
    }

    /// F2 队列口径（2026-11-16 用户改判 CHG-012）：兜底选图只补**不在屏幕上**的，
    /// 从新到旧；全都在屏上＝`None`（调用方据此提示"没有更多截图了"）；空历史＝`None`。
    /// 每张历史图在屏幕上只能有一个实例——已钉的路径一律跳过，不会钉出第二份。
    #[test]
    fn f2_fills_newest_unpinned_and_stops_when_all_are_on_screen() {
        let shots = vec![shot("newest", 30), shot("mid", 20), shot("oldest", 10)];
        let newest = PathBuf::from("newest");
        let mid = PathBuf::from("mid");
        let oldest = PathBuf::from("oldest");

        assert_eq!(
            next_unpinned(&shots, &[]).unwrap().path,
            newest,
            "一张都没钉＝补最新"
        );
        assert_eq!(
            next_unpinned(&shots, std::slice::from_ref(&newest))
                .unwrap()
                .path,
            mid,
            "最新已在屏上＝补次新，不钉第二份"
        );
        assert_eq!(
            next_unpinned(&shots, &[newest.clone(), mid.clone()])
                .unwrap()
                .path,
            oldest,
            "再按＝补最老"
        );
        assert!(
            next_unpinned(&shots, &[newest, mid, oldest]).is_none(),
            "历史全在屏上＝没有更多截图了"
        );
        assert!(
            next_unpinned(&shots, &[PathBuf::from("gone")]).is_some(),
            "在屏名单里的陌生路径不影响选图"
        );
        assert!(next_unpinned(&[], &[]).is_none(), "空历史＝None");
    }

    /// 历史目录不许落在仓库里（REQ-008：路径不可配，可配就等于把这格交出去）。
    /// 拿不到系统目录时按"存不下"报，而不是悄悄退回临时目录。
    #[test]
    fn the_history_directory_is_never_inside_the_repository() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        match shots_dir() {
            Ok(dir) => {
                assert!(!dir.starts_with(repo), "历史目录指进仓里了：{dir:?}");
                assert_eq!(dir.file_name().unwrap(), SHOTS_DIR);
                assert!(dir.is_absolute());
            }
            Err(kind) => assert_eq!(kind, Failure::Unwritable),
        }
    }

    /// 「清除所有历史」返回**真删掉的张数**（提示条要说"清了 3 张"），
    /// 目录不存在时是 0 而不是崩——用户可能一次都没截过就点了清除。
    #[test]
    fn clearing_history_reports_how_many_were_actually_removed() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["1.png", "2.png", "3.png"] {
            std::fs::File::create(dir.path().join(name)).unwrap();
        }
        assert_eq!(clear_all(dir.path()), 3);
        assert_eq!(list(dir.path()).len(), 0, "清完读出来就该是空");
        // 再点一次：0 张，不是报错也不是 3
        assert_eq!(clear_all(dir.path()), 0);
        // 一个不存在的目录
        assert_eq!(clear_all(&dir.path().join("never-created")), 0);
    }

    /// 反向检查（REQ-008 的日志字段白名单）：文件名本身可能就是用户内容
    /// （`IMG_20261105_报销.png`），所以**任何一句界面话与失败文案都不许带上它**。
    /// 这条断言故意把真名字喂进去再检查输出——将来谁把路径插进文案，这条会立刻红。
    #[test]
    fn no_sentence_shown_to_the_user_contains_the_file_name() {
        let private = "IMG_20261105_报销-发票.png";
        let dir = PathBuf::from("/Users/x/Library/Application Support/IceTranslator/shots");
        let path = dir.join(private);
        let shots = vec![
            Shot {
                path: path.clone(),
                mtime: 0,
            },
            shot("newer", 9_999),
        ];

        // 老化算出来的东西里确实有那个名字（不然这条断言什么也没锁）
        let dead = retain(&shots, 9_999, 7, 1);
        assert!(dead.contains(&path));

        // 但每一句会出现在界面上的话都不含它
        for outcome in [
            Outcome::Failed(Failure::Unwritable),
            Outcome::Failed(Failure::NoPixels),
            Outcome::Failed(Failure::ToolMissing),
            Outcome::Failed(Failure::Corrupt),
            Outcome::Failed(Failure::Unknown),
            Outcome::Unsupported,
        ] {
            if let Some(text) = outcome.explain() {
                assert!(!text.contains(private), "文案漏了文件名：{text}");
                assert!(!text.contains("shots"), "文案漏了目录：{text}");
            }
        }
        // 成功那一支把路径交回内部链路，但它自己没有对外文案
        assert_eq!(Outcome::Captured(path.clone()).explain(), None);
    }

    /// 文件名与 [`list`] 读回来的对得上：老化与 F2 都靠 mtime，读不到 mtime 的那条跳过，
    /// 不许把"读不到"当成 0 秒（那会让它变成最老的一张并被优先删掉）。
    #[test]
    fn a_written_capture_file_is_listed_with_its_own_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = target_path(dir.path(), 42, 7, 1234);
        image::RgbImage::from_pixel(2, 2, image::Rgb([0, 0, 0]))
            .save(&path)
            .unwrap();
        let found = list(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, path);
        assert!(
            found[0].mtime > 1_700_000_000,
            "mtime 读成 0 就等于把它当最老的删"
        );
    }
}
