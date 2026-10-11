//! dsh web 后端协议客户端（HTTP unary RPC，无 WebSocket）。
//!
//! 事件来源对齐 `docs/versions/v0.1.7-rc.2/completion-notification-event-discovery.md`：
//! - `session/list`（wire 键 `_request`）低频轮询会话与 running 状态；
//! - `session/projections`（`{"request":{"sessionId":"..."}}`）取 `asOfSeq`（= 日志尾 seq）；
//! - `session/page`（`{"request":{...}}`，`throughSeq` = asOfSeq）取尾部记录，判定
//!   最新 `turn/end` 的 `reason.kind == "completed"`。
//! 全程 HTTP POST + Cookie 认证，零 WS、零 waterfall 副作用、零 Agent 激活
//! （page/projections 均冷安全）。
//!
//! 解析/判定逻辑为纯函数，可单元测试；HTTP 请求依赖真实 dsh 实例。

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

/// 一次 unary RPC 的调用结果（`server-response.result`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcResult {
    Ok(Value),
    Err { code: String, message: String },
}

/// 会话列表中的一行（`SessionSummary` 的投影）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    pub running: bool,
    pub blank: bool,
    pub origin: Option<String>,
    pub parent_session_id: Option<String>,
}

/// 一条 `turn/end` 事件的投影。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnEnd {
    pub seq: u64,
    pub turn: u64,
    pub kind: String,
}

/// 自增 rpcId 生成器（进程内唯一即可，服务端只做回显比对）。
static RPC_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_rpc_id() -> String {
    let n = RPC_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("dsh-desktop-{n}")
}

