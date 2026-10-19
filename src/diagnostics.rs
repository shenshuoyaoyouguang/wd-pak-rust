//! 环境自检与诊断信息（G0-07）。
//!
//! 只使用标准库：自检结果不阻断浏览，长路径状态由注入的探针决定，
//! 便于在无 Windows 机器上测试。

use std::fs;
use std::path::Path;

/// 界面显示的版本：唯一来源是 `Cargo.toml`，不得复制字面量。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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

/// 最近错误日志；最新在前，超出上限淘汰最旧的。
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

    pub fn record(&mut self, at: u64, title: &str, detail: &str) {
        self.items.insert(
            0,
            ErrorRecord {
                at,
                title: title.to_string(),
                detail: detail.to_string(),
            },
        );
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
pub fn dir_writable(dir: &Path) -> (bool, String) {
    if let Err(error) = fs::create_dir_all(dir) {
        return (
            false,
            format!("无法创建目录 {}: {}", dir.display(), error),
        );
    }
    let probe = dir.join(format!(".wd_pak_write_probe_{}", std::process::id()));
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            (true, String::new())
        }
        Err(error) => (false, format!("{} 不可写: {}", dir.display(), error)),
    }
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
