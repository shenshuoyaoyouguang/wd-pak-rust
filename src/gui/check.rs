//! 校验纯逻辑（G5）。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::PakError;
use crate::format::{DIR_BIT, RAW_BIT};
use crate::session::SessionEntry;

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

impl CheckProblemKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OffsetOutOfBounds | Self::SizeOutOfBounds => "越界",
            Self::DecompressionFailed => "解压失败",
            Self::SizeMismatch => "解压大小不符",
            Self::DuplicateName => "重名",
            Self::Cycle => "循环引用",
            Self::BadName => "非法名",
        }
    }
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

    /// 问题表的“类型”列；与 CLI 报告使用同一套标签。
    pub fn kind_label(&self) -> &'static str {
        self.kind.label()
    }

    /// 与 CLI `check` 的问题行逐字对齐（`  {line}`）。
    pub fn cli_line(&self) -> String {
        match self.kind {
            CheckProblemKind::OffsetOutOfBounds | CheckProblemKind::SizeOutOfBounds => {
                format!("越界: {} {}", self.path, self.detail)
            }
            CheckProblemKind::SizeMismatch => {
                format!("解压大小不符: {} {}", self.path, self.detail)
            }
            CheckProblemKind::DecompressionFailed => {
                format!("解压失败: {}: {}", self.path, self.detail)
            }
            _ => format!("{}: {} {}", self.kind.label(), self.path, self.detail),
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

impl CheckConclusionState {
    /// 四态结论文案（未校验/运行中不属于结论）。
    pub fn label(self) -> String {
        match self {
            Self::Unchecked => "未校验".into(),
            Self::Running => "校验中".into(),
            Self::Passed => "校验通过".into(),
            Self::Problems(n) => format!("发现问题 {n} 个"),
            Self::Cancelled => "校验已取消".into(),
            Self::Expired => "结果已过期".into(),
        }
    }
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
    pub attribs: BTreeMap<u32, usize>,
    pub problems: Vec<CheckProblem>,
}

impl CheckResult {
    pub fn passed(&self) -> bool {
        self.problems.is_empty()
    }

    /// attrib 分布文本；报告与界面统计共用，保证与 CLI `check` 头部一致。
    pub fn attrib_distribution(&self) -> String {
        self.attribs
            .iter()
            .map(|(attrib, count)| format!("0x{attrib:08x}x{count}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// 包指纹：大小 + 修改时间（纳秒）。读取失败返回 None，调用方按“无法判定”处理。
pub fn pak_fingerprint(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(format!(
        "{}:{}:{}",
        meta.len(),
        since.as_secs(),
        since.subsec_nanos()
    ))
}

/// 按问题类型筛选；`None` 表示全部。顺序保持原样。
pub fn filter_problems(
    problems: &[CheckProblem],
    kind: Option<CheckProblemKind>,
) -> Vec<CheckProblem> {
    problems
        .iter()
        .filter(|problem| kind.map(|kind| problem.kind == kind).unwrap_or(true))
        .cloned()
        .collect()
}

/// 按包内路径排序（稳定排序，同路径保持原顺序）。
pub fn sort_problems_by_path(problems: &mut [CheckProblem], ascending: bool) {
    problems.sort_by(|a, b| {
        let ordering = a.path.cmp(&b.path);
        if ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

/// 一次校验执行的结果；取消时保留已处理的部分结果。
#[derive(Debug, Clone, PartialEq)]
pub struct CheckOutcome {
    pub result: CheckResult,
    pub cancelled: bool,
    pub processed: usize,
    pub total: usize,
}

impl CheckOutcome {
    /// 生成与 CLI `check` 结构一致的报告文本。
    ///
    /// CLI 只列出前 50 条问题，界面展示全部；超过 50 条时报告注明差异。
    pub fn report(&self, pak_path: &Path) -> String {
        let result = &self.result;
        let mut out = String::new();
        out.push_str(&format!("# {}\n", pak_path.display()));
        out.push_str(&format!(
            "# files={} dirs={} rootDirSize={} fileSize={}\n",
            result.files, result.dirs, result.root_dir_size, result.file_size
        ));
        out.push_str(&format!(
            "# stored={} org={} ratio={:.3}\n",
            result.total_stored, result.total_org, result.ratio
        ));
        out.push_str(&format!(
            "# attrib 分布: {}\n",
            result.attrib_distribution()
        ));
        if self.cancelled {
            out.push_str(&format!(
                "# 校验已取消（已处理 {}/{} 项）\n",
                self.processed, self.total
            ));
            return out;
        }
        if result.problems.is_empty() {
            out.push_str(&format!(
                "# 校验通过{}\n",
                if result.mode == CheckMode::Deep {
                    " (含全量解压)"
                } else {
                    ""
                }
            ));
            return out;
        }
        out.push_str(&format!("# 问题 {} 个:\n", result.problems.len()));
        if result.problems.len() > 50 {
            out.push_str("# （CLI 仅列前 50 条；界面与复制内容为全部）\n");
        }
        for problem in &result.problems {
            out.push_str(&format!("  {}\n", problem.cli_line()));
        }
        out
    }
}

/// 遍历会话条目执行校验。`read` 只在深度校验时调用；
/// `cancel` 置位后在条目之间停止，已处理的部分结果仍会返回。
pub fn run_check(
    entries: &[SessionEntry],
    root_dir_size: u32,
    file_size: u64,
    mode: CheckMode,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
    mut read: impl FnMut(&SessionEntry) -> Result<Vec<u8>, PakError>,
) -> CheckOutcome {
    let total = entries.len();
    let mut files = 0usize;
    let mut dirs = 0usize;
    let mut total_stored = 0u64;
    let mut total_org = 0u64;
    let mut attribs: BTreeMap<u32, usize> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut processed = 0usize;
    let mut cancelled = false;

    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        if entry.is_dir() {
            dirs += 1;
        } else {
            files += 1;
            total_stored += entry.size as u64;
            total_org += entry.org as u64;
            *attribs.entry(entry.attrib).or_insert(0) += 1;
            if entry.offset as u64 + entry.size as u64 > file_size {
                problems.push(CheckProblem::new(
                    CheckProblemKind::OffsetOutOfBounds,
                    &entry.path,
                    &format!("off=0x{:x} size={}", entry.offset, entry.size),
                ));
            }
            if mode == CheckMode::Deep {
                match read(entry) {
                    Ok(data) => {
                        if data.len() as u64 != entry.org as u64 {
                            problems.push(CheckProblem::new(
                                CheckProblemKind::SizeMismatch,
                                &entry.path,
                                &format!("{} != {}", data.len(), entry.org),
                            ));
                        }
                    }
                    Err(error) => problems.push(CheckProblem::new(
                        CheckProblemKind::DecompressionFailed,
                        &entry.path,
                        &error.to_string(),
                    )),
                }
            }
        }
        processed += 1;
        progress(processed, total);
    }

    CheckOutcome {
        result: CheckResult {
            mode,
            files,
            dirs,
            root_dir_size,
            file_size,
            total_stored,
            total_org,
            ratio: total_stored as f64 / total_org.max(1) as f64,
            attribs,
            problems,
        },
        cancelled,
        processed,
        total,
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

    fn entry(path: &str, offset: u32, size: u32, org: u32, attrib: u32) -> SessionEntry {
        SessionEntry {
            path: path.to_string(),
            attrib,
            offset,
            size,
            org,
            mtime: 0,
        }
    }

    #[test]
    fn run_shallow_reports_out_of_bounds() {
        let entries = vec![
            entry("d", 0, 0, 0, DIR_BIT),
            entry("a", 0, 5, 5, 0),
            entry("b", 10, 5, 5, 0),
        ];
        let cancel = AtomicBool::new(false);
        let mut last_progress = (0, 0);
        let outcome = run_check(
            &entries,
            1024,
            12,
            CheckMode::Shallow,
            &cancel,
            |done, total| last_progress = (done, total),
            |_| panic!("浅校验不应读取条目内容"),
        );
        assert_eq!(last_progress, (3, 3));
        assert_eq!(outcome.processed, 3);
        assert_eq!(outcome.total, 3);
        assert!(!outcome.cancelled);
        assert_eq!(outcome.result.files, 2);
        assert_eq!(outcome.result.dirs, 1);
        assert_eq!(outcome.result.total_stored, 10);
        assert_eq!(outcome.result.total_org, 10);
        assert_eq!(outcome.result.ratio, 1.0);
        assert_eq!(outcome.result.problems.len(), 1);
        assert_eq!(outcome.result.problems[0].kind, CheckProblemKind::OffsetOutOfBounds);
        assert_eq!(outcome.result.problems[0].cli_line(), "越界: b off=0xa size=5");
    }

    #[test]
    fn run_deep_reports_read_failure_and_size_mismatch() {
        let entries = vec![entry("bad", 0, 2, 10, 0), entry("short", 2, 2, 5, 0)];
        let cancel = AtomicBool::new(false);
        let outcome = run_check(
            &entries,
            0,
            100,
            CheckMode::Deep,
            &cancel,
            |_, _| {},
            |entry| {
                if entry.path == "bad" {
                    Err(PakError::DataOob("bad data".into()))
                } else {
                    Ok(vec![1, 2, 3])
                }
            },
        );
        assert_eq!(outcome.result.problems.len(), 2);
        assert_eq!(
            outcome.result.problems[0].cli_line(),
            "解压失败: bad: 读取越界: bad data"
        );
        assert_eq!(
            outcome.result.problems[1].cli_line(),
            "解压大小不符: short 3 != 5"
        );
    }

    #[test]
    fn run_check_cancel_stops_between_entries() {
        let entries = vec![
            entry("a", 0, 1, 1, 0),
            entry("b", 1, 1, 1, 0),
            entry("c", 2, 1, 1, 0),
        ];
        let cancel = AtomicBool::new(false);
        let outcome = run_check(
            &entries,
            0,
            100,
            CheckMode::Shallow,
            &cancel,
            |done, _| {
                if done == 1 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            |_| unreachable!(),
        );
        assert!(outcome.cancelled);
        assert_eq!(outcome.processed, 1);
        assert_eq!(outcome.total, 3);
        assert_eq!(outcome.result.files, 1);
    }

    #[test]
    fn conclusion_state_labels() {
        assert_eq!(CheckConclusionState::Unchecked.label(), "未校验");
        assert_eq!(CheckConclusionState::Passed.label(), "校验通过");
        assert_eq!(
            CheckConclusionState::Problems(4).label(),
            "发现问题 4 个"
        );
        assert_eq!(CheckConclusionState::Expired.label(), "结果已过期");
    }

    #[test]
    fn fingerprint_changes_with_content() {
        let dir = std::env::temp_dir().join(format!(
            "wd-pak-fp-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.pak");
        std::fs::write(&path, b"one").unwrap();
        let first = pak_fingerprint(&path).unwrap();
        std::fs::write(&path, b"two-two").unwrap();
        let second = pak_fingerprint(&path).unwrap();
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_and_sort_problems() {
        let problems = vec![
            CheckProblem::new(CheckProblemKind::DecompressionFailed, "b", "x"),
            CheckProblem::new(CheckProblemKind::OffsetOutOfBounds, "c", "y"),
            CheckProblem::new(CheckProblemKind::DecompressionFailed, "a", "z"),
        ];
        let only_decompress =
            filter_problems(&problems, Some(CheckProblemKind::DecompressionFailed));
        assert_eq!(only_decompress.len(), 2);
        assert_eq!(filter_problems(&problems, None).len(), 3);

        let mut sorted = only_decompress;
        sort_problems_by_path(&mut sorted, true);
        assert_eq!(sorted[0].path, "a");
        sort_problems_by_path(&mut sorted, false);
        assert_eq!(sorted[0].path, "b");
    }

    #[test]
    fn report_matches_cli_header() {
        let entries = vec![entry("a", 0, 5, 10, 0)];
        let cancel = AtomicBool::new(false);
        let outcome = run_check(
            &entries,
            1024,
            100,
            CheckMode::Shallow,
            &cancel,
            |_, _| {},
            |_| unreachable!(),
        );
        let report = outcome.report(Path::new("x.pak"));
        assert!(report.starts_with("# x.pak\n"), "{report}");
        assert!(report.contains("# files=1 dirs=0 rootDirSize=1024 fileSize=100\n"));
        assert!(report.contains("# stored=5 org=10 ratio=0.500\n"));
        assert!(report.contains("# attrib 分布: 0x00000000x1\n"));
        assert!(report.ends_with("# 校验通过\n"), "{report}");
    }
}
