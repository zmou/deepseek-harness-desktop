#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 会话完成通知的未读状态机（纯逻辑，可单元测试）
mod state;
// dsh web 后端协议客户端（HTTP unary RPC，可单元测试解析/判定）
mod protocol;
// 会话运行状态转换检测（纯逻辑，可单元测试）
mod watch;
// 会话完成通知主进程集成（轮询/判定/前台/徽标/通知）
mod completion;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::webview::DownloadEvent;
use tauri::{App, AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_dialog::DialogExt;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// 注入到 webview 的初始化脚本：隐藏 dsh 前端的 Session 导出反馈弹窗。
///
/// dsh 的 `SessionLogDownloadDialog`（`@deepseek-ai/dsh-client-ui-primitives` 的 `Modal`）
/// 通过 React Portal 渲染到 `document.body`，且为乐观 UI——点击下载后无论成败都会弹出
/// 「Session 导出已开始下载」提示，与我们真实的「另存为」流程重复且误导用户。
///
/// 在不修改 dsh 源码的前提下用 ARIA 属性选择器隐藏它：dsh `Modal` 渲染出
/// `role="dialog"` + `aria-modal="true"` + `aria-label={title}`，三个状态
/// （preparing / success / error）的标题都含 "Session"，而 ARIA 属性比 CSS Modules
/// 的 hash 类名稳定。整块隐藏可连同半透明遮罩一起去掉，避免留下灰幕挡住 UI。
///
/// 注意：`<style>` 必须等 `DOMContentLoaded` 后再挂到 `document.head`。脚本执行时
/// head 尚未创建，若回退挂到 `documentElement`（`<html>` 根）下浏览器不会应用该样式；
/// 另外 dsh 用 React Portal 在 DOMContentLoaded 之后才创建 dialog，因此再用
/// `MutationObserver` 兜底，把后出现的 dialog 及其外层 mask wrapper 一并隐藏。
const HIDE_SESSION_DIALOG_SCRIPT: &str = r#"
(function () {
  function injectHideRule() {
    if (document.getElementById('dsh-desktop-hide-session-dialog')) return;
    var style = document.createElement('style');
    style.id = 'dsh-desktop-hide-session-dialog';
    style.textContent = '[role="dialog"][aria-modal="true"][aria-label*="Session" i]{display:none!important}\n:has(>[role="dialog"][aria-modal="true"][aria-label*="Session" i]){display:none!important}';
    document.head.appendChild(style);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', injectHideRule);
  } else {
    injectHideRule();
  }
  function hideExistingDialogs() {
    var dialogs = document.querySelectorAll('[role="dialog"][aria-modal="true"]');
    for (var i = 0; i < dialogs.length; i++) {
      var label = dialogs[i].getAttribute('aria-label') || '';
      if (label.toLowerCase().indexOf('session') !== -1) {
        dialogs[i].style.display = 'none';
        var p = dialogs[i].parentElement;
        if (p) p.style.display = 'none';
      }
    }
  }
  var observer = new MutationObserver(function () {
    hideExistingDialogs();
  });
  function startObserver() {
    if (document.body) {
      observer.observe(document.body, { childList: true, subtree: true });
      hideExistingDialogs();
    }
  }
  if (document.body) {
    startObserver();
  } else {
    document.addEventListener('DOMContentLoaded', startObserver);
  }
})();
"#;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 全局子进程句柄，用 Arc<Mutex> 保证主进程退出钩子与监听线程之间线程安全。
struct AppState {
    child: Arc<Mutex<Option<Child>>>,
    /// dsh 0.1.2 起的浏览器会话 cookie（HttpOnly，仅 Rust 侧持有，用于下载请求）。
    cookie: Arc<Mutex<Option<String>>>,
    /// 显式退出状态：仅托盘菜单「退出」置位。置位后主窗口关闭不再被拦截为隐藏，
    /// 应用按正常退出流程回收 dsh child 并结束进程。线程安全：窗口事件、托盘回调、
    /// child 监听线程与 RunEvent 可能在不同执行上下文访问。
    quitting: Arc<AtomicBool>,
    /// 托盘图标实例。Tauri 的 TrayIcon 是引用计数的：build() 时已注册进 app 的
    /// 托盘 registry（registry 持有一份引用，托盘图标在应用退出前不会消失），
    /// 这里再持有一份引用确保任何路径下图标都存活，且只创建一次。
    tray: Arc<Mutex<Option<tauri::tray::TrayIcon<tauri::Wry>>>>,
}

impl AppState {
    fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    fn set_quitting(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }
}

/// 托盘菜单项的固定 ID。
const MENU_OPEN: &str = "open-main";
const MENU_QUIT: &str = "quit";
/// 托盘图标固定 ID，防止初始化重复创建。
const TRAY_ID: &str = "main-tray";
/// 托盘 tooltip。
const TRAY_TOOLTIP: &str = "DeepSeek Harness Desktop";

/// 统一的主窗口恢复函数：供第二实例回调、托盘双击、托盘菜单「打开主界面」共同调用。
///
/// - 最小化时先恢复（unminimize）；
/// - 显示并聚焦；
/// - 窗口尚未创建（dsh 仍在启动）时静默返回，不创建第二个窗口、不启动第二个 dsh。
fn show_main_window(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(win) => {
            let _ = win.unminimize();
            let _ = win.show();
            let _ = win.set_focus();
        }
        None => {
            log("[tray] main window not ready yet; ignore open request");
        }
    }
}

