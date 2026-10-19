//! 环境自检状态模型（G0-07）。
//!
//! 只定义自检项的状态与汇总，不涉及 Win32 API 或文件系统探测。

use crate::diagnostics::{dir_writable, scale_percent};

/// 自检项状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Ok,
    Warn,
    Failed,
}

/// 单个自检项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckItem {
    pub name: String,
    pub state: CheckState,
    pub detail: String,
}

impl CheckItem {
    pub fn new(name: &str, state: CheckState, detail: &str) -> Self {
        Self {
            name: name.to_string(),
            state,
            detail: detail.to_string(),
        }
    }
}

/// 自检汇总。
#[derive(Debug, Default, Clone)]
pub struct SelfCheckSummary {
    pub items: Vec<CheckItem>,
}

impl SelfCheckSummary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, item: CheckItem) {
        self.items.push(item);
    }

    pub fn has_failures(&self) -> bool {
        self.items.iter().any(|i| i.state == CheckState::Failed)
    }

    pub fn has_warnings(&self) -> bool {
        self.items.iter().any(|i| i.state == CheckState::Warn)
    }

    pub fn ok_count(&self) -> usize {
        self.items.iter().filter(|i| i.state == CheckState::Ok).count()
    }

    pub fn failed_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.state == CheckState::Failed)
            .count()
    }

    /// 从目录可写性探测结果构建自检项。
    pub fn probe_dir_writable(&mut self, name: &str, dir: &std::path::Path) {
        let (ok, detail) = dir_writable(dir);
        let state = if ok { CheckState::Ok } else { CheckState::Failed };
        self.add(CheckItem::new(name, state, &detail));
    }

    /// 从 DPI 缩放百分比构建自检项。
    pub fn probe_dpi(&mut self, dpi: u32) {
        let pct = scale_percent(dpi);
        let detail = format!("系统缩放 {}%", pct);
        self.add(CheckItem::new("系统缩放", CheckState::Ok, &detail));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_summary() {
        let s = SelfCheckSummary::new();
        assert!(!s.has_failures());
        assert!(!s.has_warnings());
        assert_eq!(s.ok_count(), 0);
    }

    #[test]
    fn add_and_count() {
        let mut s = SelfCheckSummary::new();
        s.add(CheckItem::new("a", CheckState::Ok, "ok"));
        s.add(CheckItem::new("b", CheckState::Failed, "fail"));
        s.add(CheckItem::new("c", CheckState::Warn, "warn"));
        assert_eq!(s.ok_count(), 1);
        assert_eq!(s.failed_count(), 1);
        assert!(s.has_failures());
        assert!(s.has_warnings());
    }

    #[test]
    fn no_failures() {
        let mut s = SelfCheckSummary::new();
        s.add(CheckItem::new("a", CheckState::Ok, "ok"));
        s.add(CheckItem::new("b", CheckState::Warn, "warn"));
        assert!(!s.has_failures());
        assert!(s.has_warnings());
    }
}
