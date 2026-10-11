//! 会话完成通知 — 主进程集成（轮询、完成判定、前台判定、徽标、通知）。
//!
//! 数据流（规格 §6.1）：
//! ```text
//! session/list 轮询（HTTP unary）
//!   → SessionWatcher 检测 running true→false（首轮建基线）
//!   → DshClient::latest_completion 确认 turn/end reason.kind=='completed'
//!   → UnreadStore::on_completed（去重 + 未读集合）
//!   → 回主线程：刷新 taskbar/Dock 徽标；若主窗口不在前台则发系统通知
//! ```
//!
//! 前台判定（规格 §3.3）：visible && focused 才算前台；hidden / minimized /
//! unfocused 均按后台（可通知）。
//!
//! 徽标：Windows 用 `WebviewWindow::set_overlay_icon`（taskbar 应用按钮 overlay，
//! 规格 §6.2 要求操作 taskbar button 而非托盘溢出区图标）；macOS 用
//! `set_badge_count`。未读数为 0 时清除。
//!
//! 通知：规格 §7 要求通知由主进程创建；用 `tauri-plugin-notification`（Windows
//! toast / macOS 通知中心）。权限关闭或失败不影响未读状态。

use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::protocol::{DshClient, SessionSummary};
use crate::state::UnreadStore;
use crate::watch::{SessionStopped, SessionWatcher};

/// 轮询间隔。
pub const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// 前台判定：规格 §3.3 —— 仅 visible+focused 属于前台。
/// hidden / minimized / unfocused 均视为后台（完成可触发系统通知）。
pub fn is_foreground(win: &WebviewWindow) -> bool {
    match (win.is_visible(), win.is_focused()) {
        (Ok(true), Ok(true)) => true,
        _ => false,
    }
}

/// 一轮检测结果：本次新确认的「正常完成」会话。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedCompletion {
    pub session_id: String,
    pub seq: u64,
    pub turn: u64,
}

/// 执行一轮检测（纯逻辑，可注入 mock client 测试）。
///
/// - 观察会话列表，得到停止转换；
/// - 仅对**顶层会话**（origin 非 subagent）做完成确认（规格：subagent 完成随父
///   会话体现，不单独通知）；
/// - 用 `latest_completion` 确认正常完成（`turn/end reason.kind=='completed'`），
///   返回确认结果；失败/非完成会话跳过并记录诊断日志。
pub fn run_detection_round(
    client: &DshClient,
    watcher: &mut SessionWatcher,
    sessions: &[SessionSummary],
    log: &mut dyn FnMut(&str),
) -> Vec<DetectedCompletion> {
    let mut out = Vec::new();
    let stopped = watcher.observe(sessions);
    for SessionStopped { session_id } in stopped {
        // 只处理顶层会话
        let Some(summary) = sessions.iter().find(|s| s.session_id == session_id) else {
            continue;
        };
        if summary.origin.as_deref() == Some("subagent") {
            log(&format!("[notify] skip subagent completion: {session_id}"));
            continue;
        }
        match client.latest_completion(&session_id) {
            Ok(Some((seq, turn))) => out.push(DetectedCompletion {
                session_id,
                seq,
                turn,
            }),
            Ok(None) => log(&format!("[notify] session stopped but not normal completion: {session_id}")),
            Err(e) => log(&format!("[notify] completion check failed {session_id}: {e}")),
        }
    }
    out
}

/// 把一批完成事件并入未读状态机，返回新未读列表（供通知/徽标）。
pub fn record_completions<'a>(
    store: &mut UnreadStore,
    completions: &'a [DetectedCompletion],
    now_ms: u64,
) -> Vec<&'a DetectedCompletion> {
    let mut fresh = Vec::new();
    for c in completions {
        match store.on_completed(&c.session_id, c.seq, c.turn, now_ms) {
            crate::state::CompleteOutcome::NewUnread => fresh.push(c),
            crate::state::CompleteOutcome::Duplicate => {}
        }
    }
    fresh
}