/// 统一退出入口：设置显式退出状态并请求应用退出。
///
/// 幂等：重复点击托盘「退出」或退出过程中再次触发，不会 panic / 死锁。
/// dsh child 的回收仍由 `RunEvent::Exit` 路径统一处理（见 main() 末尾）。
fn request_quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.is_quitting() {
        log("[tray] quit already requested; ignore duplicate request");
        return;
    }
    log("[tray] explicit quit requested");
    state.set_quitting();
    app.exit(0);
}

/// 创建托盘图标（唯一实例）并绑定菜单/双击事件。
///
/// - 从应用默认图标取得托盘图标（不复制新图标文件）。
/// - 使用固定 ID `main-tray` 防止重复创建。
/// - 菜单仅含两个项：`打开主界面`、`退出`（顺序固定，无分隔线）。
/// - 双击执行与「打开主界面」相同的恢复逻辑。
/// - 托盘是「关闭即隐藏」语义的前提：初始化失败时记录日志并返回 Err，
///   由调用方决定降级（不启用隐藏语义）或按启动失败处理（规格 §8.1）。
fn init_tray(app: &App) -> tauri::Result<tauri::tray::TrayIcon<tauri::Wry>> {
    let handle = app.handle();
    // 菜单：仅两个项，顺序固定。MenuId 用于事件分发。
    let open_item =
        tauri::menu::MenuItem::with_id(handle, MENU_OPEN, "打开主界面", true, None::<&str>)?;
    let quit_item = tauri::menu::MenuItem::with_id(handle, MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = MenuBuilder::new(handle)
        .item(&open_item)
        .item(&quit_item)
        .build()?;

    // 应用默认图标（Windows 取 bundle.icon 里的 icon.ico；其他平台取 icon.png）。
    let icon = handle
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(TRAY_TOOLTIP)
        .menu(&menu)
        // 关键：Windows 默认左键单击会弹出菜单，会吞掉双击。关闭左键弹菜单，
        // 右键弹菜单不受影响，双击事件才能正常触发（规格 §4.2/§4.3）。
        .show_menu_on_left_click(false)
        // 双击托盘图标 -> 恢复并聚焦主窗口（与「打开主界面」同一函数）。
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                let app = tray.app_handle();
                log("[tray] double-click; restoring main window");
                show_main_window(app);
            }
        })
        .on_menu_event(|app, event| {
            let id = &event.id;
            if id == MENU_OPEN {
                log("[tray] menu: open main window");
                show_main_window(app);
            } else if id == MENU_QUIT {
                request_quit(app);
            }
        })
        .build(handle)?;

    Ok(tray)
}

/// 去掉 Windows 长路径前缀（`\\?\`）。该前缀作为 node 命令行参数无法被解析。
fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => std::path::PathBuf::from(stripped),
        None => path.to_path_buf(),
    }
}

