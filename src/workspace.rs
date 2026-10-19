//! 工作区状态（G1-03.1 / G1-06.1 / G1-07.1）：打开请求、标签与启动模式。
//!
//! 纯状态模型：不读配置文件、不访问文件系统、不驱动窗口。

use std::path::{Path, PathBuf};

use crate::browse::{Selection, ViewState};

/// 打开请求的来源，仅用于后续统计与文案，不影响打开行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenSource {
    CommandBar,
    Shortcut,
    Workbench,
    Recent,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    pub path: PathBuf,
    pub source: OpenSource,
}

impl OpenRequest {
    /// 空路径不产生请求；其余路径原样保留。
    pub fn new(path: &Path, source: OpenSource) -> Option<Self> {
        if path.as_os_str().is_empty() {
            return None;
        }
        Some(Self {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// 启动落点：首次运行、恢复会话、进入工作台。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Startup {
    FirstRun,
    Restore,
    Workbench,
}

pub fn startup(has_config: bool, has_session: bool, restore_session: bool) -> Startup {
    if !has_config {
        Startup::FirstRun
    } else if restore_session && has_session {
        Startup::Restore
    } else {
        Startup::Workbench
    }
}

/// 一个包标签的独立状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabState {
    /// 私有：标签与路径一一对应，允许改写会破坏 [`Workspace::open`] 的去重前提。
    path: PathBuf,
    pub write_busy: bool,
    pub view: ViewState,
    pub selection: Selection,
    pub history: Vec<String>,
    pub history_pos: usize,
}

impl TabState {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_busy: false,
            view: ViewState::default(),
            selection: Selection::default(),
            history: vec![String::new()],
            history_pos: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// 打开的标签集合；同一路径只对应一个标签。
#[derive(Debug, Default)]
pub struct Workspace {
    tabs: Vec<TabState>,
    active: Option<usize>,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tabs(&self) -> &[TabState] {
        &self.tabs
    }

    pub fn tab(&self, index: usize) -> Option<&TabState> {
        self.tabs.get(index)
    }

    pub fn tab_mut(&mut self, index: usize) -> Option<&mut TabState> {
        self.tabs.get_mut(index)
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    /// 打开路径：已存在则激活并返回原下标，否则新建标签。
    /// 路径在入口处规范化；Windows 上再按大小写折叠比较。
    pub fn open(&mut self, path: PathBuf) -> usize {
        let path = crate::platform::normpath(&path);
        let key = crate::platform::path_key(&path);
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| crate::platform::path_key(&tab.path) == key)
        {
            self.active = Some(index);
            return index;
        }
        self.tabs.push(TabState::new(path));
        let index = self.tabs.len() - 1;
        self.active = Some(index);
        index
    }

    /// 关闭标签；写任务进行中或下标无效时拒绝。
    pub fn close(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(index) else {
            return false;
        };
        if tab.write_busy {
            return false;
        }
        self.tabs.remove(index);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if index < active => Some(active - 1),
            Some(active) if index > active => Some(active),
            Some(_) => Some(index.min(self.tabs.len() - 1)),
            None => None,
        };
        true
    }

    pub fn activate(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        self.active = Some(index);
        true
    }

    pub fn set_write_busy(&mut self, index: usize, busy: bool) -> bool {
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        tab.write_busy = busy;
        true
    }
}


