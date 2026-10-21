//! GUI 状态机（G2.5 / §2.5）：包会话、任务、校验、预检、冲突决策、通知。
//! 纯枚举与转换，便于单元测试；UI 层只读取状态并渲染。

/// 包会话状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Loading,
    Ready,
    /// 解析失败；携带可展示的错误摘要。
    Error(String),
}

impl SessionState {
    pub fn is_ready(&self) -> bool {
        matches!(self, SessionState::Ready)
    }
    pub fn is_loading(&self) -> bool {
        matches!(self, SessionState::Loading)
    }
}

/// 任务状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

impl TaskState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskState::Done | TaskState::Failed(_) | TaskState::Cancelled)
    }
    pub fn is_active(&self) -> bool {
        matches!(self, TaskState::Queued | TaskState::Running)
    }
}

/// 任务类型，决定步骤条与串行判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Export,
    Replace,
    Pack,
    Unpack,
    ShallowCheck,
    DeepCheck,
}

impl TaskKind {
    pub fn is_write(&self) -> bool {
        matches!(self, TaskKind::Replace | TaskKind::Pack | TaskKind::Unpack)
    }

    pub fn label(&self) -> &'static str {
        match self {
            TaskKind::Export => "导出",
            TaskKind::Replace => "替换",
            TaskKind::Pack => "打包",
            TaskKind::Unpack => "解包",
            TaskKind::ShallowCheck => "浅校验",
            TaskKind::DeepCheck => "深度校验",
        }
    }
}

impl TaskState {
    pub fn label(&self) -> String {
        match self {
            TaskState::Queued => "等待中".into(),
            TaskState::Running => "进行中".into(),
            TaskState::Done => "已完成".into(),
            TaskState::Failed(_) => "失败".into(),
            TaskState::Cancelled => "已取消".into(),
        }
    }
}

/// 校验状态（§2.5）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationState {
    Unchecked,
    Running,
    Passed,
    /// 发现问题，携带问题数量。
    Problems(usize),
    Cancelled,
    /// 包文件变化或写入后标为过期。
    Expired,
}

impl ValidationState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ValidationState::Passed
                | ValidationState::Problems(_)
                | ValidationState::Cancelled
                | ValidationState::Expired
        )
    }

    /// 写入成功后使校验结果过期（§2.6 规则 2）。
    pub fn invalidate_on_write(&mut self) {
        match self {
            ValidationState::Passed | ValidationState::Problems(_) | ValidationState::Running => {
                *self = ValidationState::Expired;
            }
            _ => {}
        }
    }

    pub fn complete(&mut self, problems: usize) {
        if *self == ValidationState::Running {
            *self = if problems == 0 {
                ValidationState::Passed
            } else {
                ValidationState::Problems(problems)
            };
        }
    }

    /// 指纹变化后标为过期（§2.6 规则 3）。
    pub fn invalidate_on_fingerprint_change(&mut self) {
        match self {
            ValidationState::Passed | ValidationState::Problems(_) | ValidationState::Running => {
                *self = ValidationState::Expired;
            }
            _ => {}
        }
    }
}

/// 预检状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrecheckState {
    Unchecked,
    Checking,
    Passed,
    /// 阻断，携带问题列表。
    Blocked(Vec<String>),
}

impl PrecheckState {
    pub fn can_start(&self) -> bool {
        matches!(self, PrecheckState::Passed)
    }
}

/// 冲突决策（导出/解包）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictDecision {
    Pending,
    Replace,
    Rename,
    Skip,
    ReplaceAll,
    SkipAll,
}

/// 通知状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationState {
    Unread,
    Read,
    Invalid,
}

/// 通知种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    TaskDone,
    TaskFailed,
    TaskCancelled,
    ValidationProblems,
    Environment,
}

/// 通知记录（§2.5 / G5-08）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: u64,
    pub kind: NotificationKind,
    pub title: String,
    pub target: Option<String>,
    pub state: NotificationState,
}

impl Notification {
    pub fn new(id: u64, kind: NotificationKind, title: &str, target: Option<&str>) -> Self {
        Self {
            id,
            kind,
            title: title.to_string(),
            target: target.map(|s| s.to_string()),
            state: NotificationState::Unread,
        }
    }

    pub fn mark_read(&mut self) {
        if self.state == NotificationState::Unread {
            self.state = NotificationState::Read;
        }
    }

    pub fn mark_invalid(&mut self) {
        self.state = NotificationState::Invalid;
    }
}

/// 通知中心：去重（同一任务只产生一条）、合并同类高频。
#[derive(Debug, Default, Clone)]
pub struct NotificationCenter {
    items: Vec<Notification>,
    next_id: u64,
}

impl NotificationCenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加通知；若已存在相同 target 的未读同类通知则合并（更新标题），返回通知 id。
    pub fn push(&mut self, kind: NotificationKind, title: &str, target: Option<&str>) -> u64 {
        // 合并：相同 kind + target 的未读通知
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|n| n.kind == kind && n.target == target.map(|s| s.to_string()) && n.state == NotificationState::Unread)
        {
            existing.title = title.to_string();
            return existing.id;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(Notification::new(id, kind, title, target));
        id
    }

    pub fn items(&self) -> &[Notification] {
        &self.items
    }

    pub fn unread_count(&self) -> usize {
        self.items
            .iter()
            .filter(|n| n.state == NotificationState::Unread)
            .count()
    }

    pub fn mark_all_read(&mut self) {
        for item in &mut self.items {
            item.mark_read();
        }
    }

    pub fn mark_read(&mut self, id: u64) {
        if let Some(item) = self.items.iter_mut().find(|n| n.id == id) {
            item.mark_read();
        }
    }

    /// 清除已读通知；运行中/未读保留。
    pub fn clear_read(&mut self) {
        self.items.retain(|n| n.state != NotificationState::Read);
    }

    /// 目标失效时标记为 Invalid（§2.5 通知状态）。
    pub fn invalidate_target(&mut self, target: &str) {
        for item in &mut self.items {
            if item.target.as_deref() == Some(target) {
                item.mark_invalid();
            }
        }
    }
}

/// 写任务串行判定：同一 PAK / 同一输出包 / 同一解包目录视为冲突。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteTarget {
    pub pak: String,
    pub output: Option<String>,
}

pub fn write_conflicts(a: &WriteTarget, b: &WriteTarget) -> bool {
    if crate::platform::path_key(std::path::Path::new(&a.pak))
        == crate::platform::path_key(std::path::Path::new(&b.pak))
    {
        return true;
    }
    match (&a.output, &b.output) {
        (Some(x), Some(y))
            if crate::platform::path_key(std::path::Path::new(x))
                == crate::platform::path_key(std::path::Path::new(y)) =>
        {
            true
        }
        _ => false,
    }
}

/// 队列中某任务是否需要等待前序写任务。
pub fn needs_wait(queue: &[WriteTarget], candidate: &WriteTarget) -> bool {
    queue.iter().any(|existing| write_conflicts(existing, candidate))
}