/// 日志文件路径：Windows 用 `%LOCALAPPDATA%\dsh-desktop\`，其他平台用 `~/.dsh/`。
fn log_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var("LOCALAPPDATA").ok()?;
        let dir = PathBuf::from(base).join("dsh-desktop");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir.join("dsh-desktop.log"))
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").ok()?;
        let dir = PathBuf::from(home).join(".dsh");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir.join("dsh-desktop.log"))
    }
}

/// 日志：dev 模式输出到控制台，所有模式追加写入日志文件（release 无 console 的兜底）。
fn log(msg: &str) {
    #[cfg(debug_assertions)]
    println!("{msg}");
    if let Some(path) = log_path() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{msg}");
        }
    }
}

/// 解析 node 二进制路径：优先 resource_dir 内嵌，dev 回退系统 node。
fn resolve_node(app: &AppHandle) -> String {
    // prod：resource_dir/runtime/node/node.exe（Windows）/ node（其他平台）
    if let Ok(dir) = app.path().resource_dir() {
        let node_name = if cfg!(windows) { "node.exe" } else { "node" };
        let embedded = dir.join("runtime").join("node").join(node_name);
        if embedded.exists() {
            return normalize_path(&embedded).to_string_lossy().to_string();
        }
    }
    // dev fallback：系统 PATH 的 node
    "node".to_string()
}

/// 解析 dsh 的 bin.js 路径：DSH_BIN 环境变量 > resource_dir 内嵌 > dev fallback .stage-p0。
fn resolve_dsh_bin(app: &AppHandle) -> Result<PathBuf, String> {
    // 1. DSH_BIN 环境变量（最高优先级，排障/调试用）
    if let Ok(p) = std::env::var("DSH_BIN") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return Ok(pb);
        }
        return Err(format!("DSH_BIN points to a missing file: {p}"));
    }
    // 2. 内嵌：resource_dir/runtime/dsh-runtime/node_modules/@deepseek-ai/dsh/lib/bin.js
    if let Ok(dir) = app.path().resource_dir() {
        let embedded = dir
            .join("runtime")
            .join("dsh-runtime")
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh")
            .join("lib")
            .join("bin.js");
        if embedded.exists() {
            return Ok(normalize_path(&embedded));
        }
    }
    // 3. dev fallback：cargo run 的工作目录是 src-tauri，仓库根在其上两级
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let default = cwd.join("../../.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js");
    if default.exists() {
        return Ok(default);
    }
    Err("dsh bin.js not found; set DSH_BIN env var or run build-runtime".to_string())
}

/// spawn node 运行 `dsh web --port 0 --no-open`，stdout/stderr 设为 piped。
fn spawn_dsh(app: &AppHandle) -> Result<Child, String> {
    let node = resolve_node(app);
    let bin = resolve_dsh_bin(app)?;
    log(&format!("[dsh] using node: {node}"));
    log(&format!("[dsh] using bin: {}", bin.display()));
    let mut cmd = Command::new(node);
    // 注意参数顺序与拼写：dsh 的 launcher 只解析**第一个未识别 token 之前**的自身旗标，
    // `--patch` 属 launcher 旗标，必须放在应用旗标（`--port`/`--no-open`）之前。
    // 且此处必须用显式的 `--profile web`：裸 `web` 缩写仅在作为首个 token 时成立，
    // 与 `--patch` 组合会报 `--profile <name> is required`（两者均实测）。
    cmd.arg(&bin).arg("--profile").arg("web");
    if let Some(patch_file) = write_bridge_patch(app) {
        cmd.arg("--patch").arg(&patch_file);
        log(&format!("[bridge] injecting patch: {}", patch_file.display()));
    } else {
        log("[bridge] dsh-desktop-bridge not found; viewed-session bridge disabled");
    }
    cmd.arg("--port")
        .arg("0")
        .arg("--no-open")
        // 清除宿主/IDE 注入的 NODE_OPTIONS（其 --require shim 会卡死 dsh 启动）
        .env_remove("NODE_OPTIONS")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Windows：不分配 console 窗口（否则会弹黑色 console）
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn().map_err(|e| format!("failed to spawn dsh: {e}"))
}

