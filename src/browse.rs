//! 浏览与检索的纯逻辑（G2）：列、排序、查询、选中、树投影与定位。
//!
//! 只消费 [`SessionEntry`](crate::session::SessionEntry) 快照，不读写包文件。

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use crate::session::SessionEntry;

/// 包工作区的两种浏览视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Tree,
    List,
}

/// 列表与树的九列。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Column {
    Name,
    Type,
    Org,
    Size,
    Compression,
    Attrib,
    Offset,
    Mtime,
    Ratio,
}

impl Column {
    pub const ALL: [Column; 9] = [
        Column::Name,
        Column::Type,
        Column::Org,
        Column::Size,
        Column::Compression,
        Column::Attrib,
        Column::Offset,
        Column::Mtime,
        Column::Ratio,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Column::Name => "名称",
            Column::Type => "类型",
            Column::Org => "原始大小",
            Column::Size => "存储大小",
            Column::Compression => "压缩",
            Column::Attrib => "attrib",
            Column::Offset => "offset",
            Column::Mtime => "mtime",
            Column::Ratio => "存储比",
        }
    }

    /// 类型只有「文件/目录」两值，而目录恒排在文件前，排序不会改变顺序。
    pub fn sortable(self) -> bool {
        self != Column::Type
    }

    pub fn default_width(self) -> i32 {
        match self {
            Column::Name => 320,
            Column::Type => 60,
            Column::Org => 100,
            Column::Size => 100,
            Column::Compression => 80,
            Column::Attrib => 90,
            Column::Offset => 90,
            Column::Mtime => 120,
            Column::Ratio => 70,
        }
    }
}

/// 列默认是否显示；与规格 §2.4 的默认列一致。
pub fn default_visible(column: Column, mode: ViewMode) -> bool {
    match column {
        Column::Name | Column::Org | Column::Size | Column::Compression | Column::Mtime => true,
        Column::Type => mode == ViewMode::List,
        Column::Attrib | Column::Offset | Column::Ratio => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub key: Column,
    pub dir: SortDir,
}

/// 每个标签独立的视图状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewState {
    pub mode: ViewMode,
    pub sort: SortSpec,
    pub tree: TreeState,
    pub query: Query,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            mode: ViewMode::Tree,
            sort: SortSpec {
                key: Column::Name,
                dir: SortDir::Asc,
            },
            tree: TreeState::new(),
            query: Query::default(),
        }
    }
}

/// 返回排序后的条目下标；目录恒在文件前，同值按路径升序稳定。
pub fn sorted_indices(entries: &[SessionEntry], spec: SortSpec) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..entries.len()).collect();
    indices.sort_by(|&a, &b| {
        let (ea, eb) = (&entries[a], &entries[b]);
        let by_kind = eb.is_dir().cmp(&ea.is_dir());
        if by_kind != Ordering::Equal {
            return by_kind;
        }
        compare_key(ea, eb, spec.key, spec.dir).then_with(|| ea.path.cmp(&eb.path))
    });
    indices
}

fn compare_key(a: &SessionEntry, b: &SessionEntry, key: Column, dir: SortDir) -> Ordering {
    let ord = match key {
        Column::Name => a.path.cmp(&b.path),
        // 非可排序列（见 [`Column::sortable`]）：保持路径兜底顺序。
        Column::Type => Ordering::Equal,
        Column::Org => a.org.cmp(&b.org),
        Column::Size => a.size.cmp(&b.size),
        Column::Compression => a.is_raw().cmp(&b.is_raw()),
        Column::Attrib => a.attrib.cmp(&b.attrib),
        Column::Offset => a.offset.cmp(&b.offset),
        Column::Mtime => a.mtime.cmp(&b.mtime),
        Column::Ratio => return compare_ratio(a, b, dir),
    };
    if dir == SortDir::Desc {
        ord.reverse()
    } else {
        ord
    }
}

fn compare_ratio(a: &SessionEntry, b: &SessionEntry, dir: SortDir) -> Ordering {
    match (ratio_parts(a), ratio_parts(b)) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some((sa, oa)), Some((sb, ob))) => {
            let ord = (sa * ob).cmp(&(sb * oa));
            if dir == SortDir::Desc {
                ord.reverse()
            } else {
                ord
            }
        }
    }
}

fn ratio_parts(entry: &SessionEntry) -> Option<(u64, u64)> {
    if entry.org == 0 {
        None
    } else {
        Some((entry.size as u64, entry.org as u64))
    }
}