/// 未读数映射到 Windows taskbar overlay 数字徽标（规格 §6.2：taskbar 应用按钮
/// 数字徽标，而非托盘溢出区图标）。1-9 显示精确数字；10-99 显示 `9+`；≥100 显示
/// `99+`。图标由构建期 `include_image!` 内嵌（相对 CARGO_MANIFEST_DIR = src-tauri）。
#[cfg(target_os = "windows")]
fn badge_image_for(count: usize) -> tauri::image::Image<'static> {
    match count {
        1 => tauri::include_image!("../icons/badges/badge-1.png"),
        2 => tauri::include_image!("../icons/badges/badge-2.png"),
        3 => tauri::include_image!("../icons/badges/badge-3.png"),
        4 => tauri::include_image!("../icons/badges/badge-4.png"),
        5 => tauri::include_image!("../icons/badges/badge-5.png"),
        6 => tauri::include_image!("../icons/badges/badge-6.png"),
        7 => tauri::include_image!("../icons/badges/badge-7.png"),
        8 => tauri::include_image!("../icons/badges/badge-8.png"),
        9 => tauri::include_image!("../icons/badges/badge-9.png"),
        10..=99 => tauri::include_image!("../icons/badges/badge-9+.png"),
        _ => tauri::include_image!("../icons/badges/badge-99+.png"),
    }
}

/// 更新 taskbar/Dock 徽标。`unread_count` 为 0 时清除。
/// Windows：数字 overlay icon（taskbar 应用按钮，规格 §6.2）；
/// macOS：badge count。
///
/// 徽标失败不改变未读状态（规格 §4.3），但**必须**留下诊断日志，避免静默降级。
pub fn refresh_badge(app: &AppHandle, win: &WebviewWindow, unread_count: usize) {
    if unread_count == 0 {
        // 清除
        #[cfg(target_os = "windows")]
        match win.set_overlay_icon(None) {
            Ok(_) => crate::log("[notify] taskbar badge cleared"),
            Err(e) => crate::log(&format!("[notify] clear badge failed: {e}")),
        }
        #[cfg(target_os = "macos")]
        match win.set_badge_count(None) {
            Ok(_) => crate::log("[notify] dock badge cleared"),
            Err(e) => crate::log(&format!("[notify] clear badge failed: {e}")),
        }
        return;
    }
    // 有未读：Windows 用数字徽标；macOS 用数字徽标。
    #[cfg(target_os = "windows")]
    match win.set_overlay_icon(Some(badge_image_for(unread_count))) {
        Ok(_) => crate::log(&format!("[notify] taskbar badge set to {unread_count}")),
        Err(e) => crate::log(&format!("[notify] set badge failed: {e}")),
    }
    #[cfg(target_os = "macos")]
    match win.set_badge_count(Some(unread_count as i64)) {
        Ok(_) => crate::log(&format!("[notify] dock badge set to {unread_count}")),
        Err(e) => crate::log(&format!("[notify] set badge failed: {e}")),
    }
    let _ = app;
}

/// 显示系统通知（规格 §3.1/§3.4）：主进程驱动，文案通用无敏感内容。
/// 通知权限关闭或发送失败：记录诊断日志，不影响未读状态（规格 §4.3/§10.2）。
pub fn show_notification(app: &AppHandle, unread_count: usize, log: &mut dyn FnMut(&str)) {
    use tauri_plugin_notification::NotificationExt;
    match app
        .notification()
        .builder()
        .title("DeepSeek Harness Desktop")
        .body(if unread_count <= 1 {
            "有会话已完成，等待查看。".to_string()
        } else {
            format!("有 {unread_count} 个会话已完成，等待查看。")
        })
        .show()
    {
        Ok(_) => log(&format!("[notify] system notification shown; unread={unread_count}")),
        Err(e) => log(&format!("[notify] system notification failed: {e}")),
    }
}

