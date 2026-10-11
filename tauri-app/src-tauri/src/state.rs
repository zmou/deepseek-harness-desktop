//! 会话完成通知 — 未读状态机（纯逻辑，无 tauri 依赖，可单元测试）。
//!
//! 语义对齐 `docs/versions/v0.1.7-rc.2/desktop-completion-notification-spec.md` §4/§5：
//! - 未读数 = 不同 session 数（不是完成事件数、不是通知数、不是历史总数）。
//! - 同一 session 同一完成周期（seq）的重复/重放事件只计一次。
//! - 用户查看某 session 后清除该 session 的未读；仅恢复窗口/聚焦不清除。
//! - 状态跨应用重启保留；文件缺失/损坏按空状态处理，不阻断启动。

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

/// 当前持久化 schema 版本。
const SCHEMA_VERSION: u32 = 1;

/// 一个未读完成会话的持久化记录。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnreadEntry {
    /// 完成事件的 session 日志 seq（去重键的一部分；同一完成周期稳定）。
    pub seq: u64,
    /// 完成事件的 turn 序号（等价完成周期 ID）。
    pub turn: u64,
    /// 首次完成时间（Unix epoch 毫秒）。
    pub first_completed_at: u64,
    /// 已读时间（Unix epoch 毫秒）；`None` 表示仍未读。
    pub read_at: Option<u64>,
}

/// 磁盘上的完整状态文件内容。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnreadStateFile {
    pub schema_version: u32,
    /// sessionId -> 未读记录（有序无关；序列化时按键排序保证稳定输出）。
    #[serde(serialize_with = "ordered_map")]
    pub unread: HashMap<String, UnreadEntry>,
}

fn ordered_map<S>(map: &HashMap<String, UnreadEntry>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut items: Vec<_> = map.iter().collect();
    items.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = serializer.serialize_map(Some(items.len()))?;
    for (k, v) in items {
        out.serialize_entry(k, v)?;
    }
    out.end()
}

/// 会话完成通知状态机的内存形态。
///
/// 不是 `Clone`：状态机持有单一权威未读集合，由主进程串行驱动
/// （事件处理、查看信号、持久化刷新共享同一实例）。
pub struct UnreadStore {
    /// sessionId -> 未读记录。
    unread: HashMap<String, UnreadEntry>,
}

/// 处理一次「会话正常完成」事件的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompleteOutcome {
    /// 新未读（首次完成或新的独立完成周期）。
    NewUnread,
    /// 重复/重放事件，未读数不变。
    Duplicate,
}

/// 处理一次「用户查看 session」事件的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewedOutcome {
    /// 该 session 从未读集合移除。
    Cleared,
    /// 该 session 本就不在未读集合（或已经读）。
    Noop,
}

impl UnreadStore {
    /// 从空集合创建。
    pub fn new() -> Self {
        Self {
            unread: HashMap::new(),
        }
    }

    /// 从持久化文件加载；文件缺失、损坏或 schema 未知时返回空状态（不报错）。
    ///
    /// 调用方负责记录不含敏感内容的诊断日志。
    pub fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::new();
        };
        // 容忍 UTF-8 BOM：外部工具/编辑器写出的状态文件可能带 BOM，
        // 不带 BOM 处理会让整份未读状态被当成损坏而丢弃。
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        match serde_json::from_str::<UnreadStateFile>(text) {
            Ok(file) if file.schema_version == SCHEMA_VERSION => Self {
                unread: file.unread,
            },
            _ => Self::new(),
        }
    }

    /// 原子持久化：写临时文件后 rename 替换，避免强制退出留下半写文件。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let file = UnreadStateFile {
            schema_version: SCHEMA_VERSION,
            unread: self.unread.clone(),
        };
        let text = serde_json::to_string(&file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 记录一次正常完成事件。
    ///
    /// `seq` 为完成事件（`turn/end`）的 session 日志序列号，`turn` 为 turn 序号。
    /// 同一 session 的同一 seq 重复出现视为重放，不重复计数、不更新时间；
    /// 新 seq（新的独立完成周期）重新加入未读。
    pub fn on_completed(&mut self, session_id: &str, seq: u64, turn: u64, now_ms: u64) -> CompleteOutcome {
        match self.unread.get(session_id) {
            // 同一完成周期重放：保持原记录（含首次时间、已读状态）。
            Some(entry) if entry.seq == seq => CompleteOutcome::Duplicate,
            // 新完成周期：无论先前是否已读，重新加入未读并清掉旧已读标记。
            _ => {
                self.unread.insert(
                    session_id.to_string(),
                    UnreadEntry {
                        seq,
                        turn,
                        first_completed_at: now_ms,
                        read_at: None,
                    },
                );
                CompleteOutcome::NewUnread
            }
        }
    }

    /// 用户查看了某 session：从未读集合移除。
    pub fn on_viewed(&mut self, session_id: &str) -> ViewedOutcome {
        if self.unread.remove(session_id).is_some() {
            ViewedOutcome::Cleared
        } else {
            ViewedOutcome::Noop
        }
    }

    /// 当前未读数（不同 session 数）。
    pub fn unread_count(&self) -> usize {
        self.unread.len()
    }

    /// 未读 session 集合（稳定排序，便于测试/日志）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn unread_sessions(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.unread.keys().map(String::as_str).collect();
        ids.sort_unstable();
        ids
    }

    /// 某 session 是否未读。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_unread(&self, session_id: &str) -> bool {
        self.unread.contains_key(session_id)
    }
}

