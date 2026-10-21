//! 任务面板骨架（G0-09）。
//!
//! 只定义任务面板的状态模型，不涉及 Win32 控件。

use crate::gui::states::{TaskKind, TaskState};

/// 任务面板中的单条记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: u64,
    pub kind: TaskKind,
    pub target: String,
    pub state: TaskState,
    pub progress: (usize, usize), // (done, total)
}

impl TaskRecord {
    pub fn new(id: u64, kind: TaskKind, target: &str) -> Self {
        Self {
            id,
            kind,
            target: target.to_string(),
            state: TaskState::Queued,
            progress: (0, 0),
        }
    }

    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}

/// 任务面板状态。
#[derive(Debug, Default, Clone)]
pub struct TaskPanel {
    pub collapsed: bool,
    pub height: i32, // DIP
    pub records: Vec<TaskRecord>,
    next_id: u64,
}

impl TaskPanel {
    pub fn new(height: i32) -> Self {
        Self {
            collapsed: false,
            height,
            records: Vec::new(),
            next_id: 1,
        }
    }

    /// 提交一个新任务，返回任务 ID。
    pub fn submit(&mut self, kind: TaskKind, target: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.records.push(TaskRecord::new(id, kind, target));
        id
    }

    /// 按 ID 更新任务状态；已结束的任务不再接受状态变更。
    pub fn update_state(&mut self, id: u64, state: TaskState) {
        if let Some(rec) = self.records.iter_mut().find(|r| r.id == id) {
            if rec.is_terminal() {
                return;
            }
            rec.state = state;
        }
    }

    /// 按 ID 更新进度。
    pub fn update_progress(&mut self, id: u64, done: usize, total: usize) {
        if let Some(rec) = self.records.iter_mut().find(|r| r.id == id) {
            rec.progress = (done, total);
        }
    }

    /// 活跃任务数量。
    pub fn active_count(&self) -> usize {
        self.records.iter().filter(|r| r.is_active()).count()
    }

    /// 是否有写任务进行中。
    pub fn has_write_task(&self) -> bool {
        self.records
            .iter()
            .any(|r| r.kind.is_write() && r.is_active())
    }

    /// 折叠/展开切换。
    pub fn toggle_collapsed(&mut self) {
        self.collapsed = !self.collapsed;
    }

    /// 最近一条任务记录（用于折叠态摘要）。
    pub fn latest(&self) -> Option<&TaskRecord> {
        self.records.last()
    }

    /// 清除已结束的任务；运行中/等待中的记录保留（G5-06）。
    pub fn clear_completed(&mut self) -> usize {
        let before = self.records.len();
        self.records.retain(|record| record.is_active());
        before - self.records.len()
    }

    /// 折叠态单行文本：无任务 / 最近一条任务及进度。
    pub fn summary_text(&self) -> String {
        match self.latest() {
            None => "任务：无".into(),
            Some(record) => {
                let progress = if record.progress.1 > 0 {
                    format!("（{} / {}）", record.progress.0, record.progress.1)
                } else {
                    String::new()
                };
                format!(
                    "任务：{} {} — {}{}",
                    record.kind.label(),
                    record.target,
                    record.state.label(),
                    progress
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_returns_incrementing_ids() {
        let mut panel = TaskPanel::new(160);
        let id1 = panel.submit(TaskKind::Export, "a.pak");
        let id2 = panel.submit(TaskKind::Pack, "out.pak");
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(panel.records.len(), 2);
    }

    #[test]
    fn update_state_and_progress() {
        let mut panel = TaskPanel::new(160);
        let id = panel.submit(TaskKind::Export, "a.pak");
        panel.update_state(id, TaskState::Running);
        panel.update_progress(id, 3, 10);
        let rec = &panel.records[0];
        assert_eq!(rec.state, TaskState::Running);
        assert_eq!(rec.progress, (3, 10));
    }

    #[test]
    fn active_count_and_write_task() {
        let mut panel = TaskPanel::new(160);
        let id = panel.submit(TaskKind::Replace, "x.pak");
        panel.update_state(id, TaskState::Running);
        assert_eq!(panel.active_count(), 1);
        assert!(panel.has_write_task());

        panel.update_state(id, TaskState::Done);
        assert_eq!(panel.active_count(), 0);
        assert!(!panel.has_write_task());
    }

    #[test]
    fn terminal_task_ignores_state_updates() {
        let mut panel = TaskPanel::new(160);
        let id = panel.submit(TaskKind::Export, "a.pak");
        panel.update_state(id, TaskState::Done);
        panel.update_state(id, TaskState::Running);
        assert_eq!(panel.records[0].state, TaskState::Done);

        panel.update_state(id, TaskState::Cancelled);
        assert_eq!(panel.records[0].state, TaskState::Done);
    }

    #[test]
    fn toggle_collapsed() {
        let mut panel = TaskPanel::new(160);
        assert!(!panel.collapsed);
        panel.toggle_collapsed();
        assert!(panel.collapsed);
    }

    #[test]
    fn summary_and_clear_completed() {
        let mut panel = TaskPanel::new(160);
        assert_eq!(panel.summary_text(), "任务：无");
        let done = panel.submit(TaskKind::Replace, "a.pak");
        panel.update_state(done, TaskState::Done);
        let running = panel.submit(TaskKind::Pack, "b.pak");
        panel.update_state(running, TaskState::Running);
        panel.update_progress(running, 2, 5);
        assert_eq!(panel.summary_text(), "任务：打包 b.pak — 进行中（2 / 5）");
        assert_eq!(panel.clear_completed(), 1);
        assert_eq!(panel.records.len(), 1);
        assert_eq!(panel.records[0].id, running);
    }
}