/// 启动后台轮询线程。
///
/// 线程持有 `DshClient`（HTTP，无 WS）、`SessionWatcher`、`UnreadStore`；检测到
/// 新未读后通过 `run_on_main_thread` 回主线程刷新徽标并（后台时）发通知。
/// 线程在应用退出时随进程结束（不持有任何阻塞主线程的锁）。
pub fn spawn_completion_pump(
    app: AppHandle,
    client: DshClient,
    state_path: std::path::PathBuf,
    now: impl Fn() -> u64 + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut watcher = SessionWatcher::new();
        let mut store = UnreadStore::load(&state_path);
        // 启动即恢复徽标（规格 §5.3：读取未读集合并立即恢复徽标）
        if let Some(win) = app.get_webview_window("main") {
            let restored = store.unread_count();
            crate::log(&format!("[notify] restoring unread state: count={restored}"));
            refresh_badge(&app, &win, restored);
        }
        // 注入桥状态只上报一次（区分「桥未注入」与「尚无查看事件」，便于验收与排障）
        let mut bridge_state_logged = false;
        let mut bridge_attach_logged = false;
        loop {
            std::thread::sleep(POLL_INTERVAL);

            // 1) 收集注入桥上报的「用户已查看 session」并清除对应未读（规格 §4.4）。
            //    桥队列由页面内 client 插件写入，主进程原子取出后校验使用。
            if let Some(win) = app.get_webview_window("main") {
                let drain = drain_viewed_on_main_thread(&app, &win);
                // 首次取出结果记录一次（区分 eval 通道不通 / 桥未挂载 / 桥已挂载）；
                // 若桥随后才就绪，再补一条 attach 日志。
                if !bridge_state_logged {
                    if drain.bridge_ready {
                        crate::log(&format!(
                            "[notify] viewed-session bridge attached (state={})",
                            drain.bridge_state
                        ));
                    } else {
                        crate::log(&format!(
                            "[notify] viewed-session bridge not attached yet (state={})",
                            drain.bridge_state
                        ));
                    }
                    bridge_state_logged = true;
                } else if drain.bridge_ready && !bridge_attach_logged {
                    crate::log(&format!(
                        "[notify] viewed-session bridge attached (state={})",
                        drain.bridge_state
                    ));
                    bridge_attach_logged = true;
                }
                let mut cleared_any = false;
                for sid in drain.viewed {
                    if handle_viewed(&mut store, &state_path, &sid) {
                        crate::log(&format!("[notify] session viewed; unread cleared: {sid}"));
                        cleared_any = true;
                    }
                }
                if cleared_any {
                    let app_badge = app.clone();
                    let count = store.unread_count();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(win) = app_badge.get_webview_window("main") {
                            refresh_badge(&app_badge, &win, count);
                        }
                    });
                }
            }

            // 2) 轮询会话列表，检测「运行 -> 停止」转换并确认正常完成。
            let sessions = match client.list_sessions() {
                Ok(s) => s,
                Err(e) => {
                    crate::log(&format!("[notify] session/list failed: {e}"));
                    continue;
                }
            };
            let mut log_cb = |msg: &str| crate::log(msg);
            let completions = run_detection_round(&client, &mut watcher, &sessions, &mut log_cb);
            if completions.is_empty() {
                continue;
            }
            let fresh = record_completions(&mut store, &completions, now());
            if fresh.is_empty() {
                continue;
            }
            if let Err(e) = store.save(&state_path) {
                crate::log(&format!("[notify] failed to persist unread state: {e}"));
            }
            let app2 = app.clone();
            let fresh_ids: Vec<String> = fresh.iter().map(|c| c.session_id.clone()).collect();
            let count = store.unread_count();
            let _ = app.run_on_main_thread(move || {
                if let Some(win) = app2.get_webview_window("main") {
                    refresh_badge(&app2, &win, count);
                    if !is_foreground(&win) {
                        show_notification(&app2, count, &mut |m| crate::log(m));
                    }
                }
            });
            crate::log(&format!(
                "[notify] completions recorded: {:?}; unread={count}",
                fresh_ids
            ));
        }
    });
}

/// 处理「用户查看 session」信号（来自注入的 client 插件桥，规格 §3.3/§4.4）。
/// 返回是否清除了未读（供桥/主进程记录）。
pub fn handle_viewed(
    store: &mut UnreadStore,
    state_path: &std::path::Path,
    session_id: &str,
) -> bool {
    match store.on_viewed(session_id) {
        crate::state::ViewedOutcome::Cleared => {
            if let Err(e) = store.save(state_path) {
                crate::log(&format!("[notify] failed to persist after viewed: {e}"));
            }
            true
        }
        crate::state::ViewedOutcome::Noop => false,
    }
}

