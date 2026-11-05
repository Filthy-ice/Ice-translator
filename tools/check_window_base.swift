#!/usr/bin/env swift
//
// 整窗底色的机器判据（缺陷 D-26，2026-11-05）。
//
// 判的事只有一件：**标题栏那条带子与它下面的内容区，画出来是不是同一个底**。
// 为什么需要这么一条脚本——D-23 那一版把 `NSWindow.setBackgroundColor:` 当成了"整窗一个色"，
// 无头侧三条断言（`window_base == window_fill == panel_fill`）全绿，真机上那 32pt 仍是
// `NSThemeFrame` 按系统 chrome 画的纯白（实测 `#FFFFFF` vs 正文 `#F7F9_FC`）。
// 病根是**画那两条带子的不是同一个绘制者**，而 egui 的内省断言永远看不见原生那一位。
// 所以这条规矩不能靠"记住"，必须有一条跑一下就知道红不红的检查——将来五套预设色（任务 #28）
// 每一套都要跑它一次，色差就不会按主题数复发。
//
// 判据取"标题栏带 vs 内容区带"两处的**众数色**直接比，不跟配置里的目标色比：
// 整窗失焦半透明（`setAlphaValue`，默认 0.75）会把两块都按同一个比例叠到桌面上，
// 比目标色就会假红，比彼此则不受影响。取左边缘那一条竖带（x=3）的众数是因为那里
// 永远是底色本身，不会被输入框、按钮、文字挑走。
//
// 用法：
//   swift tools/check_window_base.swift                  # 现抓 Ice Translator 那扇窗
//   swift tools/check_window_base.swift <pid>            # 现抓指定进程的窗
//   swift tools/check_window_base.swift --image <png>    # 量一张存好的截图（取证与回归用）
// 退出码 0 = 同一个底；非 0 = 读得出两块色。

import AppKit
import CoreGraphics

/// 单通道允许的最大差。实测管线噪声（sRGB 取值 + 窗口半透明叠加）≤ 3，
/// 而 D-26 那一档色差是 (8, 7, 4)——这条线要拦得住后者又不会被前者骗红。
let TOLERANCE: Int = 5

/// 标题栏那一条取哪几行（1x 下 32pt，避开顶边那两行圆角与投影）。
let BAND_TITLEBAR = 4..<28
/// 内容区那一条取哪几行（贴着接缝往下，仍是面板底色那层，不是输入框）。
let BAND_CONTENT = 40..<64
/// 竖带所在列。左边缘 0..2 是圆角外的透明像素，3 起才是窗口自己。
let MARGIN_X = 3

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("✗ \(message)\n".utf8))
    exit(2)
}

// ---- 1) 拿到要量的位图 ----

func bitmapFromScreenshot(windowID: CGWindowID, size: CGSize) -> NSBitmapImageRep {
    let path = NSTemporaryDirectory() + "ice-window-\(windowID).png"
    let task = Process()
    task.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    // `-x` 不出快门声，`-o` 不连阴影一起拍（阴影那圈半透明会把顶行污染成"看着像色差"）
    task.arguments = ["-x", "-o", "-l\(windowID)", path]
    do {
        try task.run()
        task.waitUntilExit()
    } catch {
        fail("跑不了 screencapture：\(error)")
    }
    guard task.terminationStatus == 0,
          let image = NSImage(contentsOfFile: path),
          let tiff = image.tiffRepresentation,
          let rep = NSBitmapImageRep(data: tiff)
    else {
        fail("抓不到窗口 #\(windowID)。Terminal/脚本没有「屏幕录制」权限时这一句就是拿不到像素")
    }
    return rep
}

func onScreenWindow(for pid: pid_t) -> (CGWindowID, CGSize)? {
    guard let list = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
    ) as? [[String: Any]] else {
        return nil
    }
    var best: (CGWindowID, CGSize)? = nil
    for info in list {
        guard let owner = info[kCGWindowOwnerPID as String] as? pid_t, owner == pid,
              let raw = info[kCGWindowBounds as String] as? [String: Any],
              let w = raw["Width"] as? CGFloat, let h = raw["Height"] as? CGFloat,
              let number = info[kCGWindowNumber as String] as? CGWindowID
        else { continue }
        // 面板级（layer 3 = 置顶浮窗）与常规级（0）都要认；菜单、提示条那些更高档不算窗
        guard let layer = info[kCGWindowLayer as String] as? Int, layer == 0 || layer == 3 else {
            continue
        }
        if best == nil || w * h > best!.1.width * best!.1.height {
            best = (number, CGSize(width: w, height: h))
        }
    }
    return best
}

