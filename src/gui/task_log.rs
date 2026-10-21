//! 任务日志纯逻辑（G5-06）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLogEntry {
    pub timestamp: u64,
    pub level: LogLevel,
    pub message: String,
}

impl TaskLogEntry {
    pub fn new(timestamp: u64, level: LogLevel, message: &str) -> Self {
        Self {
            timestamp,
            level,
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskLog {
    pub entries: Vec<TaskLogEntry>,
    pub max_entries: usize,
    pub auto_scroll: bool,
}

impl Default for TaskLog {
    fn default() -> Self {
        Self::new(0)
    }
}

impl TaskLog {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries,
            auto_scroll: true,
        }
    }

    pub fn push(&mut self, entry: TaskLogEntry) {
        self.entries.push(entry);
        if self.max_entries > 0 && self.entries.len() > self.max_entries {
            let excess = self.entries.len() - self.max_entries;
            self.entries.drain(0..excess);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn filter(&self, level: Option<LogLevel>) -> Vec<&TaskLogEntry> {
        self.entries
            .iter()
            .filter(|e| level.map(|l| e.level == l).unwrap_or(true))
            .collect()
    }

    pub fn export_text(&self) -> String {
        self.entries
            .iter()
            .map(|e| format!("[{}] {}", e.timestamp, e.message))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_cap() {
        let mut log = TaskLog::new(2);
        log.push(TaskLogEntry::new(1, LogLevel::Info, "a"));
        log.push(TaskLogEntry::new(2, LogLevel::Info, "b"));
        log.push(TaskLogEntry::new(3, LogLevel::Info, "c"));
        assert_eq!(log.len(), 2);
        assert_eq!(log.entries[0].message, "b");
    }

    #[test]
    fn filter_level() {
        let mut log = TaskLog::new(10);
        log.push(TaskLogEntry::new(1, LogLevel::Info, "a"));
        log.push(TaskLogEntry::new(2, LogLevel::Error, "b"));
        assert_eq!(log.filter(Some(LogLevel::Error)).len(), 1);
        assert_eq!(log.filter(None).len(), 2);
    }

    #[test]
    fn default_matches_new_zero() {
        let default = TaskLog::default();
        let explicit = TaskLog::new(0);
        assert_eq!(default.max_entries, explicit.max_entries);
        assert_eq!(default.auto_scroll, explicit.auto_scroll);
        assert_eq!(default.entries, explicit.entries);
    }

    #[test]
    fn export_text() {
        let mut log = TaskLog::new(10);
        log.push(TaskLogEntry::new(1, LogLevel::Info, "hi"));
        assert_eq!(log.export_text(), "[1] hi");
    }
}
