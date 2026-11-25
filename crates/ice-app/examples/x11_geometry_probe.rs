//! 取证（CHG-017 T-008）：Linux 上"屏幕可用区"这三格到底问谁要。
//!
//! 规格里 design D-9 记的是 `xwininfo` 那一轮的读数（RandR 只报一块虚拟屏
//! `DP-1 2560x1440+0+0`，Wayland 应用的窗坐在 `+0+32`）。`screen::probe()` 的 Linux 分支
//! 要落进同一个形状，得先把这三件事量准：
//!
//! 1. `RRGetMonitors(get_active=true)` 给几块屏、每块的 x/y/w/h（**左上原点**，与 X11 一致）；
//! 2. `_NET_WORKAREA` 给不给得出"去掉顶栏与 Dock 的那块"（EWMH 按桌面向量排，四个一组）；
//! 3. RandR 与 EWMH 都拿不到时的兜底：root 窗的尺寸。
//!
//! 用法：`cargo run -p ice-app --example x11_geometry_probe`（Linux，需要 `DISPLAY`）。
//! 只打印几何数字，不打印任何窗口内容与标题。
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
use x11rb::rust_connection::RustConnection;

fn main() {
    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(pair) => pair,
        Err(err) => {
            println!("连不上 X：{err}（本机没起 XWayland 时就是这个形状）");
            std::process::exit(1);
        }
    };
    let screen = &conn.setup().roots[screen_num];
    println!(
        "root 窗尺寸（兜底那格）: {}x{}",
        screen.width_in_pixels, screen.height_in_pixels
    );
    let root = screen.root;
    monitors(&conn, root);
    eewmh(&conn, root);
}

/// RandR 1.5 的 monitor 列表：这是"几块屏"的唯一正解
fn monitors(conn: &RustConnection, root: u32) {
    match conn.randr_query_version(1, 5) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => println!(
                "RandR 版本: {}.{}",
                reply.major_version, reply.minor_version
            ),
            Err(err) => {
                println!("RandR 版本查询失败: {err}");
                return;
            }
        },
        Err(err) => {
            println!("RandR 扩展没起来: {err}");
            return;
        }
    }
    match conn.randr_get_monitors(root, true) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => {
                println!("RandR 在用的屏 {} 块:", reply.monitors.len());
                for m in &reply.monitors {
                    println!(
                        "  primary={} automatic={} x={} y={} w={} h={}",
                        m.primary, m.automatic, m.x, m.y, m.width, m.height
                    );
                }
            }
            Err(err) => println!("RRGetMonitors 回信失败: {err}"),
        },
        Err(err) => println!("RRGetMonitors 发不出去: {err}"),
    }
}

/// EWMH 那几个属性：`_NET_WORKAREA` 与 `_NET_DESKTOP_GEOMETRY`，都是 CARDINAL 数组
fn eewmh(conn: &RustConnection, root: u32) {
    for name in ["_NET_WORKAREA", "_NET_DESKTOP_GEOMETRY"] {
        let atom = match intern(conn, name) {
            Some(atom) => atom,
            None => {
                println!("{name}: 连 atom 都拿不到");
                continue;
            }
        };
        match cards(conn, root, atom) {
            Some(v) if !v.is_empty() => println!("{name}: {v:?}"),
            Some(_) => println!("{name}: 在但空"),
            None => println!("{name}: 读不到"),
        }
    }
}

fn intern(conn: &RustConnection, name: &str) -> Option<u32> {
    let cookie = conn.intern_atom(false, name.as_bytes()).ok()?;
    let reply = cookie.reply().ok()?;
    Some(reply.atom)
}

fn cards(conn: &RustConnection, root: u32, atom: u32) -> Option<Vec<u32>> {
    let cookie = conn
        .get_property(false, root, atom, AtomEnum::CARDINAL, 0, 16)
        .ok()?;
    let reply = cookie.reply().ok()?;
    Some(
        reply
            .value32()
            .map(|it| it.collect::<Vec<u32>>())
            .unwrap_or_default(),
    )
}