let arguments = Array(CommandLine.arguments.dropFirst())
let target: NSBitmapImageRep
if let at = arguments.firstIndex(of: "--image") {
    guard arguments.count > at + 1 else { fail("--image 后面要跟一个 png 路径") }
    guard let image = NSImage(contentsOfFile: arguments[at + 1]),
          let tiff = image.tiffRepresentation,
          let rep = NSBitmapImageRep(data: tiff)
    else { fail("读不到截图 \(arguments[at + 1])") }
    target = rep
    print("来源：\(arguments[at + 1])")
} else {
    let pid: pid_t
    if let first = arguments.first, let parsed = Int32(first) {
        pid = parsed
    } else {
        // 默认找正在跑的那一实例（打包名与开发名都认）
        let app = NSWorkspace.shared.runningApplications.first {
            $0.localizedName == "Ice Translator" || $0.executableURL?.lastPathComponent == "ice-translator"
        }
        guard let found = app?.processIdentifier else {
            fail("没找到在跑的 Ice Translator；要量别的进程就把它的第一格填上 pid")
        }
        pid = found
    }
    guard let window = onScreenWindow(for: pid) else {
        fail("pid \(pid) 没有在屏的窗（收进托盘/隐藏时量不到，先唤回再跑）")
    }
    target = bitmapFromScreenshot(windowID: window.0, size: window.1)
    print("来源：pid \(pid) 窗口 #\(window.0)")
}

// ---- 2) 取两块的众数色 ----

let width = target.pixelsWide, height = target.pixelsHigh
guard height > BAND_CONTENT.upperBound else {
    fail("这张图只有 \(height) 高，量不出内容区那一条（太矮的窗先拉大一点再跑）")
}
_ = width

func modeColor(in rows: Range<Int>) -> (red: Int, green: Int, blue: Int, hits: Int) {
    var tally: [Int: Int] = [:]
    for row in rows {
        guard let color = target.colorAt(x: MARGIN_X, y: row)?.usingColorSpace(.sRGB) else {
            continue
        }
        let packed = (Int((color.redComponent * 255).rounded()) << 16)
            | (Int((color.greenComponent * 255).rounded()) << 8)
            | Int((color.blueComponent * 255).rounded())
        tally[packed, default: 0] += 1
    }
    guard let top = tally.max(by: { $0.value < $1.value })?.key else {
        fail("x=\(MARGIN_X) 那一列读不到像素")
    }
    return ((top >> 16) & 0xFF, (top >> 8) & 0xFF, top & 0xFF, tally[top]!)
}

let band = modeColor(in: BAND_TITLEBAR)
let body = modeColor(in: BAND_CONTENT)
let drift = max(abs(band.red - body.red), abs(band.green - body.green), abs(band.blue - body.blue))

func hex(_ c: (red: Int, green: Int, blue: Int, hits: Int)) -> String {
    String(format: "#%02X_%02X_%02X", c.red, c.green, c.blue)
}

print("标题栏 y=\(BAND_TITLEBAR.lowerBound)..<\(BAND_TITLEBAR.upperBound) 众数 \(hex(band))（\(band.hits) 行）")
print("内容区 y=\(BAND_CONTENT.lowerBound)..<\(BAND_CONTENT.upperBound) 众数 \(hex(body))（\(body.hits) 行）")
print("单通道最大差 \(drift)，判据 ≤ \(TOLERANCE)")

if drift > TOLERANCE {
    print("✗ 读得出两块色：标题栏那一层没有跟着整窗底色走。")
    print("  先查 `system::background::sync_titlebar` 有没有把标题栏压平"
        + "（日志里那句 `标题栏平铺核对 flat=?`），再查底色下发"
        + "（`整窗底色下发 want=… native=?`）。")
    exit(1)
}
print("✓ 整窗一个底：标题栏那条带子与内容区是同一个色。")
