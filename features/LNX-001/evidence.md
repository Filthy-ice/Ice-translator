Change: LNX-001
Version: v0.1
Spec-Hash: 73873bc2b37854d01f6f79faf9fdc94813c8003b9aaad9bb6c49e164d85d87cc

# Evidence（由 evidence.json 生成，勿直接修改）

- 2026-11-24T17:07:02+00:00 V-001 **passed** VERIFIED_CODE/local
  argv=["cargo", "test", "--workspace"] exit=0；附件：evidence/0001-V-001.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-24T17:12:04+00:00 V-005 **passed** VERIFIED_RUNTIME/sandbox
  本机真机（GNOME 50 + xdg-desktop-portal，2026-11-25 01:10–01:14）： 对我们这个 app 回 response=0，uri 是 （百分号编码，需解码），实测 2560x1440 / 1,485,463~1,794,593 字节，端到端耗时 508ms（含门户 PNG 编码与落盘）。首次调用未拦住我们（权限库自动放行）。**H1 成立**：REQ-002 按门户主路径实现，Q-1 那条口径（隐藏截图入口）自动作废。副作用记录：GNOME 的门户实现把图落进用户的 图片/ 目录，本包实现 SHALL 读完即删。；附件：evidence/0002-V-005.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-24T17:13:32+00:00 V-005 **passed** VERIFIED_RUNTIME/sandbox
  门户取证实测（GNOME 50 + xdg-desktop-portal，2026-11-25 01:17）：Screenshot 方法对本 app 返回 response=0；请求路径形如 /org/freedesktop/portal/desktop/request/1_240/t；Response 里的 uri 为 file:///home/<用户>/%E5%9B%BE%E7%89%87/Screenshot-5.png，即 percent-encoded，须解码后才能打开；解码后读到像素 2560x1440、文件 1804957 字节；端到端 508ms（含门户侧 PNG 编码与落盘）。H1 成立，REQ-002 走门户主路径，Q-1 的隐藏截图入口口径作废。副作用：GNOME 门户实现把图写进用户的 图片/ 目录，本包实现 SHALL 读完即删（本次运行结束后该文件已不存在）。；附件：evidence/0003-V-005.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-24T18:12:13+00:00 V-004 **passed** VERIFIED_RUNTIME/sandbox
  H4 真机（2026-11-25 02:11，GNOME 50 + XWayland，清掉 WAYLAND_DISPLAY 启动）：winit 自报 linux::x11::window（实际用了 X11 后端）；wgpu 仍列出 4 个适配器并以 Vulkan 呈现（NVIDIA RTX 5060 Ti / RADV / llvmpipe / Gl 回退）；单实例锁取到 pid=94834；API Key 从系统密钥库读回（key_len=35）→ Secret Service 应答（H3 同批成立）。两条已知差异记账：X11 下 wgpu 表面不支持透明合成（Transparent window 那条 WARN），剪贴板回退 X11 协议（Wayland data-control 连不上）。；附件：evidence/0004-V-004.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-24T18:12:17+00:00 V-006 **passed** VERIFIED_CODE/mock
  H5 真机（2026-11-25）：默认三把裸键在本机 X11 上 XGrabKey 全部抓到——F1 keycode=67 / F2 68 / F3 69，四组锁掩码（无/CapsLock/NumLock/两者）逐组 cookie.check() 均通过，未被 GNOME 或浏览器先占。实现侧的注册路径与探针同一写法（先 UngrabKey 再绑、check 判 BadAccess）。；附件：evidence/0005-V-006.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-25T14:45:13+00:00 V-008 **failed** VERIFIED_RUNTIME/sandbox
  REQ-005/REQ-008 的 Linux 侧（2026-11-25 真机）：「复制图片」可用（arboard 走 X11）；导出图片的"另存为"本轮不可用——门户 FileChooser.SaveFile 受理但收不到 Request.Response（空 parent 还会报 Failed to associate portal window with parent window；handle_token 不许带 -）。故面板保留 Unavailable 并如实提示。定格→框选→钉住→确认 一条链已通，字体面由 V-001 覆盖。；附件：evidence/0006-V-008.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-25T14:47:49+00:00 V-002 **passed** VERIFIED_CODE/local
  argv=["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"] exit=0；附件：evidence/0007-V-002.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-25T14:53:05+00:00 V-010 **passed** VERIFIED_RUNTIME/sandbox
  本机真机 SNI：watcher 在场（gnome-shell 持有，host 已注册）；应用注册 org.kde.StatusNotifierItem-47400-1，watcher 列表可见；IconName=ice-translator + 22×22 ARGB32 IconPixmap；dbusmenu GetLayout 返回五项（显示窗口/全量/设置/清除历史截图/退出程序），点选走 com.canonical.dbusmenu.Event → tray::mark → 既有 pump_tray（与 mac 同一个 PENDING）。日志 托盘这一档定下来了 next=Available。自查修掉一个缺陷：连接若不留活，名字随函数退出释放（日志 Available 但没有图标）。；附件：evidence/0008-V-010.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-25T14:53:51+00:00 V-007 **passed** VERIFIED_RUNTIME/sandbox
  T-002 对照（2026-11-25 真机）：X11 GetImage(root) 在 XWayland 上直接 BadMatch（error_code 8，GetImage），取不回任何像素；同一时刻门户 Screenshot 正常出 2560×1440。结论比 design D-6 原先的推断更强：GetImage 不是"漏掉 Wayland 原生窗"，而是在这一档不可用，所以截图只能走门户。对照探针留在 examples/grab_source_compare.rs。；附件：evidence/0009-V-007.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2

- 2026-11-25T15:08:16+00:00 V-008 **passed** VERIFIED_RUNTIME/sandbox
  REQ-005/REQ-008 Linux 侧：门户另存为走通——SaveFile 受理后，用户点"取消"收到 Response(code=1, uris=[])，parent_window 用真实 x11:0x… 关联成功。上一轮 V-008 记的 failed 是测量错误（旧探针没让对话框真正结束）。实现按既有异步形状：portal::save_file → SaveReply → panel::request_save 的回信通道，「复制图片」不受影响。；附件：evidence/0010-V-008.txt；Spec：fa20de951d58500f1d8d0024adebbd17ec3ee646dc21025176e0e0f9a201fac2