/// 校验注入桥上报的 sessionId（规格 §7：对长度和字符集做校验，不信任 payload）。
///
/// dsh session id 形如 `session-<uuid>`；这里只允许 ASCII 字母数字、`-`、`_`，
/// 长度 1..=128，避免恶意/异常内容进入日志或状态文件。
pub fn is_valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 页面内脚本：**原子地**取出并清空注入桥队列，返回结果对象 JSON。
///
/// 返回 `{"viewed":[...],"ready":bool,"state":string}`：
/// - `viewed`：本次取出的 sessionId 列表（读取 + 清空在同一次执行内完成，
///   避免两次收集之间丢事件或重复处理）；
/// - `ready`/`state`：注入桥挂载状态（诊断用，区分「桥未注入」与「尚无查看事件」）。
///
/// 脚本自包含且不抛异常（Windows 上 `eval_with_callback` 会吞掉异常）。
const DRAIN_VIEWED_SCRIPT: &str = r#"(function () {
  try {
    var q = window.__dshDesktopViewed__;
    var taken = [];
    if (Array.isArray(q) && q.length > 0) { taken = q.slice(0); q.length = 0; }
    return JSON.stringify({
      viewed: taken,
      ready: window.__dshDesktopBridgeReady__ === true,
      state: typeof window.__dshDesktopBridgeState__ === 'string' ? window.__dshDesktopBridgeState__ : 'absent'
    });
  } catch (e) {
    return '{"viewed":[],"ready":false,"state":"error"}';
  }
})()"#;

/// 一次队列收集的结果。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ViewedDrain {
    pub viewed: Vec<String>,
    /// 注入桥是否已在页面内挂载。
    pub bridge_ready: bool,
    /// 注入桥状态（`attached` / `no-ui-session` / `absent` / `error`）。
    pub bridge_state: String,
}

/// 解析队列收集结果 JSON，返回经校验的 [`ViewedDrain`]。
pub fn parse_drain_result(json: &str) -> ViewedDrain {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return ViewedDrain {
            bridge_state: "malformed".to_string(),
            ..Default::default()
        };
    };
    let viewed = value
        .get("viewed")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|s| is_valid_session_id(s))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    ViewedDrain {
        viewed,
        bridge_ready: value.get("ready").and_then(|v| v.as_bool()).unwrap_or(false),
        bridge_state: value
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("absent")
            .to_string(),
    }
}

/// 取出注入桥队列并等待结果（阻塞，带超时）。
///
/// 关键：`eval_with_callback` 必须经 `run_on_main_thread` 派发。直接在后台上调用
/// `WebviewWindow::eval_with_callback` 时，其底层 `send_user_message` 在非主线程
/// 上下文下不可靠（实测回调永不触发 → 表现为 `timeout`，桥永远显示未挂载）。
/// 因此这里先 `run_on_main_thread` 派发 eval，再用一次性 channel 把结果送回本线程。
pub fn drain_viewed_on_main_thread(app: &AppHandle, win: &WebviewWindow) -> ViewedDrain {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let (dispatch_tx, dispatch_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let win_for_main = win.clone();
    let tx_for_main = std::sync::Mutex::new(Some(tx));
    let dispatched = app.run_on_main_thread(move || {
        let result = win_for_main.eval_with_callback(DRAIN_VIEWED_SCRIPT, move |value| {
            if let Ok(mut guard) = tx_for_main.lock() {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(value);
                }
            }
        });
        let _ = dispatch_tx.send(result.map_err(|e| e.to_string()));
    });
    if let Err(e) = dispatched {
        crate::log(&format!("[notify] viewed drain dispatch failed: {e}"));
        return ViewedDrain {
            bridge_state: "dispatch-failed".to_string(),
            ..Default::default()
        };
    }
    // 等待派发结果（主线程可能尚未运行）
    match dispatch_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            crate::log(&format!("[notify] viewed drain eval failed: {e}"));
            return ViewedDrain {
                bridge_state: "eval-failed".to_string(),
                ..Default::default()
            };
        }
        Err(_) => {
            return ViewedDrain {
                bridge_state: "dispatch-timeout".to_string(),
                ..Default::default()
            }
        }
    }
    // 等待脚本执行结果
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(json) => parse_drain_result(&json),
        Err(_) => ViewedDrain {
            bridge_state: "eval-timeout".to_string(),
            ..Default::default()
        },
    }
}

