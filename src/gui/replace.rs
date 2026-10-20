//! 替换纯逻辑（G4-01/02）。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionChoice {
    #[default]
    Auto,
    Lzss,
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceOptions {
    pub compress: bool,
    pub backup: bool,
    pub compression: CompressionChoice,
}

impl Default for ReplaceOptions {
    fn default() -> Self {
        Self {
            compress: true,
            backup: true,
            compression: CompressionChoice::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceIssueKind {
    TargetMissing,
    NewFileTooLarge,
    TargetNotWritable,
    SizeLimit,
    NameInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceIssue {
    pub kind: ReplaceIssueKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplacePrecheck {
    pub target_exists: bool,
    pub old_size: u64,
    pub new_size: u64,
    pub size_delta: i64,
    pub target_writable: bool,
    pub issues: Vec<ReplaceIssue>,
}

impl ReplacePrecheck {
    pub fn can_start(&self) -> bool {
        self.issues.is_empty() && self.target_exists
    }
}

pub fn replace_precheck(
    target_exists: bool,
    old_size: u64,
    new_size: u64,
    target_writable: bool,
    max_file_size: u64,
) -> ReplacePrecheck {
    let mut issues = Vec::new();
    if !target_exists {
        issues.push(ReplaceIssue {
            kind: ReplaceIssueKind::TargetMissing,
            detail: "目标条目不存在".into(),
        });
    }
    if !target_writable {
        issues.push(ReplaceIssue {
            kind: ReplaceIssueKind::TargetNotWritable,
            detail: "目标不可写".into(),
        });
    }
    if new_size > max_file_size {
        issues.push(ReplaceIssue {
            kind: ReplaceIssueKind::NewFileTooLarge,
            detail: format!("新文件 {} 超过 {}", new_size, max_file_size),
        });
    }
    if new_size > u64::from(u32::MAX) || old_size > u64::from(u32::MAX) {
        issues.push(ReplaceIssue {
            kind: ReplaceIssueKind::SizeLimit,
            detail: "超过 4GB 限制".into(),
        });
    }
    ReplacePrecheck {
        target_exists,
        old_size,
        new_size,
        size_delta: new_size as i64 - old_size as i64,
        target_writable,
        issues,
    }
}

pub fn backup_path(pak: &Path) -> PathBuf {
    let mut p = pak.to_path_buf();
    let mut name = pak
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".bak");
    p.set_file_name(name);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precheck_can_start() {
        let p = replace_precheck(true, 10, 12, true, 1000);
        assert!(p.can_start());
        assert_eq!(p.size_delta, 2);
    }

    #[test]
    fn precheck_missing_target() {
        let p = replace_precheck(false, 0, 12, true, 1000);
        assert!(!p.can_start());
    }

    #[test]
    fn default_backup_enabled() {
        // spec: 替换备份默认勾选
        assert!(ReplaceOptions::default().backup);
        assert!(ReplaceOptions::default().compress);
        assert_eq!(
            ReplaceOptions::default().compression,
            CompressionChoice::Auto
        );
    }

    #[test]
    fn backup_path_appends_bak() {
        assert_eq!(
            backup_path(Path::new("a.pak")),
            PathBuf::from("a.pak.bak")
        );
    }
}
