//! 工作台与启动页状态模型（G1-01）。
//!
//! 只定义工作台的卡片与启动模式，不涉及 Win32 控件。

use crate::gui::self_check::{CheckItem, CheckState};

/// 工作台卡片。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchCard {
    OpenPak,
    PackFromDir,
    DropFiles,
}

impl WorkbenchCard {
    pub fn title(self) -> &'static str {
        match self {
            WorkbenchCard::OpenPak => "打开 PAK",
            WorkbenchCard::PackFromDir => "从目录打包",
            WorkbenchCard::DropFiles => "拖入文件",
        }
    }
}

/// 启动模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartupMode {
    #[default]
    Workbench,
    FirstRun,
    Restore,
}

/// 工作台状态。
#[derive(Debug, Default, Clone)]
pub struct WorkbenchState {
    pub startup: StartupMode,
    pub cards: Vec<WorkbenchCard>,
    pub self_checks: Vec<CheckItem>,
    pub version: String,
}

impl WorkbenchState {
    pub fn new(version: &str) -> Self {
        Self {
            startup: StartupMode::Workbench,
            cards: vec![
                WorkbenchCard::OpenPak,
                WorkbenchCard::PackFromDir,
                WorkbenchCard::DropFiles,
            ],
            self_checks: Vec::new(),
            version: version.to_string(),
        }
    }

    pub fn is_first_run(&self) -> bool {
        self.startup == StartupMode::FirstRun
    }

    pub fn add_self_check(&mut self, name: &str, state: CheckState, detail: &str) {
        self.self_checks.push(CheckItem::new(name, state, detail));
    }

    pub fn has_failed_checks(&self) -> bool {
        self.self_checks
            .iter()
            .any(|c| c.state == CheckState::Failed)
    }

    pub fn failed_count(&self) -> usize {
        self.self_checks
            .iter()
            .filter(|c| c.state == CheckState::Failed)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_workbench_has_three_cards() {
        let wb = WorkbenchState::new("1.0");
        assert_eq!(wb.cards.len(), 3);
        assert_eq!(wb.cards[0], WorkbenchCard::OpenPak);
        assert_eq!(wb.cards[1], WorkbenchCard::PackFromDir);
        assert_eq!(wb.cards[2], WorkbenchCard::DropFiles);
        assert_eq!(wb.version, "1.0");
    }

    #[test]
    fn first_run_flag() {
        let mut wb = WorkbenchState::new("1.0");
        wb.startup = StartupMode::FirstRun;
        assert!(wb.is_first_run());
    }

    #[test]
    fn self_check_tracking() {
        let mut wb = WorkbenchState::new("1.0");
        wb.add_self_check("目录可写", CheckState::Ok, "OK");
        wb.add_self_check("长路径", CheckState::Failed, "未启用");
        assert!(wb.has_failed_checks());
        assert_eq!(wb.failed_count(), 1);
    }

    #[test]
    fn no_failed_checks() {
        let mut wb = WorkbenchState::new("1.0");
        wb.add_self_check("目录可写", CheckState::Ok, "OK");
        assert!(!wb.has_failed_checks());
        assert_eq!(wb.failed_count(), 0);
    }
}
