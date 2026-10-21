//! 环境自检与诊断信息（G0-07）。
//!
//! 只使用标准库：自检结果不阻断浏览，长路径状态由注入的探针决定，
//! 便于在无 Windows 机器上测试。

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// 界面显示的版本：规范要求原样显示 `1.0`（`docs/windows-ui-plan.md` §805），
/// 与 `Cargo.toml` 的包版本（1.0.0）解耦。
///
/// CLI（`main.rs` 的 `--version`）与 GUI 关于对话框共用的唯一显示版本来源；
/// 修改显示版本只需改本常量。
pub const APP_VERSION: &str = "1.0";

/// 诊断里保留的最近错误条数。
pub const RECENT_ERROR_LIMIT: usize = 5;

/// 自检项状态；`Failed` 只降级展示，不返回错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Ok,
    Warn,
    Failed,
}

/// 一条最近错误；`at` 为 Unix 秒，用于排序与展示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorRecord {
    pub at: u64,
    pub title: String,
    pub detail: String,
}

/// 最近错误日志；按 `at` 降序（最新在前），超出上限淘汰 `at` 最旧的。
#[derive(Debug)]
pub struct ErrorLog {
    limit: usize,
    items: Vec<ErrorRecord>,
}

impl ErrorLog {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            items: Vec::new(),
        }
    }

    /// 记录一条错误；按时间戳排序而非到达顺序，乱序时间戳不会破坏
    /// [`ErrorLog::items`] / [`ErrorLog::last`] 的「最新在前」约定。
    /// 同一时间戳保持稳定顺序，后到者排在前。
    pub fn record(&mut self, at: u64, title: &str, detail: &str) {
        self.items.insert(
            0,
            ErrorRecord {
                at,
                title: title.to_string(),
                detail: detail.to_string(),
            },
        );
        // 稳定排序：插入在前的新记录在同时间戳中保持最新。
        self.items.sort_by(|a, b| b.at.cmp(&a.at));
        self.items.truncate(self.limit);
    }

    pub fn items(&self) -> &[ErrorRecord] {
        &self.items
    }

    pub fn last(&self) -> Option<&ErrorRecord> {
        self.items.first()
    }
}

impl Default for ErrorLog {
    fn default() -> Self {
        Self::new(RECENT_ERROR_LIMIT)
    }
}

/// DPI → 缩放百分比；取不到 DPI（0）时按 100% 处理。
pub fn scale_percent(dpi: u32) -> u32 {
    if dpi == 0 {
        100
    } else {
        dpi * 100 / 96
    }
}

/// 默认目录是否可写；探针文件用完即删，失败时给出可读原因。
///
/// 探针使用「进程 id + 进程内计数」的唯一名字并以 `create_new` 独占创建：
/// 既不会截断或删除已存在的同名文件，也只删除本次调用实际创建的文件。
pub fn dir_writable(dir: &Path) -> (bool, String) {
    if let Err(error) = fs::create_dir_all(dir) {
        return (
            false,
            format!("无法创建目录 {}: {}", dir.display(), error),
        );
    }
    static NEXT_PROBE: AtomicU64 = AtomicU64::new(0);
    for _ in 0..16 {
        let serial = NEXT_PROBE.fetch_add(1, Ordering::Relaxed);
        let probe = dir.join(format!(
            ".wd_pak_write_probe_{}_{}",
            std::process::id(),
            serial
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(file) => {
                drop(file);
                let _ = fs::remove_file(&probe);
                return (true, String::new());
            }
            // 名字被历史残留占用：换下一个序号重试，绝不截断已有文件。
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return (false, format!("{} 不可写: {}", dir.display(), error)),
        }
    }
    (
        false,
        format!("{} 不可写: 无法创建唯一的探针文件", dir.display()),
    )
}

/// 系统长路径支持；`probe` 收到一条超过 260 字符的临时路径，由调用方实际创建。
pub fn long_paths_enabled(probe: &mut dyn FnMut(&Path) -> std::io::Result<()>) -> (bool, String) {
    let mut path = std::env::temp_dir();
    path.push("wd_pak_long_path_probe");
    for index in 0..4 {
        path.push(format!("d{index}").repeat(30));
    }
    match probe(&path) {
        Ok(()) => (true, "系统长路径支持：已启用".into()),
        Err(error) => (
            false,
            format!("系统长路径支持：未启用（{}）", error),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // D6：界面版本原样显示 `1.0`；回退为 env!("CARGO_PKG_VERSION") 会得到 1.0.0 而失败。
    #[test]
    fn app_version_is_display_spec_value() {
        assert_eq!(APP_VERSION, "1.0");
    }

    #[test]
    fn error_log_orders_by_timestamp_not_arrival() {
        let mut log = ErrorLog::new(3);
        log.record(100, "旧", "");
        log.record(300, "新", "");
        log.record(200, "中", "");

        let ats: Vec<u64> = log.items().iter().map(|item| item.at).collect();
        assert_eq!(ats, vec![300, 200, 100]);
        assert_eq!(log.last().unwrap().title, "新");

        // 迟到的更旧记录被淘汰，而不是挤掉最新记录。
        log.record(50, "迟到", "");
        assert_eq!(log.items().len(), 3);
        assert!(log.items().iter().all(|item| item.at >= 100));
        assert!(!log.items().iter().any(|item| item.title == "迟到"));
    }

    #[test]
    fn dir_writable_preserves_existing_files_and_cleans_probe() {
        let dir = std::env::temp_dir().join(format!(
            "wd_pak_diag_probe_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join(format!(".wd_pak_write_probe_{}", std::process::id()));
        std::fs::write(&existing, b"keep me").unwrap();

        let (ok, detail) = dir_writable(&dir);
        assert!(ok, "{detail}");
        assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");
        let existing_name = existing.file_name().unwrap().to_string_lossy().into_owned();
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != &existing_name)
            .collect();
        assert!(leftovers.is_empty(), "探针未清理: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
