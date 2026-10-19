//! 包工作区共享状态；只由 `shell` 修改，各面板只读。

use std::path::PathBuf;

use wd_pak::browse::ViewState;
use wd_pak::session::PakSession;

/// 已打开包的界面侧状态。
pub struct AppState {
    pub session: Option<PakSession>,
    pub path: Option<PathBuf>,
    pub last_dir: Option<PathBuf>,
    /// 当前可见条目的下标（过滤并排序后），与列表行号一一对应。
    pub visible: Vec<usize>,
    /// 视图状态：模式、排序、查询与树展开；后续按标签各持一份。
    pub view: ViewState,
    /// 当前选中的条目下标（不随视图变化）；列表与树都从它派生。
    pub selected: Option<usize>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            session: None,
            path: None,
            last_dir: None,
            visible: Vec::new(),
            view: ViewState::default(),
            selected: None,
        }
    }
}
