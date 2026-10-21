//! 深层定位状态模型（G2-06）。
//!
//! 只定义定位后的高亮状态，不涉及 Win32 控件。

use crate::session::SessionEntry;

/// 在条目集合中查找定位目标；目标不存在时回退到最近的已存在父目录。
pub fn nearest_entry_index(entries: &[SessionEntry], path: &str) -> Option<usize> {
    if let Some(index) = entries.iter().position(|entry| entry.path == path) {
        return Some(index);
    }
    let mut cursor = path;
    while let Some(position) = cursor.rfind('/') {
        cursor = &cursor[..position];
        if let Some(index) = entries
            .iter()
            .position(|entry| entry.path == cursor && entry.is_dir())
        {
            return Some(index);
        }
    }
    None
}

/// 高亮状态：定位后高亮 N 秒。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightState {
    pub path: String,
    pub remaining_ms: u32,
}

impl HighlightState {
    pub fn new(path: &str, duration_ms: u32) -> Self {
        Self {
            path: path.to_string(),
            remaining_ms: duration_ms,
        }
    }

    pub fn tick(&mut self, elapsed_ms: u32) {
        self.remaining_ms = self.remaining_ms.saturating_sub(elapsed_ms);
    }

    pub fn is_active(&self) -> bool {
        self.remaining_ms > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_ticks_down() {
        let mut h = HighlightState::new("x.txt", 2000);
        assert!(h.is_active());
        h.tick(500);
        assert!(h.is_active());
        assert_eq!(h.remaining_ms, 1500);
        h.tick(2000);
        assert!(!h.is_active());
        assert_eq!(h.remaining_ms, 0);
    }

    fn entry(path: &str, attrib: u32) -> SessionEntry {
        SessionEntry {
            path: path.to_string(),
            attrib,
            offset: 0,
            size: 0,
            org: 0,
            mtime: 0,
        }
    }

    #[test]
    fn nearest_entry_falls_back_to_parent_directory() {
        let entries = vec![
            entry("a", crate::format::DIR_BIT),
            entry("a/b.txt", 0),
            entry("a/c", crate::format::DIR_BIT),
        ];
        assert_eq!(nearest_entry_index(&entries, "a/b.txt"), Some(1));
        assert_eq!(nearest_entry_index(&entries, "a/c/missing.txt"), Some(2));
        assert_eq!(nearest_entry_index(&entries, "a/missing/deep.txt"), Some(0));
        assert_eq!(nearest_entry_index(&entries, "x/y"), None);
    }
}