/// 解析 `session/list` 响应 value（`{items:[SessionSummary]}`）。
pub fn parse_list_sessions(value: &Value) -> Vec<SessionSummary> {
    let mut out = Vec::new();
    let Some(items) = value.get("items").and_then(Value::as_array) else {
        return out;
    };
    for item in items {
        let Some(session_id) = item.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        out.push(SessionSummary {
            session_id: session_id.to_string(),
            running: item.get("running").and_then(Value::as_bool).unwrap_or(false),
            blank: item.get("blank").and_then(Value::as_bool).unwrap_or(false),
            origin: item
                .get("origin")
                .and_then(Value::as_str)
                .map(str::to_string),
            parent_session_id: item
                .get("parentSessionId")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    out
}

/// 解析 `session/projections` 响应 value（`{asOfSeq, values}`），返回 asOfSeq。
///
/// `asOfSeq` 为日志尾 seq（`cursorBefore(session.seq)`），可作为 `session/page` 的
/// `throughSeq`。空日志时 asOfSeq 为 `-1`（无任何事件），返回 `None` 表示无完成可查。
pub fn parse_projection_cursor(value: &Value) -> Option<u64> {
    match value.get("asOfSeq").and_then(Value::as_i64) {
        Some(seq) if seq >= 0 => Some(seq as u64),
        _ => None,
    }
}

/// 从 `session/page` 的 records 中找**最新**一条 `turn/end` 事件。
///
/// records 形如 `[{type:"event", event:{type,seq,time,data}}]`；
/// `event.data` 为 `{turn, reason:{kind,...}}`。取 seq 最大的 `turn/end`。
pub fn find_latest_turn_end(records: &Value) -> Option<TurnEnd> {
    let Some(records) = records.as_array() else {
        return None;
    };
    let mut best: Option<TurnEnd> = None;
    for record in records {
        let event = match record.get("event") {
            Some(e) => e,
            None => continue,
        };
        if event.get("type").and_then(Value::as_str) != Some("turn/end") {
            continue;
        }
        let seq = event.get("seq").and_then(Value::as_u64)?;
        let turn = event
            .get("data")
            .and_then(|d| d.get("turn"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let kind = event
            .get("data")
            .and_then(|d| d.get("reason"))
            .and_then(|r| r.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if best.as_ref().map(|b| b.seq).unwrap_or(0) < seq {
            best = Some(TurnEnd { seq, turn, kind });
        }
    }
    best
}

/// 是否正常完成（规格 §1.2：仅 `completed` 计入；aborted/blocked/error/max-tokens/
/// interrupted/forked 均排除）。
pub fn is_normal_completion(turn_end: &TurnEnd) -> bool {
    turn_end.kind == "completed"
}

/// dsh web 后端 unary RPC 客户端。
///
/// `base` 形如 `http://127.0.0.1:<port>`（无尾斜杠）；`cookie` 为 `dsh-auth-*` 会话
/// cookie（`name=value` 段）。
pub struct DshClient {
    base: String,
    cookie: String,
    agent: ureq::Agent,
}

impl DshClient {
    pub fn new(base: &str, cookie: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            cookie: cookie.to_string(),
            agent: ureq::builder()
                .timeout_connect(std::time::Duration::from_secs(5))
                .build(),
        }
    }

    /// 执行一次 unary RPC。
    ///
    /// `endpoint` 形如 `session/list`；`args` 为 **wire 键已就位**的 args 对象
    /// （如 `session/list` 传 `json!({"_request":{}})`）。返回 `result` 字段值。
    pub fn rpc_unary(&self, endpoint: &str, args: Value) -> Result<RpcResult, String> {
        let url = format!("{}/api/{}", self.base, endpoint);
        let body = json!({
            "type": "client-request",
            "rpcId": next_rpc_id(),
            "method": endpoint,
            "payload": { "args": args },
        });
        let resp = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json")
            .set("Cookie", &self.cookie)
            .send_string(&body.to_string())
            .map_err(|e| format!("rpc {endpoint} request failed: {e}"))?;
        let text = resp
            .into_string()
            .map_err(|e| format!("rpc {endpoint} response read failed: {e}"))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| format!("rpc {endpoint} response decode failed: {e}"))?;
        let result = value
            .get("result")
            .ok_or_else(|| format!("rpc {endpoint} missing result"))?;
        if result.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(RpcResult::Ok(
                result
                    .get("value")
                    .cloned()
                    .unwrap_or(Value::Null),
            ))
        } else {
            Ok(RpcResult::Err {
                code: result
                    .get("error")
                    .and_then(|e| e.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                message: result
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        }
    }

    /// 会话列表（unary `session/list`）。
    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, String> {
        match self.rpc_unary("session/list", json!({"_request": {}}))? {
            RpcResult::Ok(value) => Ok(parse_list_sessions(&value)),
            RpcResult::Err { code, message } => Err(format!("session/list: {code} {message}")),
        }
    }

    /// 某顶层会话的日志尾 seq（unary `session/projections` 的 asOfSeq）。
    /// 会话不存在时返回 Ok(None)。
    pub fn session_cursor(&self, session_id: &str) -> Result<Option<u64>, String> {
        match self.rpc_unary(
            "session/projections",
            json!({"request": {"sessionId": session_id}}),
        )? {
            RpcResult::Ok(value) => Ok(parse_projection_cursor(&value)),
            RpcResult::Err { code, .. } if code == "session/not-found" => Ok(None),
            RpcResult::Err { code, message } => {
                Err(format!("session/projections: {code} {message}"))
            }
        }
    }

    /// 取某顶层会话的尾部记录（unary `session/page`，`throughSeq` = 日志尾 seq）。
    /// 返回原始 records 数组（含 `turn/end`）。
    pub fn session_tail(&self, session_id: &str, through_seq: u64) -> Result<Value, String> {
        let request = json!({
            "address": {"kind": "session", "sessionId": session_id},
            "throughSeq": through_seq,
            "maxMessages": 50,
        });
        match self.rpc_unary("session/page", json!({"request": request}))? {
            RpcResult::Ok(value) => Ok(value.get("records").cloned().unwrap_or(Value::Null)),
            RpcResult::Err { code, message } => Err(format!("session/page: {code} {message}")),
        }
    }

    /// 一站式：判断某顶层会话是否刚「正常完成」。
    ///
    /// - 取日志尾 seq（projections.asOfSeq）→ `session/page` 尾部记录 → 最新 `turn/end`；
    /// - `reason.kind == "completed"` 视为正常完成，返回 (seq, turn)。
    pub fn latest_completion(
        &self,
        session_id: &str,
    ) -> Result<Option<(u64, u64)>, String> {
        let Some(cursor) = self.session_cursor(session_id)? else {
            return Ok(None);
        };
        let records = self.session_tail(session_id, cursor)?;
        let Some(turn_end) = find_latest_turn_end(&records) else {
            return Ok(None);
        };
        if is_normal_completion(&turn_end) {
            Ok(Some((turn_end.seq, turn_end.turn)))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_list_sessions_extracts_fields() {
        let value = json!({
            "items": [
                {"sessionId": "s1", "running": true, "blank": false, "updatedAt": 1},
                {"sessionId": "s2", "running": false, "blank": true, "origin": "subagent", "parentSessionId": "s1", "updatedAt": 2},
            ]
        });
        let sessions = parse_list_sessions(&value);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "s1");
        assert!(sessions[0].running);
        assert_eq!(sessions[0].origin, None);
        assert_eq!(sessions[1].origin, Some("subagent".to_string()));
        assert_eq!(sessions[1].parent_session_id, Some("s1".to_string()));
    }

    #[test]
    fn parse_list_sessions_skips_malformed() {
        let value = json!({ "items": [{"running": true}, {"sessionId": 42}] });
        assert_eq!(parse_list_sessions(&value).len(), 0);
    }

    #[test]
    fn parse_projection_cursor_reads_as_of_seq() {
        assert_eq!(
            parse_projection_cursor(&json!({"asOfSeq": 1899, "values": {}})),
            Some(1899)
        );
        // 空日志：asOfSeq = -1 → 无完成可查
        assert_eq!(parse_projection_cursor(&json!({"asOfSeq": -1, "values": {}})), None);
        assert_eq!(parse_projection_cursor(&json!({"values": {}})), None);
        assert_eq!(parse_projection_cursor(&Value::Null), None);
    }

    #[test]
    fn find_latest_turn_end_picks_max_seq() {
        let records = json!([
            {"type": "event", "event": {"type": "user/message", "seq": 100, "time": 1, "data": {}}},
            {"type": "event", "event": {"type": "turn/end", "seq": 200, "time": 2, "data": {"turn": 1, "reason": {"kind": "completed"}}}},
            {"type": "event", "event": {"type": "step/end", "seq": 300, "time": 3, "data": {}}},
            {"type": "event", "event": {"type": "turn/end", "seq": 400, "time": 4, "data": {"turn": 2, "reason": {"kind": "interrupted"}}}},
        ]);
        let t = find_latest_turn_end(&records).unwrap();
        assert_eq!(t.seq, 400);
        assert_eq!(t.turn, 2);
        assert_eq!(t.kind, "interrupted");
    }

    #[test]
    fn find_latest_turn_end_returns_none_without_turn_end() {
        let records = json!([
            {"type": "event", "event": {"type": "user/message", "seq": 100, "time": 1, "data": {}}}
        ]);
        assert_eq!(find_latest_turn_end(&records), None);
    }

    #[test]
    fn is_normal_completion_only_completed() {
        assert!(is_normal_completion(&TurnEnd { seq: 1, turn: 1, kind: "completed".into() }));
        for bad in ["aborted", "blocked", "error", "max-tokens", "interrupted", "forked"] {
            assert!(!is_normal_completion(&TurnEnd {
                seq: 1,
                turn: 1,
                kind: bad.into()
            }));
        }
    }

    // ---- 集成：mock HTTP 服务器验证 ureq 调用链（请求构造 + 响应解析） ----

    /// 以 `std::net::TcpListener` 起一个多请求 mock HTTP 服务器，接受固定连接数后退出。
    /// 每个请求：按 Content-Length 可靠读完请求头+体后把 (request_line, headers, body)
    /// 交给 `on_request`，用其返回值 (status, payload) 写回 HTTP 响应。
    fn mock_server(
        max_connections: usize,
        on_request: impl Fn(String, String, String) -> (u16, String) + Send + Sync + 'static,
    ) -> (std::thread::JoinHandle<()>, u16) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..max_connections {
                let Ok((mut stream, _)) = listener.accept() else { break };
                // 读头部直到 \r\n\r\n
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
                // 解析 Content-Length 并读满 body
                let head_text = String::from_utf8_lossy(&head).to_string();
                let content_length = head_text
                    .lines()
                    .find_map(|line| {
                        let lower = line.to_ascii_lowercase();
                        lower.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                let mut body = head_text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                while body.len() < content_length {
                    let mut chunk = [0u8; 1024];
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => body.push_str(&String::from_utf8_lossy(&chunk[..n])),
                        Err(_) => break,
                    }
                }
                let mut lines = head_text.lines();
                let request_line = lines.next().unwrap_or("").to_string();
                let mut headers = String::new();
                for line in lines {
                    if line.is_empty() {
                        continue;
                    }
                    headers.push_str(line);
                    headers.push('\n');
                }
                let (status, payload) = on_request(request_line, headers, body);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
            }
        });
        (handle, port)
    }

    fn wait_thread(handle: std::thread::JoinHandle<()>) {
        let _ = handle.join();
    }

    #[test]
    fn rpc_unary_sends_expected_request_and_parses_ok() {
        use std::sync::{Arc, Mutex};
        let seen = Arc::new(Mutex::new(None::<(String, String, String)>));
        let seen_clone = seen.clone();
        let (server, port) = mock_server(1, move |request_line, headers, body| {
            *seen_clone.lock().unwrap() = Some((request_line, headers, body));
            (
                200,
                r#"{"type":"server-response","rpcId":"dsh-desktop-0","result":{"ok":true,"value":{"items":[]}}}"#
                    .to_string(),
            )
        });
        let client = DshClient::new(&format!("http://127.0.0.1:{port}"), "dsh-auth-test=abc");
        let result = client.list_sessions().unwrap();
        assert_eq!(result.len(), 0);
        wait_thread(server);

        let (request_line, headers, body) = seen.lock().unwrap().clone().unwrap();
        // 方法/路径
        assert!(request_line.starts_with("POST /api/session/list "), "got {request_line}");
        // Cookie 头
        assert!(headers.to_lowercase().contains("cookie: dsh-auth-test=abc"), "got {headers}");
        // body 结构
        let body_json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body_json["type"], "client-request");
        assert_eq!(body_json["method"], "session/list");
        assert!(body_json["payload"]["args"]["_request"].is_object());
        assert!(body_json["rpcId"].is_string());
    }

    #[test]
    fn rpc_unary_parses_error_result_and_not_found_maps_to_none() {
        // session/not-found → session_cursor 返回 Ok(None)
        let (server, port) = mock_server(1, |_, _, _| {
            (
                200,
                r#"{"type":"server-response","rpcId":"x","result":{"ok":false,"error":{"code":"session/not-found","message":"nope","details":{}}}}"#
                    .to_string(),
            )
        });
        let client = DshClient::new(&format!("http://127.0.0.1:{port}"), "c=v");
        assert_eq!(client.session_cursor("no-such").unwrap(), None);
        wait_thread(server);

        // 其他错误 → Err
        let (server2, port2) = mock_server(1, |_, _, _| {
            (
                200,
                r#"{"type":"server-response","rpcId":"x","result":{"ok":false,"error":{"code":"gateway/bad-request","message":"bad","details":{}}}}"#
                    .to_string(),
            )
        });
        let client2 = DshClient::new(&format!("http://127.0.0.1:{port2}"), "c=v");
        let err = client2.session_cursor("s1").unwrap_err();
        assert!(err.contains("gateway/bad-request"), "got {err}");
        wait_thread(server2);
    }

    #[test]
    fn latest_completion_detects_completed_turn_end_over_mock() {
        let (server, port) = mock_server(2, |request_line, _, _| {
            if request_line.contains("/session/projections") {
                (
                    200,
                    r#"{"type":"server-response","rpcId":"a","result":{"ok":true,"value":{"asOfSeq":5,"values":{}}}}"#
                        .to_string(),
                )
            } else if request_line.contains("/session/page") {
                (
                    200,
                    r#"{"type":"server-response","rpcId":"b","result":{"ok":true,"value":{"records":[
                        {"type":"event","event":{"type":"turn/end","seq":5,"time":1,"data":{"turn":1,"reason":{"kind":"completed"}}}}
                    ],"hasMore":false}}}"#
                        .to_string(),
                )
            } else {
                (404, r#"{"error":"unexpected"}"#.to_string())
            }
        });
        let client = DshClient::new(&format!("http://127.0.0.1:{port}"), "c=v");
        let completion = client.latest_completion("s1").unwrap();
        assert_eq!(completion, Some((5, 1)));
        wait_thread(server);
    }

    #[test]
    fn latest_completion_none_when_turn_end_not_completed() {
        let (server, port) = mock_server(2, |request_line, _, _| {
            if request_line.contains("/session/projections") {
                (
                    200,
                    r#"{"type":"server-response","rpcId":"a","result":{"ok":true,"value":{"asOfSeq":9,"values":{}}}}"#
                        .to_string(),
                )
            } else {
                (
                    200,
                    r#"{"type":"server-response","rpcId":"b","result":{"ok":true,"value":{"records":[
                        {"type":"event","event":{"type":"turn/end","seq":9,"time":1,"data":{"turn":2,"reason":{"kind":"interrupted"}}}}
                    ],"hasMore":false}}}"#
                        .to_string(),
                )
            }
        });
        let client = DshClient::new(&format!("http://127.0.0.1:{port}"), "c=v");
        let completion = client.latest_completion("s1").unwrap();
        assert_eq!(completion, None);
        wait_thread(server);
    }
}
