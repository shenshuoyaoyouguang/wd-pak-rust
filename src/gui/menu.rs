//! 菜单与命令栏纯逻辑（G6-01/02）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuId {
    FileOpen,
    FilePack,
    FileExtract,
    FileReplace,
    FileCheck,
    FileClose,
    FileExit,
    EditCopyPath,
    EditFind,
    EditSelectAll,
    ViewTree,
    ViewList,
    ViewLog,
    ViewNotifications,
    ViewRefresh,
    ToolsOptions,
    ToolsOpenLogDir,
    ToolsSelfCheck,
    HelpContents,
    HelpAbout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: MenuId,
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub separator_after: bool,
}

impl MenuItem {
    pub fn new(id: MenuId, label: &str) -> Self {
        Self {
            id,
            label: label.to_string(),
            shortcut: None,
            enabled: true,
            separator_after: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MenuBar {
    pub items: Vec<MenuItem>,
}

impl MenuBar {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, item: MenuItem) {
        self.items.push(item);
    }

    pub fn set_enabled(&mut self, id: MenuId, enabled: bool) {
        for it in &mut self.items {
            if it.id == id {
                it.enabled = enabled;
            }
        }
    }

    pub fn find(&self, id: MenuId) -> Option<&MenuItem> {
        self.items.iter().find(|i| i.id == id)
    }
}

/// 菜单栏可见性（§6.1：默认隐藏，Alt 唤起，设置中可固定显示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuState {
    pinned: bool,
    summoned: bool,
}

impl MenuState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(&self) -> bool {
        self.pinned || self.summoned
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    /// 固定显示时 Alt 不改变可见性。
    pub fn toggle_by_alt(&mut self) {
        if self.pinned {
            return;
        }
        self.summoned = !self.summoned;
    }

    pub fn summon(&mut self) {
        self.summoned = true;
    }

    pub fn dismiss(&mut self) {
        self.summoned = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandBarMode {
    #[default]
    NoPackage,
    Package,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBarItem {
    pub id: MenuId,
    pub label: String,
    pub enabled: bool,
}

impl CommandBarItem {
    pub fn new(id: MenuId, label: &str, enabled: bool) -> Self {
        Self {
            id,
            label: label.to_string(),
            enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_enable_toggle() {
        let mut bar = MenuBar::new();
        bar.push(MenuItem::new(MenuId::FileOpen, "打开"));
        bar.set_enabled(MenuId::FileOpen, false);
        assert!(!bar.find(MenuId::FileOpen).unwrap().enabled);
    }

    #[test]
    fn menu_bar_hidden_by_default() {
        let state = MenuState::new();
        assert!(!state.visible());
    }

    #[test]
    fn alt_summon_then_dismiss() {
        let mut state = MenuState::new();
        state.toggle_by_alt();
        assert!(state.visible());
        state.toggle_by_alt();
        assert!(!state.visible());
    }

    #[test]
    fn ellipsis_summon_and_esc_dismiss() {
        let mut state = MenuState::new();
        state.summon();
        assert!(state.visible());
        state.dismiss();
        assert!(!state.visible());
    }

    #[test]
    fn pinned_menu_stays_visible_across_alt_and_esc() {
        let mut state = MenuState::new();
        state.set_pinned(true);
        assert!(state.visible());

        state.toggle_by_alt();
        assert!(state.visible());
        state.dismiss();
        assert!(state.visible());

        state.set_pinned(false);
        assert!(!state.visible());
    }
}