/// 解析 dsh-desktop-bridge 包的主入口（host 半 `lib/index.js`）。
/// 生产：resource_dir/bridge/lib/index.js；开发：仓库根 desktop-bridge/lib/index.js。
fn resolve_bridge_entry(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(dir) = app.path().resource_dir() {
        let embedded = dir.join("bridge").join("lib").join("index.js");
        if embedded.exists() {
            return Some(normalize_path(&embedded));
        }
    }
    // dev：cargo run 工作目录是 src-tauri，仓库根在其上级
    let cwd = std::env::current_dir().ok()?;
    let dev = cwd
        .join("../../desktop-bridge/lib/index.js")
        .canonicalize()
        .unwrap_or_else(|_| cwd.join("../../desktop-bridge/lib/index.js"));
    if dev.exists() {
        return Some(normalize_path(&dev));
    }
    None
}

/// 生成 bridge 注入 patch 文件（应用数据目录），返回其路径。
/// patch 内容：插入 dsh-desktop-bridge 行，name 指向 host 半入口的 file URL。
fn write_bridge_patch(app: &AppHandle) -> Option<PathBuf> {
    let entry = resolve_bridge_entry(app)?;
    let dir = app
        .path()
        .app_local_data_dir()
        .ok()?;
    let _ = std::fs::create_dir_all(&dir);
    let patch_file = dir.join("dsh-bridge.patch.yml");
    let file_url = url::Url::from_file_path(&entry).ok()?.to_string();
    let content = format!("- insert:\n  - id: dsh-desktop-bridge\n    name: {file_url}\n");
    std::fs::write(&patch_file, content).ok()?;
    Some(patch_file)
}

/// 把文本中 URL 的敏感 query 值（`token`）替换为 `***`，避免一次性凭据落盘到日志。
///
/// 日志行通常不是纯 URL（如 `dsh web: http://...`），直接 `Url::parse` 整行会失败，
/// 所以先用正则把文本里的 URL 逐个找出来脱敏；真正解析替换的是 `redact_single_url`。
fn redact_url(text: &str) -> String {
    static URL_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = URL_RE.get_or_init(|| regex::Regex::new(r"https?://[^\s]+").expect("invalid regex"));
    re.replace_all(text, |caps: &regex::Captures| redact_single_url(&caps[0]))
        .to_string()
}

fn redact_single_url(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.to_string();
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if !pairs.iter().any(|(k, _)| k == "token") {
        return url.to_string();
    }
    {
        let mut query = parsed.query_pairs_mut();
        query.clear();
        for (key, value) in &pairs {
            if key == "token" {
                query.append_pair(key, "***");
            } else {
                query.append_pair(key, value);
            }
        }
    }
    parsed.to_string()
}

/// ready URL 的正则（OnceLock 缓存；泵送线程会反复匹配）。
///
/// 注意必须取整条 URL（`http://[^\s]+`）而不是只取到端口：0.1.2 起 URL 带
/// `?token=` 一次性凭据，只取到端口会让 WebView 以无凭据地址打开 → 401 白屏。
fn ready_url_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"dsh web: (http://[^\s]+)").expect("invalid regex"))
}

/// 取出 child 的 stdout/stderr，交给两个后台线程持续逐行消费。
///
/// 关键：Windows 匿名管道缓冲只有约 64KB，读端不消费会出大事：
/// 1) **stderr 写满后 dsh 阻塞在 write 上，事件循环假死**，之后所有请求无响应
///    （实测 alpha 版：工具调用出错会往 stderr 写诊断，累积塞满后连 401 都不返回，
///    表现为“历史会话载入不出来 + 工具调用卡死”，两个症状同源）；
/// 2) stdout 读端若被提前 drop，dsh 再写入会 EPIPE，行为不可控。
///
/// 所以 spawn 后必须立即泵送：逐行经 redact_url 脱敏后写应用日志，
/// ready URL 通过 channel 上报给主流程。
fn start_pipe_pump(
    child: &mut Child,
    url_tx: std::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().flatten() {
                log(&format!("[dsh] {}", redact_url(&line)));
                if let Some(cap) = ready_url_re().captures(&line) {
                    let url = cap[1].to_string();
                    log(&format!("[dsh] ready url: {}", redact_url(&url)));
                    if url_tx.send(url).is_err() {
                        return; // 主流程已离开，无需再上报
                    }
                }
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().flatten() {
                log(&format!("[dsh-err] {}", redact_url(&line)));
            }
        });
    }
    Ok(())
}

