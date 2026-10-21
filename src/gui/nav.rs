//! 导航模型与页面容器（G0-02）。
//!
//! 只定义导航项与折叠状态，不涉及 Win32 控件。

/// 导航项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Workbench,
    Recent,
    Tasks,
    Notifications,
    Settings,
    Help,
    AccountLicense,
}

impl NavItem {
    pub fn label(self) -> &'static str {
        match self {
            NavItem::Workbench => "工作台",
            NavItem::Recent => "最近打开",
            NavItem::Tasks => "任务中心",
            NavItem::Notifications => "通知",
            NavItem::Settings => "设置",
            NavItem::Help => "帮助",
            NavItem::AccountLicense => "账户与许可",
        }
    }

    pub fn all() -> &'static [NavItem] {
        &[
            NavItem::Workbench,
            NavItem::Recent,
            NavItem::Tasks,
            NavItem::Notifications,
            NavItem::Settings,
            NavItem::Help,
            NavItem::AccountLicense,
        ]
    }
}

/// 导航状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavState {
    pub collapsed: bool,
    pub selected: NavItem,
    pub width: i32, // DIP: 220 或 48
}

impl NavState {
    pub fn new(collapsed: bool) -> Self {
        Self {
            collapsed,
            selected: NavItem::Workbench,
            width: if collapsed { 48 } else { 220 },
        }
    }

    pub fn toggle_collapsed(&mut self) {
        self.collapsed = !self.collapsed;
        self.width = if self.collapsed { 48 } else { 220 };
    }

    pub fn select(&mut self, item: NavItem) {
        self.selected = item;
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

/// 目录历史（后退/前进栈）。
#[derive(Debug, Default, Clone)]
pub struct DirHistory {
    pub entries: Vec<String>,
    pub pos: usize,
}

impl DirHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, path: String) {
        if self.entries.get(self.pos) == Some(&path) {
            return;
        }
        // 截断前进历史
        self.entries.truncate(self.pos + 1);
        self.entries.push(path);
        self.pos = self.entries.len() - 1;
    }

    pub fn can_go_back(&self) -> bool {
        self.pos > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.pos + 1 < self.entries.len()
    }

    pub fn go_back(&mut self) -> Option<&str> {
        if self.can_go_back() {
            self.pos -= 1;
            self.entries.get(self.pos).map(|s| s.as_str())
        } else {
            None
        }
    }

    pub fn go_forward(&mut self) -> Option<&str> {
        if self.can_go_forward() {
            self.pos += 1;
            self.entries.get(self.pos).map(|s| s.as_str())
        } else {
            None
        }
    }

    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.pos).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_toggle() {
        let mut nav = NavState::new(false);
        assert_eq!(nav.width, 220);
        nav.toggle_collapsed();
        assert!(nav.is_collapsed());
        assert_eq!(nav.width, 48);
    }

    #[test]
    fn nav_select() {
        let mut nav = NavState::new(false);
        nav.select(NavItem::Settings);
        assert_eq!(nav.selected, NavItem::Settings);
    }

    #[test]
    fn nav_items_match_spec_order_and_count() {
        let labels: Vec<&str> = NavItem::all().iter().map(|item| item.label()).collect();
        assert_eq!(
            labels,
            vec![
                "工作台",
                "最近打开",
                "任务中心",
                "通知",
                "设置",
                "帮助",
                "账户与许可",
            ]
        );
        assert_eq!(NavItem::all().len(), 7);
    }

    #[test]
    fn dir_history_back_forward() {
        let mut h = DirHistory::new();
        h.push("a".into());
        h.push("b".into());
        h.push("c".into());
        assert!(h.can_go_back());
        assert_eq!(h.go_back(), Some("b"));
        assert_eq!(h.go_back(), Some("a"));
        assert!(!h.can_go_back());
        assert!(h.can_go_forward());
        assert_eq!(h.go_forward(), Some("b"));
    }

    #[test]
    fn dir_history_no_duplicate_consecutive() {
        let mut h = DirHistory::new();
        h.push("a".into());
        h.push("a".into());
        assert_eq!(h.entries.len(), 1);
    }

    #[test]
    fn dir_history_push_current_keeps_forward() {
        let mut h = DirHistory::new();
        h.push("a".into());
        h.push("b".into());
        h.push("c".into());
        assert_eq!(h.go_back(), Some("b"));
        h.push("b".into());
        assert!(h.can_go_forward());
        assert_eq!(h.entries, vec!["a", "b", "c"]);
    }

    #[test]
    fn dir_history_truncates_forward() {
        let mut h = DirHistory::new();
        h.push("a".into());
        h.push("b".into());
        h.push("c".into());
        h.go_back();
        h.push("d".into());
        assert!(!h.can_go_forward());
        assert_eq!(h.entries, vec!["a", "b", "d"]);
    }
}