/// 搜索范围：整个包或某个目录子树。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Package,
    Directory(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFilter {
    All,
    Files,
    Dirs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionFilter {
    All,
    Lzss,
    Raw,
}

/// 搜索条件；空关键字等价于不过滤（与 CLI `find` 一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub keyword: String,
    pub scope: Scope,
    pub kind: KindFilter,
    pub compression: CompressionFilter,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            keyword: String::new(),
            scope: Scope::Package,
            kind: KindFilter::All,
            compression: CompressionFilter::All,
        }
    }
}

impl Query {
    /// 返回命中条目的下标，保持目录表顺序。
    pub fn apply(&self, entries: &[SessionEntry]) -> Vec<usize> {
        let keyword = self.keyword.to_lowercase();
        entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.matches(entry, &keyword))
            .map(|(index, _)| index)
            .collect()
    }

    fn matches(&self, entry: &SessionEntry, keyword: &str) -> bool {
        if !entry.path.to_lowercase().contains(keyword) {
            return false;
        }
        if let Scope::Directory(dir) = &self.scope {
            let dir = dir.trim_matches('/');
            if !dir.is_empty() && entry.path != dir && !entry.path.starts_with(&format!("{dir}/")) {
                return false;
            }
        }
        match self.kind {
            KindFilter::All => {}
            KindFilter::Files if entry.is_dir() => return false,
            KindFilter::Dirs if !entry.is_dir() => return false,
            _ => {}
        }
        if self.compression != CompressionFilter::All {
            if entry.is_dir() {
                return false;
            }
            let want_raw = self.compression == CompressionFilter::Raw;
            if entry.is_raw() != want_raw {
                return false;
            }
        }
        true
    }
}

/// 条目选中状态；`primary` 是详情面板焦点，`anchor` 是 Shift 连选的固定端。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Selection {
    primary: Option<usize>,
    anchor: Option<usize>,
    items: BTreeSet<usize>,
}

impl Selection {
    pub fn primary(&self) -> Option<usize> {
        self.primary
    }

    pub fn contains(&self, index: usize) -> bool {
        self.items.contains(&index)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> impl Iterator<Item = usize> + '_ {
        self.items.iter().copied()
    }

    pub fn select_one(&mut self, index: usize) {
        self.items.clear();
        self.items.insert(index);
        self.primary = Some(index);
        self.anchor = Some(index);
    }

    /// Ctrl 单击：切换单项，并把主选中移到该项。
    pub fn toggle(&mut self, index: usize) {
        if self.items.remove(&index) {
            if self.primary == Some(index) {
                self.primary = self.items.iter().next_back().copied();
            }
            if self.anchor == Some(index) {
                self.anchor = self.primary;
            }
        } else {
            self.items.insert(index);
            self.primary = Some(index);
            self.anchor = Some(index);
        }
    }

    /// Shift 单击：从锚点扩展到目标（以当前可见顺序为范围）；锚点保持不变。
    pub fn extend_to(&mut self, visible: &[usize], target: usize) {
        let Some(target_pos) = visible.iter().position(|&i| i == target) else {
            return;
        };
        let anchor = self
            .anchor
            .and_then(|a| visible.iter().position(|&i| i == a))
            .unwrap_or(target_pos);
        let (lo, hi) = if anchor <= target_pos {
            (anchor, target_pos)
        } else {
            (target_pos, anchor)
        };
        self.items.clear();
        self.items.extend(visible[lo..=hi].iter().copied());
        self.primary = Some(target);
    }

    /// 全选当前可见项；不可见的已选项会被清除。
    pub fn select_all(&mut self, visible: &[usize]) {
        self.items.clear();
        self.items.extend(visible.iter().copied());
        self.primary = self
            .primary
            .filter(|p| self.items.contains(p))
            .or_else(|| visible.first().copied());
        self.anchor = self.primary;
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.primary = None;
        self.anchor = None;
    }

    /// 方向键移动主选中并重置为单选；到边界后夹住。
    pub fn move_primary(&mut self, visible: &[usize], step: i32) {
        if visible.is_empty() {
            return;
        }
        let current = self
            .primary
            .and_then(|p| visible.iter().position(|&i| i == p));
        let next = match current {
            Some(pos) => (pos as i32 + step).clamp(0, visible.len() as i32 - 1) as usize,
            None if step < 0 => visible.len() - 1,
            None => 0,
        };
        self.select_one(visible[next]);
    }
}

/// 树中的一行：条目下标、缩进层级、是否含子项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    pub index: usize,
    pub depth: usize,
    pub has_children: bool,
}

/// 树的展开状态，按包内路径保存；首层目录默认展开。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TreeState {
    expanded: BTreeSet<String>,
    /// 显式折叠的目录；首层靠它覆盖「默认展开」。
    collapsed: BTreeSet<String>,
}