/// 从 channel 收 ready URL；30s 无结果且 dsh 已退出时报错。
fn wait_for_ready_url(
    child: &mut Child,
    url_rx: std::sync::mpsc::Receiver<String>,
) -> Result<String, String> {
    loop {
        match url_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(url) => return Ok(url),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => match child.try_wait() {
                Ok(Some(status)) => return Err(format!("dsh exited early with {status}")),
                _ => return Err("timed out (30s) waiting for dsh URL".to_string()),
            },
        }
    }
}

/// 从下载 URL 的 `sessionId` query 重建安全文件名，规则与 dsh 前端 `sessionLogZipFilename` 一致
/// （保留 `[A-Za-z0-9_-]`，其余替换为 `_`）。
fn session_zip_filename(url: &url::Url) -> String {
    let id = url
        .query_pairs()
        .find(|(k, _)| k == "sessionId")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| "download".to_string());
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("dsh-session-{safe}.zip")
}

/// 「记住上次下载目录」的设置文件（纯文本，一行路径）。
fn last_download_dir_path() -> Option<PathBuf> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    Some(
        PathBuf::from(base)
            .join("dsh-desktop")
            .join("last-download-dir.txt"),
    )
}

/// 读取记住的上次下载目录；不存在/失效则回退 None（调用方再回退系统「下载」目录）。
fn remembered_download_dir() -> Option<PathBuf> {
    let path = last_download_dir_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let dir = PathBuf::from(text.trim());
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// 保存本次选择的下载目录，供下次对话框默认定位。
fn remember_download_dir(dir: &PathBuf) {
    let Some(path) = last_download_dir_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, dir.to_string_lossy().as_bytes());
}

/// 用 ready URL 携带的 token 换一个浏览器会话 cookie。
///
/// 0.1.2 起 web 服务只认会话 cookie：`BrowserAuth.isAuthenticated` 只读 `cookie`
/// 头，不再接受 URL 上的 token；而该 cookie 是 `HttpOnly`，前端 JS 和 Tauri 都
/// 读不到。因此 Rust 侧要自己下载，必须先复刻浏览器的首次访问
/// （`GET /?token=…` → 303 + `Set-Cookie`）拿一份 cookie。
///
/// token 是进程级且可重复兑换（服务端只做常量时间比对，不消耗），
/// 所以这里的兑换不会影响 WebView 之后的正常访问。
fn exchange_session_cookie(url: &str) -> Result<String, String> {
    // redirects(0)：我们需要 303 响应本身的 Set-Cookie，不能让 ureq 自动跳转
    let agent = ureq::builder().redirects(0).build();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("token exchange request failed: {e}"))?;
    let raw = resp.header("set-cookie").ok_or_else(|| {
        format!(
            "token exchange returned no Set-Cookie (status {})",
            resp.status()
        )
    })?;
    // 只保留 `name=value`，丢弃 Max-Age / Path / HttpOnly / SameSite 等属性
    let cookie = raw.split(';').next().unwrap_or("").trim().to_string();
    if cookie.is_empty() {
        return Err("token exchange returned an empty cookie".to_string());
    }
    Ok(cookie)
}

/// Rust 侧直接拉取下载 URL 并写盘（on_download 取消 WebView2 下载后由此接管）。
/// `cookie` 为 0.1.2 起的会话 cookie，缺失时服务端会返回 401。
/// 返回写入的字节数。
fn download_to(url: &str, dest: &PathBuf, cookie: Option<&str>) -> Result<u64, String> {
    let agent = ureq::builder().build();
    let mut request = agent.get(url);
    if let Some(c) = cookie {
        request = request.set("Cookie", c);
    }
    let resp = request.call().map_err(|e| format!("request failed: {e}"))?;
    if !(200..300).contains(&resp.status()) {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("create file failed: {e}"))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("write failed: {e}"))
}

