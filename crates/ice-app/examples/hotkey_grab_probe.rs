//! 取证（CHG-017 T-011 / 假设 H5）：本机 X11 上 `XGrabKey` 抓不抓得到默认那三把裸键。
//!
//! 规格担心的是"浏览器/Shell 先把 F1/F2/F3 抓走了（H5）"。这里照实现的写法抓一遍：
//! 同样的四组锁掩码、同样的 `cookie.check()` 判据，抓完立刻放开，不改任何系统状态。
//!
//! 用法：`cargo run -p ice-app --example hotkey_grab_probe`。只打印键码与成败，不打印窗口内容。
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, Grab, GrabMode, ModMask};

const LOCK: u16 = 1 << 1;
const NUM: u16 = 1 << 4;

fn main() {
    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(pair) => pair,
        Err(err) => {
            println!("连不上 X：{err}");
            std::process::exit(1);
        }
    };
    let root = conn.setup().roots[screen_num].root;
    let setup = conn.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode - first + 1;
    let map = match conn
        .get_keyboard_mapping(first, count)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        Some(reply) => reply,
        None => {
            println!("键盘映射读不到");
            std::process::exit(1);
        }
    };
    let per = map.keysyms.len() / usize::from(count);
    for (name, keysym) in [("F1", 0xffbeu32), ("F2", 0xffbf), ("F3", 0xffc0)] {
        let keycode = map
            .keysyms
            .chunks(per)
            .position(|chunk| chunk.contains(&x11rb::protocol::xproto::Keysym::from(keysym)))
            .and_then(|i| u8::try_from(first as usize + i).ok());
        let Some(keycode) = keycode else {
            println!("{name}: 这个布局里没有对应键码");
            continue;
        };
        let mut verdict = "成功";
        for extra in [0u16, LOCK, NUM, LOCK | NUM] {
            let mask = ModMask::from(extra);
            let ok =
                match conn.grab_key(false, root, mask, keycode, GrabMode::ASYNC, GrabMode::ASYNC) {
                    Ok(cookie) => cookie.check().is_ok(),
                    Err(_) => false,
                };
            if !ok {
                verdict = "被占用（BadAccess）";
                break;
            }
        }
        println!("{name}: keycode={keycode} → {verdict}");
        let _ = conn.ungrab_key(Grab::ANY, root, ModMask::ANY);
        let _ = conn.flush();
    }
}