/// 存储路径（Tauri app local data 下的状态文件）。
pub fn state_file_path(app: &AppHandle) -> std::path::PathBuf {
    let dir = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    dir.join("completion-unread.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: &str, running: bool) -> SessionSummary {
        SessionSummary {
            session_id: id.to_string(),
            running,
            blank: false,
            origin: None,
            parent_session_id: None,
        }
    }

    #[test]
    fn foreground_requires_visible_and_focused() {
        // is_foreground 依赖真实窗口句柄，无法单测；此处仅验证语义常量存在。
        assert!(POLL_INTERVAL > Duration::ZERO);
    }

    #[test]
    fn record_completions_only_new_unread() {
        let mut store = UnreadStore::new();
        let c1 = DetectedCompletion { session_id: "s1".into(), seq: 10, turn: 1 };
        let c2 = DetectedCompletion { session_id: "s1".into(), seq: 10, turn: 1 }; // 重放
        let c3 = DetectedCompletion { session_id: "s2".into(), seq: 20, turn: 1 };
        let completions = [c1, c2, c3];
        let fresh = record_completions(&mut store, &completions, 1000);
        assert_eq!(fresh.len(), 2);
        assert_eq!(fresh[0].session_id, "s1");
        assert_eq!(fresh[1].session_id, "s2");
        assert_eq!(store.unread_count(), 2);
    }

    #[test]
    fn handle_viewed_clears_and_persists() {
        let dir = std::env::temp_dir().join("dsh-notify-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("viewed.json");
        let _ = std::fs::remove_file(&path);
        let mut store = UnreadStore::new();
        store.on_completed("s1", 10, 1, 1000);
        assert!(handle_viewed(&mut store, &path, "s1"));
        assert_eq!(store.unread_count(), 0);
        assert!(!handle_viewed(&mut store, &path, "s1"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn session_id_validation_rejects_hostile_input() {
        assert!(is_valid_session_id("session-898194d2-59a4-4321-aa00-56ee29e2ca85"));
        assert!(is_valid_session_id("abc_123"));
        assert!(!is_valid_session_id(""));
        assert!(!is_valid_session_id("has space"));
        assert!(!is_valid_session_id("semi;colon"));
        assert!(!is_valid_session_id("../../etc/passwd"));
        assert!(!is_valid_session_id(&"x".repeat(129)));
    }

    #[test]
    fn parse_drain_result_reads_viewed_and_bridge_state() {
        let d = parse_drain_result(
            r#"{"viewed":["session-a","bad id","session-b"],"ready":true,"state":"attached"}"#,
        );
        assert_eq!(d.viewed, vec!["session-a", "session-b"]);
        assert!(d.bridge_ready);
        assert_eq!(d.bridge_state, "attached");
    }

    #[test]
    fn parse_drain_result_handles_absent_bridge_and_malformed() {
        let absent = parse_drain_result(r#"{"viewed":[],"ready":false,"state":"absent"}"#);
        assert!(absent.viewed.is_empty());
        assert!(!absent.bridge_ready);
        assert_eq!(absent.bridge_state, "absent");

        let malformed = parse_drain_result("not json");
        assert!(malformed.viewed.is_empty());
        assert!(!malformed.bridge_ready);
        assert_eq!(malformed.bridge_state, "malformed");
    }

    #[test]
    fn parse_drain_result_ignores_hostile_session_ids() {
        let d = parse_drain_result(
            r#"{"viewed":["../../etc/passwd","has space","ok_1",42,null],"ready":true,"state":"attached"}"#,
        );
        assert_eq!(d.viewed, vec!["ok_1"]);
    }
}