/// 从 ready URL 推导无 token 的 HTTP 基址（`http://127.0.0.1:<port>`），
/// 供会话完成通知的 unary RPC 客户端使用（RPC 走 Cookie 认证，不接受 query token）。
fn base_url_of(url: &str) -> String {
    let mut u = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return url.to_string(),
    };
    u.set_query(None);
    u.set_fragment(None);
    u.to_string().trim_end_matches('/').to_string()
}

fn main() {
    let app = tauri::Builder::default()
        // 单实例限制：必须第一个注册。第二实例启动时立即退出，
        // 并在本回调中复用统一恢复函数把已有主窗口恢复/聚焦
        // （窗口未创建时忽略——dsh 就绪前窗口可能还不存在，规格 §8.4）。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log("[single-instance] second instance; restoring main window");
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        // 会话完成通知：原生系统通知插件（Rust 主进程直接调用，无需 capability）。
        .plugin(tauri_plugin_notification::init())
        .manage(AppState {
            child: Arc::new(Mutex::new(None)),
            cookie: Arc::new(Mutex::new(None)),
            quitting: Arc::new(AtomicBool::new(false)),
            tray: Arc::new(Mutex::new(None)),
        })
        .setup(|app| {
            // 0. 初始化托盘：必须在隐藏窗口之前可用，否则关闭后没有恢复入口。
            //    失败时按启动失败处理（规格 §8.1：不能静默创建「关闭后不可恢复」的隐藏窗口）。
            let tray = match init_tray(app) {
                Ok(t) => {
                    log("[tray] tray icon initialized");
                    t
                }
                Err(e) => {
                    log(&format!("[tray] tray initialization failed: {e}"));
                    std::process::exit(1);
                }
            };
            app.state::<AppState>().tray.lock().unwrap().replace(tray);

            // 1. spawn dsh
            let mut child = match spawn_dsh(app.handle()) {
                Ok(c) => c,
                Err(e) => {
                    log(&format!("[dsh] {e}"));
                    std::process::exit(1);
                }
            };

            // 2. 先启动 stdout/stderr 泵送线程（防止管道缓冲塞满把 dsh 卡死），
            //    再从 channel 收 ready URL。
            let (url_tx, url_rx) = std::sync::mpsc::channel();
            if let Err(e) = start_pipe_pump(&mut child, url_tx) {
                log(&format!("[dsh] {e}"));
                std::process::exit(1);
            }
            let url = match wait_for_ready_url(&mut child, url_rx) {
                Ok(u) => u,
                Err(e) => {
                    log(&format!("[dsh] startup failed: {e}"));
                    // stderr 已由泵送线程持续脱敏记录，无需再 dump
                    let _ = child.kill();
                    let _ = child.wait();
                    std::process::exit(1);
                }
            };

            // 3. 保存 child 句柄到全局状态
            let state = app.state::<AppState>();
            *state.child.lock().unwrap() = Some(child);

            // 3.5 用 token 预换一份会话 cookie，供 Rust 侧「另存为」下载使用。
            // 换不到不阻断启动：界面照常可用，只是下载会 401 并在日志里体现。
            let cookie = match exchange_session_cookie(&url) {
                Ok(c) => {
                    log("[dsh] session cookie acquired for downloads");
                    Some(c)
                }
                Err(e) => {
                    log(&format!("[dsh] session cookie exchange failed: {e}"));
                    None
                }
            };
            *state.cookie.lock().unwrap() = cookie;
            // 提前取值 move 进下载回调，避免回调线程再去争全局锁
            let download_cookie = state.cookie.lock().unwrap().clone();

            // 4. 用解析到的 URL 创建窗口
            let parsed = url
                .parse::<url::Url>()
                .map_err(|e| format!("invalid url {url}: {e}"))?;
            let handle = app.handle().clone();
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(parsed))
                .title("DeepSeek Harness Desktop")
                .inner_size(1400.0, 900.0)
                .resizable(true)
                // 隐藏 dsh 乐观 UI 的 Session 导出弹窗（详见常量注释）
                .initialization_script(HIDE_SESSION_DIALOG_SCRIPT)
                // Windows 专用：WebView2 附加浏览器参数。
                // 1) wry 默认会传 --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection，
                //    一旦用本方法就必须自行补上（见 tauri docs warning）。
                // 2) --no-proxy-server：强制 WebView2 不走系统代理（WinINET 里的
                //    Clash/mihomo 等代理会把 localhost 的 HTTP/WebSocket 连接也代理出去，
                //    导致 dsh 前端与本地 server 的 WebSocket 通信异常/卡死，headless Edge 不卡）。
                // 3) 注意：排查阶段曾用 --remote-debugging-port 开远程调试端口，
                //    生产构建必须移除（会对外暴露 DevTools 控制端口）。
                .additional_browser_args("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --no-proxy-server")
                .on_download(move |webview, event| {
                    match event {
                        // 接管 WebView2 下载：返回 false 取消 WebView2 自身下载，
                        // 改由 tauri-plugin-dialog 异步弹「另存为」，用户确认后 Rust 侧 fetch + 写盘。
                        // 切勿在此回调线程上调用 blocking 对话框 API——WebView2 事件线程
                        // 无法泵 IFileDialog 的消息循环，会死锁整个 webview（实测卡死）。
                        DownloadEvent::Requested { url, .. } => {
                            let cookie = download_cookie.clone();
                            // 默认目录：记住的上次目录 > 系统「下载」目录
                            let default_dir = remembered_download_dir().or_else(|| {
                                handle.path().resolve("", tauri::path::BaseDirectory::Download).ok()
                            });
                            let filename = session_zip_filename(&url);
                            let mut dialog = handle
                                .dialog()
                                .file()
                                .add_filter("ZIP 压缩文件", &["zip"])
                                .set_file_name(&filename);
                            if let Some(dir) = default_dir {
                                dialog = dialog.set_directory(dir);
                            }
                            // 对话框归属主窗口（此处仅记录窗口句柄，弹窗由插件在专用线程执行）
                            dialog = dialog.set_parent(&webview.window());
                            // 日志只记脱敏 URL，真实下载仍使用原始 URL
                            let url_display = redact_url(&url.to_string());
                            dialog.save_file(move |file_path| {
                                match file_path {
                                    Some(fp) => {
                                        let chosen: PathBuf = match fp {
                                            tauri_plugin_dialog::FilePath::Path(p) => p,
                                            tauri_plugin_dialog::FilePath::Url(u) => {
                                                PathBuf::from(u.path())
                                            }
                                        };
                                        if let Some(parent) = chosen.parent() {
                                            remember_download_dir(&parent.to_path_buf());
                                        }
                                        log(&format!(
                                            "[download] user chose destination: {}",
                                            chosen.display()
                                        ));
                                        // fetch + 写盘放到独立线程执行，不阻塞回调线程
                                        let url_s = url_display.clone();
                                        std::thread::spawn(move || {
                                            match download_to(&url_s, &chosen, cookie.as_deref()) {
                                                Ok(n) => log(&format!(
                                                    "[download] saved {n} bytes to {}",
                                                    chosen.display()
                                                )),
                                                Err(e) => {
                                                    log(&format!("[download] failed: {e}"))
                                                }
                                            }
                                        });
                                    }
                                    None => {
                                        log(&format!(
                                            "[download] cancelled by user: {url_display}"
                                        ));
                                    }
                                }
                            });
                            // 取消 WebView2 下载，由上述异步流程接管
                            return false;
                        }
                        DownloadEvent::Finished { url, success, path, .. } => {
                            // WebView2 自身下载均已取消，此分支理论上不再触发；保留日志兜底。
                            log(&format!(
                                "[download] webview download finished: {} success={} path={:?}",
                                redact_url(url.as_str()),
                                success,
                                path
                            ));
                        }
                        _ => {}
                    }
                    true
                })
                .build()
                .map_err(|e| format!("failed to create window: {e}"))?;

            // 4.6 会话完成通知：启动后台完成检测线程（规格 §12 第 3 步）。
            //     - 需要会话 cookie（unary RPC 走 Cookie 认证）；换不到则跳过（界面不受影响）。
            //     - 线程用 HTTP unary 轮询 session/list + projections + page，
            //       零 WS、零 waterfall 副作用、零 Agent 激活（冷安全）。
            if let Some(cookie) = state.cookie.lock().unwrap().clone() {
                let client =
                    crate::protocol::DshClient::new(&base_url_of(&url), &cookie);
                let state_path = crate::completion::state_file_path(app.handle());
                let app_handle = app.handle().clone();
                crate::log(&format!(
                    "[notify] completion pump starting; base={}",
                    redact_url(&base_url_of(&url))
                ));
                crate::completion::spawn_completion_pump(
                    app_handle,
                    client,
                    state_path,
                    || std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                );
            } else {
                log("[notify] no session cookie; completion notifications disabled");
            }

            // 4.5 主窗口关闭拦截：区分用户关闭与应用主动退出。
            //     - 未进入退出状态：prevent_default + 隐藏（关闭到托盘，规格 §4.4）。
            //     - 已进入退出状态（托盘「退出」/ 系统退出）：不拦截，正常退出（规格 §4.5）。
            //     - dsh child 异常退出路径会先置位退出状态再关窗，因此不会被再次隐藏（§5）。
            let state_for_close = app.state::<AppState>();
            let close_state = state_for_close.quitting.clone();
            let main_win = app.get_webview_window("main").unwrap();
            main_win.clone().on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if !close_state.load(Ordering::SeqCst) {
                        // 用户关闭 -> 隐藏到托盘（必要时先恢复非最小化，避免下次打开异常）
                        log("[tray] close requested; hiding to tray");
                        api.prevent_close();
                        let _ = main_win.unminimize();
                        let _ = main_win.hide();
                    }
                }
            });

            // 5. 启动 child 退出监听线程。
            //    child 异常退出必须绕过「用户关闭即隐藏」逻辑，最终退出应用，
            //    不能留下只有托盘、没有后端的死 UI（规格 §5 / §8.3）。
            let app_handle = app.handle().clone();
            let child_arc = state.child.clone();
            let quit_flag = state.quitting.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(500));
                let result = {
                    let mut guard = child_arc.lock().unwrap();
                    match guard.as_mut() {
                        Some(c) => Some(c.try_wait()),
                        None => None,
                    }
                };
                match result {
                    // child 已被 take（正常退出路径），结束监听
                    None => break,
                    // 仍在运行，继续轮询
                    Some(Ok(None)) => continue,
                    // 子进程退出：异常退出必须关掉窗口并退出应用；
                    // 正常退出（如外部关闭）同样不能让托盘空转，直接退出。
                    Some(Ok(Some(status))) => {
                        if !status.success() {
                            log(&format!(
                                "[dsh] child exited abnormally: code={:?}",
                                status.code()
                            ));
                        } else {
                            log(&format!(
                                "[dsh] child exited normally: code={:?}",
                                status.code()
                            ));
                        }
                        // 置位退出状态：后续 close 不再被拦截成隐藏。
                        quit_flag.store(true, Ordering::SeqCst);
                        // 关闭所有窗口。置位后 close 事件放行，窗口真正关闭；
                        // 最后一个窗口销毁后触发 ExitRequested -> Exit，回收 child。
                        for (_, w) in app_handle.webview_windows().iter() {
                            let _ = w.close();
                        }
                        break;
                    }
                    // try_wait 出错，视作已退出
                    Some(Err(_)) => break,
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // 应用退出时回收子进程、清理托盘（托盘图标随应用退出消失，规格 §4.1）
    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            let state = app_handle.state::<AppState>();
            // 置位退出状态，兜底任何 close 路径都放行
            state.set_quitting();
            // 显式 drop 托盘引用：让托盘图标立即消失（TrayIcon 是引用计数，
            // registry 的引用随 app 清理，这里主动释放我们持有的那份）。
            state.tray.lock().unwrap().take();
            let mut guard = state.child.lock().unwrap();
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    });
}
