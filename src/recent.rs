//! 最近打开列表模型（G1-02.1）。
//!
//! 只负责排序、去重与固定；不探测文件系统，也不格式化时间。

use std::path::{Path, PathBuf};

pub const DEFAULT_LIMIT: usize = 10;
pub const MAX_LIMIT: usize = 20;

/// 一条最近打开记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub entry_count: Option<usize>,
    pub last_opened: u64,
    pub pinned: bool,
}

/// 最近打开列表；固定项始终排在未固定项之前。
#[derive(Debug)]
pub struct RecentList {
    limit: usize,
    items: Vec<RecentEntry>,
}

impl RecentList {
    /// 上限按规格收敛到 0–20；默认值见 [`DEFAULT_LIMIT`]。
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.min(MAX_LIMIT),
            items: Vec::new(),
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn items(&self) -> &[RecentEntry] {
        &self.items
    }

    /// 记录一次打开；同一路径去重并刷新元数据与时间。
    /// 固定状态沿用原记录，避免刷新时间时被取消固定。
    pub fn record(&mut self, mut entry: RecentEntry) {
        if self.limit == 0 {
            return;
        }
        if let Some(existing) = self.items.iter().find(|e| e.path == entry.path) {
            entry.pinned = existing.pinned;
        }
        self.items.retain(|e| e.path != entry.path);
        self.items.push(entry);
        self.sort();
        self.enforce_limit();
    }

    /// 固定或取消固定；路径不存在时返回 false。
    pub fn set_pinned(&mut self, path: &Path, pinned: bool) -> bool {
        let Some(entry) = self.items.iter_mut().find(|e| e.path == path) else {
            return false;
        };
        entry.pinned = pinned;
        self.sort();
        true
    }

    /// 移除一条记录；路径不存在时返回 false。
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.items.len();
        self.items.retain(|e| e.path != path);
        self.items.len() != before
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// 上限是硬限：优先淘汰最久未打开的未固定项；全部固定时淘汰最久未打开的固定项。
    fn enforce_limit(&mut self) {
        while self.items.len() > self.limit {
            let Some(index) = self
                .items
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| (e.pinned, e.last_opened))
                .map(|(index, _)| index)
            else {
                break;
            };
            self.items.remove(index);
        }
    }

    fn sort(&mut self) {
        self.items.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.last_opened.cmp(&a.last_opened))
                .then_with(|| a.path.cmp(&b.path))
        });
    }
}
