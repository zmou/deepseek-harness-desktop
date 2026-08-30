#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, RunEvent};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Windows 子进程创建标志：不分配 console 窗口（spawn 出来的 node.exe 默认会弹黑色窗口）
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

fn main() {
    let app = tauri::Builder::default()
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
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(parsed))
                .title("DeepSeek Harness")
                .inner_size(1400.0, 900.0)
                .resizable(true)
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
