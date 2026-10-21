//! 校验纯逻辑（G5）。

use crate::format::{DIR_BIT, RAW_BIT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckMode {
    #[default]
    Shallow,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckProblemKind {
    OffsetOutOfBounds,
    SizeOutOfBounds,
    DecompressionFailed,
    SizeMismatch,
    DuplicateName,
    Cycle,
    BadName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckProblem {
    pub kind: CheckProblemKind,
    pub path: String,
    pub detail: String,
}

impl CheckProblem {
    pub fn new(kind: CheckProblemKind, path: &str, detail: &str) -> Self {
        Self {
            kind,
            path: path.to_string(),
            detail: detail.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckConclusionState {
    #[default]
    Unchecked,
    Running,
    Passed,
    Problems(usize),
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckConclusion {
    pub state: CheckConclusionState,
    pub fingerprint: Option<String>,
    pub checked_at: Option<u64>,
}

impl CheckConclusion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self) {
        self.state = CheckConclusionState::Running;
    }

    pub fn pass(&mut self, fingerprint: Option<String>, now: u64) {
        if !matches!(self.state, CheckConclusionState::Running) {
            return;
        }
        self.state = CheckConclusionState::Passed;
        self.fingerprint = fingerprint;
        self.checked_at = Some(now);
    }

    pub fn problems(&mut self, count: usize, fingerprint: Option<String>, now: u64) {
        if !matches!(self.state, CheckConclusionState::Running) {
            return;
        }
        // 零问题等价于通过，与 `ValidationState::complete` 保持一致。
        self.state = if count == 0 {
            CheckConclusionState::Passed
        } else {
            CheckConclusionState::Problems(count)
        };
        self.fingerprint = fingerprint;
        self.checked_at = Some(now);
    }

    pub fn cancel(&mut self) {
        if !matches!(self.state, CheckConclusionState::Running) {
            return;
        }
        self.state = CheckConclusionState::Cancelled;
    }

    pub fn expire(&mut self) {
        if matches!(
            self.state,
            CheckConclusionState::Running
                | CheckConclusionState::Passed
                | CheckConclusionState::Problems(_)
        ) {
            self.state = CheckConclusionState::Expired;
        }
    }

    /// 写入会改变包内容：已有结论一律作废（未校验/已作废保持不变）。
    pub fn invalidate_on_write(&mut self) {
        self.expire();
    }

    pub fn invalidate_on_fingerprint_change(&mut self) {
        self.invalidate_on_write();
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            CheckConclusionState::Passed | CheckConclusionState::Problems(_) | CheckConclusionState::Expired | CheckConclusionState::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckResult {
    pub mode: CheckMode,
    pub files: usize,
    pub dirs: usize,
    pub root_dir_size: u32,
    pub file_size: u64,
    pub total_stored: u64,
    pub total_org: u64,
    pub ratio: f64,
    pub problems: Vec<CheckProblem>,
}

impl CheckResult {
    pub fn passed(&self) -> bool {
        self.problems.is_empty()
    }
}

/// 浅校验：offset+size 越界。
pub fn shallow_check_entry(path: &str, offset: u64, size: u64, container_size: u64) -> Vec<CheckProblem> {
    let mut out = Vec::new();
    if offset.saturating_add(size) > container_size {
        out.push(CheckProblem::new(
            CheckProblemKind::OffsetOutOfBounds,
            path,
            "offset+size 超出容器",
        ));
    }
    out
}

/// 深校验：LZSS 解压大小匹配。
pub fn deep_check_entry(
    path: &str,
    is_raw: bool,
    stored: &[u8],
    org_size: usize,
) -> Vec<CheckProblem> {
    let mut out = Vec::new();
    if is_raw {
        if stored.len() < org_size {
            out.push(CheckProblem::new(
                CheckProblemKind::SizeMismatch,
                path,
                "RAW 大小不匹配",
            ));
        }
        return out;
    }
    match crate::lzss::decompress_to_vec(stored, org_size as u64) {
        Ok(buf) => {
            if buf.len() != org_size {
                out.push(CheckProblem::new(
                    CheckProblemKind::SizeMismatch,
                    path,
                    "解压大小不匹配",
                ));
            }
        }
        Err(_) => {
            out.push(CheckProblem::new(
                CheckProblemKind::DecompressionFailed,
                path,
                "解压失败",
            ));
        }
    }
    out
}

/// attrib 位解释辅助。
pub fn attrib_is_dir(attrib: u32) -> bool {
    attrib & DIR_BIT != 0
}

pub fn attrib_is_raw(attrib: u32) -> bool {
    attrib & RAW_BIT != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conclusion_lifecycle() {
        let mut c = CheckConclusion::new();
        c.start();
        assert_eq!(c.state, CheckConclusionState::Running);
        c.pass(Some("fp".into()), 100);
        assert_eq!(c.state, CheckConclusionState::Passed);
        assert_eq!(c.checked_at, Some(100));
        c.invalidate_on_write();
        assert_eq!(c.state, CheckConclusionState::Expired);
    }

    #[test]
    fn conclusion_zero_problems_is_passed() {
        let mut c = CheckConclusion::new();
        c.start();
        c.problems(0, Some("fp".into()), 7);
        assert_eq!(c.state, CheckConclusionState::Passed);
        assert_eq!(c.checked_at, Some(7));
    }

    #[test]
    fn conclusion_ignores_completion_after_cancel() {
        let mut c = CheckConclusion::new();
        c.start();
        c.cancel();
        c.pass(Some("fp".into()), 1);
        assert_eq!(c.state, CheckConclusionState::Cancelled);
        assert_eq!(c.checked_at, None);
    }

    #[test]
    fn conclusion_write_during_running_expires_and_blocks_completion() {
        let mut c = CheckConclusion::new();
        c.start();
        c.invalidate_on_write();
        assert_eq!(c.state, CheckConclusionState::Expired);
        c.problems(2, Some("fp".into()), 3);
        assert_eq!(c.state, CheckConclusionState::Expired);
        assert_eq!(c.checked_at, None);
    }

    #[test]
    fn shallow_check() {
        let probs = shallow_check_entry("a", 10, 5, 12);
        assert!(!probs.is_empty());
        let ok = shallow_check_entry("a", 0, 5, 12);
        assert!(ok.is_empty());
    }

    #[test]
    fn deep_check_raw_mismatch() {
        let probs = deep_check_entry("a", true, &[1, 2], 3);
        assert!(!probs.is_empty());
    }

    #[test]
    fn deep_check_raw_trailing_bytes_accepted() {
        let probs = deep_check_entry("a", true, &[1, 2, 3, 4], 3);
        assert!(probs.is_empty());
    }
}
