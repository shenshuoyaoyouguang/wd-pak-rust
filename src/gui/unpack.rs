//! 解包纯逻辑（G4-06/07）。

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnpackOptions {
    pub out_dir: PathBuf,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnpackIssueKind {
    CaseConflict,
    IllegalName,
    PathTooLong,
    NotWritable,
    InsufficientSpace,
    TargetExists,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpackIssue {
    pub kind: UnpackIssueKind,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnpackPrecheckResult {
    pub issues: Vec<UnpackIssue>,
    pub file_count: usize,
    pub dir_count: usize,
    pub total_size: u64,
}

impl UnpackPrecheckResult {
    pub fn can_start(&self) -> bool {
        self.issues.is_empty()
    }

    /// 硬阻断：非法名、长路径、不可写、空间不足（大小写冲突与同名目标可显式确认）。
    pub fn blocking(&self) -> bool {
        self.issues.iter().any(|issue| {
            matches!(
                issue.kind,
                UnpackIssueKind::IllegalName
                    | UnpackIssueKind::PathTooLong
                    | UnpackIssueKind::NotWritable
                    | UnpackIssueKind::InsufficientSpace
            )
        })
    }

    pub fn has_kind(&self, kind: UnpackIssueKind) -> bool {
        self.issues.iter().any(|issue| issue.kind == kind)
    }
}

/// Windows 文件名非法字符（不含路径分隔符；分隔符在包内路径中本就不出现）。
pub fn has_invalid_windows_char(name: &str) -> bool {
    name.chars()
        .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
}

/// Windows 非法文件名检测（CON, PRN, AUX, NUL, COM1.., LPT1.. 等）。
pub fn is_windows_reserved(name: &str) -> bool {
    let upper = name.to_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    if matches!(stem, "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let is_device = |prefix: &str| {
        stem.len() == 4 && stem.starts_with(prefix) && stem.as_bytes()[3].is_ascii_digit()
    };
    is_device("COM") || is_device("LPT")
}

/// 检测尾部空格/点。
pub fn has_trailing_space_or_dot(name: &str) -> bool {
    name.ends_with(' ') || name.ends_with('.')
}

pub fn unpack_precheck(
    entries: &[(String, u64, bool)],
    out_dir_writable: bool,
    available_space: u64,
    path_limit: usize,
) -> UnpackPrecheckResult {
    unpack_precheck_full(entries, out_dir_writable, available_space, 0, path_limit)
}

/// 完整预检：`base_len` 为输出目录绝对路径长度（长路径按完整目标路径计算）。
pub fn unpack_precheck_full(
    entries: &[(String, u64, bool)],
    out_dir_writable: bool,
    available_space: u64,
    base_len: usize,
    path_limit: usize,
) -> UnpackPrecheckResult {
    let mut issues = Vec::new();
    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut total_size = 0u64;
    for (a, b) in detect_case_conflicts(entries) {
        issues.push(UnpackIssue {
            kind: UnpackIssueKind::CaseConflict,
            path: a.clone(),
            detail: format!("大小写冲突: {} / {}", a, b),
        });
    }
    for (path, size, is_dir) in entries {
        if *is_dir {
            dir_count += 1;
        } else {
            file_count += 1;
            total_size += size;
        }
        for part in path.split('/').filter(|part| !part.is_empty()) {
            if is_windows_reserved(part) {
                issues.push(UnpackIssue {
                    kind: UnpackIssueKind::IllegalName,
                    path: path.clone(),
                    detail: format!("Windows 保留名: {}", part),
                });
            }
            if has_trailing_space_or_dot(part) {
                issues.push(UnpackIssue {
                    kind: UnpackIssueKind::IllegalName,
                    path: path.clone(),
                    detail: format!("尾部空格或点: {}", part),
                });
            }
            if has_invalid_windows_char(part) {
                issues.push(UnpackIssue {
                    kind: UnpackIssueKind::IllegalName,
                    path: path.clone(),
                    detail: format!("非法字符: {}", part),
                });
            }
        }
        let total_len = if base_len == 0 {
            path.chars().count()
        } else {
            base_len + 1 + path.chars().count()
        };
        if total_len > path_limit {
            issues.push(UnpackIssue {
                kind: UnpackIssueKind::PathTooLong,
                path: path.clone(),
                detail: format!("目标路径超过 {} 字符", path_limit),
            });
        }
    }
    if !out_dir_writable {
        issues.push(UnpackIssue {
            kind: UnpackIssueKind::NotWritable,
            path: String::new(),
            detail: "目标目录不可写".into(),
        });
    }
    if total_size > available_space {
        issues.push(UnpackIssue {
            kind: UnpackIssueKind::InsufficientSpace,
            path: String::new(),
            detail: "空间不足".into(),
        });
    }
    UnpackPrecheckResult {
        issues,
        file_count,
        dir_count,
        total_size,
    }
}

/// 同层大小写冲突（`entries` 为包内路径；比较父目录与 basename）。
pub fn detect_case_conflicts(entries: &[(String, u64, bool)]) -> Vec<(String, String)> {
    use std::collections::HashMap;
    let mut dirs: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (path, _, _) in entries {
        let (parent, base) = match path.rsplit_once('/') {
            Some((parent, base)) => (parent, base),
            None => ("", path.as_str()),
        };
        dirs.entry(parent.to_lowercase())
            .or_default()
            .push((base.to_string(), path.clone()));
    }
    let mut out = Vec::new();
    for names in dirs.into_values() {
        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                if names[i].0.to_lowercase() == names[j].0.to_lowercase()
                    && names[i].0 != names[j].0
                {
                    out.push((names[j].1.clone(), names[i].1.clone()));
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_name() {
        assert!(is_windows_reserved("CON.txt"));
        assert!(is_windows_reserved("COM1"));
        assert!(!is_windows_reserved("config.txt"));
    }

    #[test]
    fn trailing_dot() {
        assert!(has_trailing_space_or_dot("a. "));
        assert!(has_trailing_space_or_dot("b."));
        assert!(!has_trailing_space_or_dot("c.txt"));
    }

    #[test]
    fn precheck_space() {
        let entries = vec![("a.txt".into(), 10u64, false)];
        let r = unpack_precheck(&entries, true, 5, 260);
        assert!(!r.can_start());
    }

    #[test]
    fn precheck_case_conflict_is_soft() {
        let entries = vec![
            ("sub/A.txt".into(), 1u64, false),
            ("sub/a.txt".into(), 1u64, false),
        ];
        let r = unpack_precheck(&entries, true, 1000, 260);
        assert!(r.has_kind(UnpackIssueKind::CaseConflict));
        assert!(!r.blocking());
    }

    #[test]
    fn precheck_invalid_names_are_blocking() {
        let entries = vec![
            ("CON.txt".into(), 1u64, false),
            ("bad?.txt".into(), 1u64, false),
            ("tail.".into(), 1u64, false),
        ];
        let r = unpack_precheck(&entries, true, 1000, 260);
        assert_eq!(
            r.issues
                .iter()
                .filter(|issue| issue.kind == UnpackIssueKind::IllegalName)
                .count(),
            3
        );
        assert!(r.blocking());
    }

    #[test]
    fn precheck_path_length_uses_output_base() {
        let entries = vec![("a.txt".into(), 1u64, false)];
        let r = unpack_precheck_full(&entries, true, 1000, 260, 260);
        assert!(r.has_kind(UnpackIssueKind::PathTooLong));
        let r = unpack_precheck_full(&entries, true, 1000, 100, 260);
        assert!(!r.has_kind(UnpackIssueKind::PathTooLong));
    }
}