impl TreeState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_expanded(&self, path: &str) -> bool {
        !path.is_empty()
            && (self.expanded.contains(path)
                || (!path.contains('/') && !self.collapsed.contains(path)))
    }

    pub fn expand(&mut self, path: &str) {
        if !path.is_empty() {
            self.expanded.insert(path.to_string());
            self.collapsed.remove(path);
        }
    }

    pub fn collapse(&mut self, path: &str) {
        self.expanded.remove(path);
        self.collapsed.insert(path.to_string());
    }

    pub fn toggle(&mut self, path: &str) {
        if self.is_expanded(path) {
            self.collapse(path);
        } else {
            self.expand(path);
        }
    }

    pub fn expand_all(&mut self, entries: &[SessionEntry]) {
        for entry in entries.iter().filter(|e| e.is_dir()) {
            self.expand(&entry.path);
        }
    }

    pub fn collapse_all(&mut self, entries: &[SessionEntry]) {
        self.expanded.clear();
        for entry in entries.iter().filter(|e| e.is_dir()) {
            self.collapsed.insert(entry.path.clone());
        }
    }

    /// 只返回当前可见行；目录先于文件、同层按路径升序。
    pub fn rows(&self, entries: &[SessionEntry]) -> Vec<TreeRow> {
        let mut children: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let parent = entry
                .path
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_string())
                .unwrap_or_default();
            children.entry(parent).or_default().push(index);
        }
        for list in children.values_mut() {
            list.sort_by(|&a, &b| {
                let (ea, eb) = (&entries[a], &entries[b]);
                eb.is_dir()
                    .cmp(&ea.is_dir())
                    .then_with(|| ea.path.cmp(&eb.path))
            });
        }

        let mut rows = Vec::new();
        render_rows("", 0, &children, entries, self, &mut rows);
        rows
    }
}

fn render_rows(
    parent: &str,
    depth: usize,
    children: &HashMap<String, Vec<usize>>,
    entries: &[SessionEntry],
    tree: &TreeState,
    rows: &mut Vec<TreeRow>,
) {
    let Some(list) = children.get(parent) else {
        return;
    };
    for &index in list {
        let entry = &entries[index];
        let has_children = children
            .get(&entry.path)
            .map(|c| !c.is_empty())
            .unwrap_or(false);
        rows.push(TreeRow {
            index,
            depth,
            has_children,
        });
        if entry.is_dir() && has_children && tree.is_expanded(&entry.path) {
            render_rows(&entry.path, depth + 1, children, entries, tree, rows);
        }
    }
}

/// 精确定位包内路径（允许首尾 `/` 与 `\` 分隔）。
pub fn locate(entries: &[SessionEntry], target: &str) -> Option<usize> {
    let target = normalize_inner(target);
    entries.iter().position(|entry| entry.path == target)
}

/// 精确定位失败时，逐级回退到最近的已存在父目录；都不存在则返回 None。
pub fn locate_nearest(entries: &[SessionEntry], target: &str) -> Option<usize> {
    let mut target = normalize_inner(target);
    if let Some(index) = entries.iter().position(|entry| entry.path == target) {
        return Some(index);
    }
    loop {
        let (parent, _) = target.rsplit_once('/')?;
        target = parent.to_string();
        // 父级必须是目录：同名文件不能当兜底目标选中。
        if let Some(index) = entries
            .iter()
            .position(|entry| entry.path == target && entry.is_dir())
        {
            return Some(index);
        }
    }
}

/// 定位来源，仅用于区分搜索、校验与任务发起的请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocateSource {
    Search,
    Validation,
    Task,
}

/// 一次定位请求；路径已归一，默认带 2 秒高亮标记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocateRequest {
    pub path: String,
    pub source: LocateSource,
    pub highlight: bool,
}

impl LocateRequest {
    /// 空路径不产生请求；反斜杠与首尾斜杠按包内路径归一。
    pub fn new(path: &str, source: LocateSource) -> Option<Self> {
        let path = normalize_inner(path);
        if path.is_empty() {
            return None;
        }
        Some(Self {
            path,
            source,
            highlight: true,
        })
    }
}

/// 同一路径只保留最早的一次请求；来源不参与去重。
pub fn dedupe_requests(requests: &[LocateRequest]) -> Vec<LocateRequest> {
    let mut seen = BTreeSet::new();
    requests
        .iter()
        .filter(|request| seen.insert(request.path.clone()))
        .cloned()
        .collect()
}

fn normalize_inner(target: &str) -> String {
    target.replace('\\', "/").trim_matches('/').to_string()
}
