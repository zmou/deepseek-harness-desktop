#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Manager, RunEvent};
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
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
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
    cmd.arg(&bin)
        .arg("web")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        // 清除宿主/IDE 注入的 NODE_OPTIONS（其 --require shim 会卡死 dsh 启动）
        .env_remove("NODE_OPTIONS")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Windows：不分配 console 窗口（否则会弹黑色 console）
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn()
        .map_err(|e| format!("failed to spawn dsh: {e}"))
}

/// 逐行读 stdout，正则解析 `dsh web: http://127.0.0.1:PORT`，30s 超时。
fn wait_for_url(child: &mut Child) -> Result<String, String> {
    let re = regex::Regex::new(r"dsh web: (http://127\.0\.0\.1:\d+)").expect("invalid regex");
    let stdout = child.stdout.take().ok_or("cannot take stdout")?;
    let reader = BufReader::new(stdout);
    let deadline = Instant::now() + Duration::from_secs(30);

    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        log(&format!("[dsh] {line}"));
        if let Some(cap) = re.captures(&line) {
            return Ok(cap[1].to_string());
        }
        if Instant::now() > deadline {
            break;
        }
    }
    Err("timed out (30s) waiting for dsh URL".to_string())
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
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("dsh-session-{safe}.zip")
}

/// 「记住上次下载目录」的设置文件（纯文本，一行路径）。
fn last_download_dir_path() -> Option<PathBuf> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    Some(PathBuf::from(base).join("dsh-desktop").join("last-download-dir.txt"))
}

/// 读取记住的上次下载目录；不存在/失效则回退 None（调用方再回退系统「下载」目录）。
fn remembered_download_dir() -> Option<PathBuf> {
    let path = last_download_dir_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let dir = PathBuf::from(text.trim());
    if dir.is_dir() { Some(dir) } else { None }
}

/// 保存本次选择的下载目录，供下次对话框默认定位。
fn remember_download_dir(dir: &PathBuf) {
    let Some(path) = last_download_dir_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, dir.to_string_lossy().as_bytes());
}

/// Rust 侧直接拉取下载 URL 并写盘（on_download 取消 WebView2 下载后由此接管）。
/// 返回写入的字节数。
fn download_to(url: &str, dest: &PathBuf) -> Result<u64, String> {
    let resp = ureq::get(url).call().map_err(|e| format!("request failed: {e}"))?;
    if !(200..300).contains(&resp.status()) {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("create file failed: {e}"))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("write failed: {e}"))
}

fn main() {
    let app = tauri::Builder::default()
        // 单实例限制：必须第一个注册。第二实例启动时立即退出，
        // 并在本回调中把已有主窗口恢复/聚焦（窗口未创建时忽略——dsh 就绪前窗口可能还不存在）。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            child: Arc::new(Mutex::new(None)),
        })
        .setup(|app| {
            // 1. spawn dsh
            let mut child = match spawn_dsh(app.handle()) {
                Ok(c) => c,
                Err(e) => {
                    log(&format!("[dsh] {e}"));
                    std::process::exit(1);
                }
            };

            // 2. 解析出实际 URL
            let url = match wait_for_url(&mut child) {
                Ok(u) => u,
                Err(e) => {
                    log(&format!("[dsh] startup failed: {e}"));
                    if let Some(err) = child.stderr.take() {
                        for l in BufReader::new(err).lines().flatten() {
                            log(&format!("[dsh-err] {l}"));
                        }
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    std::process::exit(1);
                }
            };

            // 3. 保存 child 句柄到全局状态
            let state = app.state::<AppState>();
            *state.child.lock().unwrap() = Some(child);

            // 4. 用解析到的 URL 创建窗口
            let parsed = url
                .parse::<url::Url>()
                .map_err(|e| format!("invalid url {url}: {e}"))?;
            let handle = app.handle().clone();
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(parsed))
                .title("DeepSeek Harness")
                .inner_size(1400.0, 900.0)
                .resizable(true)
                // 隐藏 dsh 乐观 UI 的 Session 导出弹窗（详见常量注释）
                .initialization_script(HIDE_SESSION_DIALOG_SCRIPT)
                .on_download(move |webview, event| {
                    match event {
                        // 接管 WebView2 下载：返回 false 取消 WebView2 自身下载，
                        // 改由 tauri-plugin-dialog 异步弹「另存为」，用户确认后 Rust 侧 fetch + 写盘。
                        // 切勿在此回调线程上调用 blocking 对话框 API——WebView2 事件线程
                        // 无法泵 IFileDialog 的消息循环，会死锁整个 webview（实测卡死）。
                        DownloadEvent::Requested { url, .. } => {
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
                            let url_display = url.to_string();
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
                                            match download_to(&url_s, &chosen) {
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
                                url, success, path
                            ));
                        }
                        _ => {}
                    }
                    true
                })
                .build()
                .map_err(|e| format!("failed to create window: {e}"))?;

            // 5. 启动 child 退出监听线程
            let app_handle = app.handle().clone();
            let child_arc = state.child.clone();
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
                    // 子进程退出
                    Some(Ok(Some(status))) => {
                        if !status.success() {
                            log(&format!(
                                "[dsh] child exited abnormally: code={:?}",
                                status.code()
                            ));
                            for (_, w) in app_handle.webview_windows().iter() {
                                let _ = w.close();
                            }
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

    // 应用退出时回收子进程
    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            let state = app_handle.state::<AppState>();
            let mut guard = state.child.lock().unwrap();
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    });
}
