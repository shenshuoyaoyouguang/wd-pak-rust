//! 配置与会话快照的 JSON 读写（G0-06 / G1-06.4）。
//!
//! 只负责路径拼装与文件 I/O；配置结构的校验仍由 [`crate::config`] 负责。
//! 写入采用临时文件 + 改名，损坏或不可写时由调用方决定回退方式。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::{Config, LoadOutcome};

pub const SESSION_VERSION: u32 = 1;

/// 配置文件加载状态：缺失、存在但不可读（只读模式）、正常。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLoadState {
    Missing,
    ReadOnly,
    Ok,
}

/// 单个标签的会话快照；只存路径与浏览状态，不存包内容。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TabSnapshot {
    pub path: String,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub selected: Option<String>,
}

/// 会话快照；`active` 越界时由恢复方收敛。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionSnapshot {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub tabs: Vec<TabSnapshot>,
}

impl SessionSnapshot {
    pub fn from_json(text: &str) -> Option<Self> {
        let snapshot = serde_json::from_str::<Self>(text).ok()?;
        if snapshot.version != SESSION_VERSION {
            return None;
        }
        Some(snapshot)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }
}

/// 配置与会话文件的默认位置。
pub fn paths() -> (PathBuf, PathBuf) {
    let directory = crate::platform::app_config_dir();
    (directory.join("config.json"), directory.join("session.json"))
}

/// 读取配置；返回加载状态（缺失 / 只读 / 正常），供首次运行与只读模式判定。
pub fn load_config(path: &Path) -> (Config, Option<String>, ConfigLoadState) {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (config, outcome) = Config::from_json(&text);
            let reason = match outcome {
                LoadOutcome::Ok => None,
                LoadOutcome::FallbackDefault(reason) | LoadOutcome::ReadOnly(reason) => {
                    Some(reason)
                }
            };
            (config, reason, ConfigLoadState::Ok)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (Config::default(), None, ConfigLoadState::Missing)
        }
        Err(error) => (
            Config::default(),
            Some(format!("读取配置失败: {error}")),
            ConfigLoadState::ReadOnly,
        ),
    }
}

pub fn save_config(path: &Path, config: &Config) -> std::io::Result<()> {
    write_atomic(path, &config.to_json())
}

pub fn load_session(path: &Path) -> Option<SessionSnapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    SessionSnapshot::from_json(&text)
}

pub fn save_session(path: &Path, snapshot: &SessionSnapshot) -> std::io::Result<()> {
    write_atomic(path, &snapshot.to_json())
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temp_path(path);
    std::fs::write(&temporary, text)?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Windows 上目标存在时改名失败；复制会覆盖旧文件，失败则保留旧文件。
            match std::fs::copy(&temporary, path) {
                Ok(_) => {
                    let _ = std::fs::remove_file(&temporary);
                    Ok(())
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&temporary);
                    Err(error)
                }
            }
        }
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.json".into());
    path.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("wd-pak-{name}-{}-{stamp}.json", std::process::id()))
    }

    #[test]
    fn session_snapshot_roundtrip_and_version_guard() {
        let snapshot = SessionSnapshot {
            version: SESSION_VERSION,
            active: 1,
            tabs: vec![
                TabSnapshot {
                    path: r"C:\tmp\a.pak".into(),
                    filter: "list".into(),
                    selected: Some("basic_book.list".into()),
                },
                TabSnapshot {
                    path: r"C:\tmp\b.pak".into(),
                    filter: String::new(),
                    selected: None,
                },
            ],
        };
        let text = snapshot.to_json();
        assert_eq!(SessionSnapshot::from_json(&text), Some(snapshot));
        assert!(SessionSnapshot::from_json(r#"{"version":99,"tabs":[]}"#).is_none());
    }

    #[test]
    fn config_roundtrip_reports_existing_file() {
        let path = unique("config");
        let (missing, reason, state) = load_config(&path);
        assert_eq!(state, ConfigLoadState::Missing);
        assert!(reason.is_none());
        assert_eq!(missing, Config::default());

        let mut config = Config::default();
        config.ui.recent_limit = 7;
        config.touch_recent(r"C:\tmp\a.pak", 42);
        save_config(&path, &config).unwrap();

        let (loaded, reason, state) = load_config(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(state, ConfigLoadState::Ok);
        assert!(reason.is_none());
        assert_eq!(loaded.ui.recent_limit, 7);
        assert_eq!(loaded.recent.len(), 1);
    }

    #[test]
    fn config_load_state_read_only_for_unreadable_existing_path() {
        let path = unique("config-dir");
        std::fs::create_dir(&path).unwrap();
        let (config, reason, state) = load_config(&path);
        let _ = std::fs::remove_dir(&path);
        assert_eq!(state, ConfigLoadState::ReadOnly);
        assert!(reason.is_some());
        assert_eq!(config, Config::default());
    }

    #[test]
    fn save_config_overwrites_existing_file() {
        let path = unique("config-overwrite");
        let mut config = Config::default();
        save_config(&path, &config).unwrap();
        config.ui.recent_limit = 3;
        save_config(&path, &config).unwrap();

        let (loaded, reason, state) = load_config(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(state, ConfigLoadState::Ok);
        assert!(reason.is_none());
        assert_eq!(loaded.ui.recent_limit, 3);
    }
}