impl Default for UnreadStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("dsh-desktop-state-test");
        let _ = fs::create_dir_all(&dir);
        dir.join(name)
    }

    #[test]
    fn first_completion_adds_unread() {
        let mut s = UnreadStore::new();
        assert_eq!(s.on_completed("s1", 10, 1, 1000), CompleteOutcome::NewUnread);
        assert_eq!(s.unread_count(), 1);
        assert!(s.is_unread("s1"));
    }

    #[test]
    fn same_seq_replay_keeps_count() {
        let mut s = UnreadStore::new();
        s.on_completed("s1", 10, 1, 1000);
        // 同 seq 重放（刷新/重连/重复传输）
        assert_eq!(s.on_completed("s1", 10, 1, 2000), CompleteOutcome::Duplicate);
        assert_eq!(s.unread_count(), 1);
    }

    #[test]
    fn distinct_sessions_count_independently() {
        let mut s = UnreadStore::new();
        s.on_completed("s1", 10, 1, 1000);
        s.on_completed("s2", 20, 1, 1000);
        s.on_completed("s3", 30, 1, 1000);
        assert_eq!(s.unread_count(), 3);
    }

    #[test]
    fn viewed_removes_only_that_session() {
        let mut s = UnreadStore::new();
        s.on_completed("s1", 10, 1, 1000);
        s.on_completed("s2", 20, 1, 1000);
        assert_eq!(s.on_viewed("s1"), ViewedOutcome::Cleared);
        assert_eq!(s.unread_count(), 1);
        assert!(!s.is_unread("s1"));
        assert!(s.is_unread("s2"));
        // 再次查看同一 session：Noop
        assert_eq!(s.on_viewed("s1"), ViewedOutcome::Noop);
    }

    #[test]
    fn new_completion_cycle_after_read_reunreads() {
        let mut s = UnreadStore::new();
        s.on_completed("s1", 10, 1, 1000);
        s.on_viewed("s1");
        assert_eq!(s.unread_count(), 0);
        // 已读 session 开始新完成周期（新 seq/turn）并再次完成
        assert_eq!(s.on_completed("s1", 40, 2, 2000), CompleteOutcome::NewUnread);
        assert_eq!(s.unread_count(), 1);
    }

    #[test]
    fn completion_replaces_older_cycle() {
        // 同一 session 在未读状态下进入新一轮完成：新周期覆盖旧周期，仍计 1 个
        let mut s = UnreadStore::new();
        s.on_completed("s1", 10, 1, 1000);
        assert_eq!(s.on_completed("s1", 40, 2, 2000), CompleteOutcome::NewUnread);
        assert_eq!(s.unread_count(), 1);
        let entry = s.unread.get("s1").unwrap();
        assert_eq!(entry.seq, 40);
        assert_eq!(entry.turn, 2);
        assert_eq!(entry.first_completed_at, 2000);
    }

    #[test]
    fn persistence_roundtrip() {
        let path = tmp_path("roundtrip.json");
        let _ = fs::remove_file(&path);
        {
            let mut s = UnreadStore::new();
            s.on_completed("s1", 10, 1, 1000);
            s.on_completed("s2", 20, 1, 1000);
            s.on_viewed("s1");
            s.save(&path).unwrap();
        }
        {
            let s = UnreadStore::load(&path);
            assert_eq!(s.unread_count(), 1);
            assert!(!s.is_unread("s1"));
            assert!(s.is_unread("s2"));
            let entry = s.unread.get("s2").unwrap();
            assert_eq!(entry.seq, 20);
            assert_eq!(entry.first_completed_at, 1000);
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_file_loads_empty() {
        let s = UnreadStore::load(&tmp_path("nope.json"));
        assert_eq!(s.unread_count(), 0);
    }

    #[test]
    fn corrupt_file_loads_empty() {
        let path = tmp_path("corrupt.json");
        fs::write(&path, "not json {").unwrap();
        let s = UnreadStore::load(&path);
        assert_eq!(s.unread_count(), 0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn bom_prefixed_file_still_loads() {
        // 外部工具写出的状态文件可能带 UTF-8 BOM；必须仍能恢复未读集合。
        let path = tmp_path("bom.json");
        let body = r#"{"schema_version":1,"unread":{"s1":{"seq":10,"turn":1,"first_completed_at":1000,"read_at":null}}}"#;
        fs::write(&path, format!("\u{feff}{body}")).unwrap();
        let s = UnreadStore::load(&path);
        assert_eq!(s.unread_count(), 1);
        assert!(s.is_unread("s1"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn unknown_schema_loads_empty() {        let path = tmp_path("schema.json");
        fs::write(
            &path,
            r#"{"schema_version":999,"unread":{"s1":{"seq":10,"turn":1,"first_completed_at":1000,"read_at":null}}}"#,
        )
        .unwrap();
        let s = UnreadStore::load(&path);
        assert_eq!(s.unread_count(), 0);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_is_atomic_temp_rename() {
        let path = tmp_path("atomic.json");
        let _ = fs::remove_file(&path);
        let mut s = UnreadStore::new();
        s.on_completed("s1", 10, 1, 1000);
        s.save(&path).unwrap();
        // 原子写：不应残留临时文件
        assert!(!path.with_extension("tmp").exists());
        let _ = fs::remove_file(&path);
    }
}
