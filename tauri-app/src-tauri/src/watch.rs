//! 会话运行状态转换检测（纯逻辑，可单元测试）。
//!
//! 用途：主进程低频轮询 `session/list`，用本模块检测「某会话从 running=true 变为
//! running=false」的停止转换——这是「会话可能刚完成」的触发器。首轮观察只建立
//! 基线不触发，避免把启动前就已处于停止态的会话误判为「刚完成」（规格 §10.2/
//! §1.2 的未创建/已在停止语义）。
//!
//! 转换检测只负责「停止」，不判定「是否正常完成」；正常完成由上层用
//! `crate::protocol::DshClient::latest_completion`（`turn/end reason.kind`）确认。

use std::collections::HashMap;

use crate::protocol::SessionSummary;

/// 一个会话的停止转换（sessionId 为 owned，避免生命周期纠缠）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStopped {
    pub session_id: String,
}

/// 会话运行状态观察器。
pub struct SessionWatcher {
    /// sessionId -> 上次观察到的 running。
    known: HashMap<String, bool>,
    /// 是否已完成首轮基线。首轮只记录，不产出转换。
    initialized: bool,
}

impl SessionWatcher {
    pub fn new() -> Self {
        Self {
            known: HashMap::new(),
            initialized: false,
        }
    }

    /// 观察一轮会话状态。
    ///
    /// - 首轮：记录全部 running 作为基线，返回空（不把既有停止态当完成）。
    /// - 后续：对 `known` 中曾有 running=true 的会话，若本轮 running=false，
    ///   记为一次停止转换。
    /// - 新出现的会话只记录基线；下次轮询它变为 false 才转换。
    /// - 消失的会话（列表不再包含）：若上次是 running=true 也视为停止转换
    ///   （进程/会话被移除），否则忽略。
    pub fn observe(&mut self, sessions: &[SessionSummary]) -> Vec<SessionStopped> {
        let mut stopped = Vec::new();
        let mut next: HashMap<String, bool> = HashMap::with_capacity(sessions.len());

        for session in sessions {
            let was = self.known.get(&session.session_id).copied();
            next.insert(session.session_id.clone(), session.running);
            if !self.initialized {
                continue;
            }
            if was == Some(true) && !session.running {
                stopped.push(SessionStopped {
                    session_id: session.session_id.clone(),
                });
            }
        }

        // 消失的会话：上次 running=true → 停止转换
        if self.initialized {
            for (id, was_running) in self.known.iter() {
                if *was_running && !next.contains_key(id) {
                    stopped.push(SessionStopped {
                        session_id: id.clone(),
                    });
                }
            }
        }

        if !self.initialized {
            self.initialized = true;
        }
        self.known = next;
        stopped
    }
}

impl Default for SessionWatcher {
    fn default() -> Self {
        Self::new()
    }
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
    fn first_round_establishes_baseline() {
        let mut w = SessionWatcher::new();
        // 首轮：s1 已停止（running=false）——不得判定为「刚完成」
        let stopped = w.observe(&[summary("s1", false)]);
        assert!(stopped.is_empty());
    }

    #[test]
    fn running_to_stopped_detected() {
        let mut w = SessionWatcher::new();
        w.observe(&[summary("s1", true)]);
        let stopped = w.observe(&[summary("s1", false)]);
        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].session_id, "s1");
    }

    #[test]
    fn stays_stopped_no_repeat() {
        let mut w = SessionWatcher::new();
        w.observe(&[summary("s1", true)]);
        w.observe(&[summary("s1", false)]);
        let stopped = w.observe(&[summary("s1", false)]);
        assert!(stopped.is_empty());
    }

    #[test]
    fn new_session_then_stop_detected() {
        let mut w = SessionWatcher::new();
        w.observe(&[summary("s1", true), summary("s2", false)]);
        // s2 新出现且已停止仅建基线；下一轮出现就不算「刚完成」。
        // 这里验证：s1 复跑 true→false 依旧被检测。
        let stopped = w.observe(&[summary("s1", false), summary("s2", false)]);
        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].session_id, "s1");
    }

    #[test]
    fn removed_running_session_stops() {
        let mut w = SessionWatcher::new();
        w.observe(&[summary("s1", true)]);
        // s1 从列表消失（会话被移除）
        let stopped = w.observe(&[]);
        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].session_id, "s1");
    }

    #[test]
    fn removed_idle_session_ignored() {
        let mut w = SessionWatcher::new();
        w.observe(&[summary("s1", false)]);
        let stopped = w.observe(&[]);
        assert!(stopped.is_empty());
    }

    #[test]
    fn multiple_stops_in_one_round() {
        let mut w = SessionWatcher::new();
        w.observe(&[summary("s1", true), summary("s2", true)]);
        let mut stopped = w.observe(&[summary("s1", false), summary("s2", false)]);
        stopped.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        assert_eq!(stopped.len(), 2);
        assert_eq!(stopped[0].session_id, "s1");
        assert_eq!(stopped[1].session_id, "s2");
    }
}