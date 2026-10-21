//! 打包纯逻辑（G4-03/04/05）。

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::error::PakError;
use crate::format::{CatalogEntry, MAX_NAME};
use crate::manifest::MANIFEST_NAME;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackCompression {
    #[default]
    Lzss,
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackOptions {
    pub compression: PackCompression,
    pub jobs: usize,
    pub use_manifest: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            compression: PackCompression::Lzss,
            jobs: default_jobs(),
            use_manifest: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackEntry {
    pub rel_path: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackIssueKind {
    NameInvalid,
    NameTooLong,
    NonAscii,
    CaseConflict,
    TooLarge,
    ManifestMtime,
    NotWritable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackIssue {
    pub kind: PackIssueKind,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackPrecheckResult {
    pub issues: Vec<PackIssue>,
    pub file_count: usize,
    pub dir_count: usize,
    pub total_size: u64,
}

impl PackPrecheckResult {
    pub fn can_start(&self) -> bool {
        self.issues.is_empty()
    }
}

/// 默认并行度 `min(CPU 核数, 8)`（对齐 spec 与 CLI `default_jobs`）。
pub fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8)
}

/// 校验单个打包侧名字（OS 名 `&str`）：先拒非 Latin-1（字符 > U+00FF，无法单字节编码），
/// 再编码为 Latin-1 字节后套用 [`CatalogEntry::validate_name`] 的既有规则。
pub fn validate_pack_name(name: &str) -> Result<(), PackIssue> {
    if name.chars().any(|c| c as u32 > 0xFF) {
        return Err(PackIssue {
            kind: PackIssueKind::NonAscii,
            path: name.to_string(),
            detail: PakError::NameNonAscii(name.to_string()).to_string(),
        });
    }
    let bytes: Vec<u8> = name.chars().map(|c| c as u8).collect();
    if let Err(e) = CatalogEntry::validate_name(&bytes) {
        return Err(PackIssue {
            kind: if bytes.len() > MAX_NAME {
                PackIssueKind::NameTooLong
            } else {
                PackIssueKind::NameInvalid
            },
            path: name.to_string(),
            detail: e.to_string(),
        });
    }
    Ok(())
}

/// 检测同层大小写冲突（`names` 为同一目录下的名字）。
pub fn detect_case_conflicts(names: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in 0..names.len() {
        for j in (i + 1)..names.len() {
            if names[i].eq_ignore_ascii_case(&names[j]) && names[i] != names[j] {
                out.push((names[i].clone(), names[j].clone()));
            }
        }
    }
    out
}

/// 按父目录分组检测大小写冲突；返回完整相对路径对。
pub fn detect_path_case_conflicts(entries: &[PackEntry]) -> Vec<(String, String)> {
    let mut dirs: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for entry in entries {
        let (parent, base) = split_parent(&entry.rel_path);
        dirs.entry(parent.to_lowercase())
            .or_default()
            .push((base.to_string(), entry.rel_path.clone()));
    }
    let mut out = Vec::new();
    for mut names in dirs.into_values() {
        names.sort_by(|a, b| a.0.cmp(&b.0));
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

fn split_parent(rel: &str) -> (&str, &str) {
    match rel.rsplit_once('/') {
        Some((parent, base)) => (parent, base),
        None => ("", rel),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackScan {
    pub root: PathBuf,
    pub entries: Vec<PackEntry>,
    pub manifest_present: bool,
    pub ignored_manifest: Vec<String>,
}

impl PackScan {
    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.is_dir).count()
    }

    pub fn dir_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_dir).count()
    }

    pub fn total_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| entry.size)
            .sum()
    }
}

/// 扫描源目录：跳过 `_pak_manifest.json`（与 CLI `_collect_files` 一致），
/// 记录其余文件/目录与总大小；符号链接/Windows 重解析点整条跳过（不收集、不递归），
/// `is_dir`/`size` 均取 lstat 语义（`DirEntry::file_type`/`metadata`，不跟随链接）。
pub fn scan_dir(root: &Path) -> std::io::Result<PackScan> {
    let mut scan = PackScan {
        root: root.to_path_buf(),
        ..Default::default()
    };
    scan_rec(root, "", &mut scan)?;
    scan.entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(scan)
}

fn scan_rec(dir: &Path, prefix: &str, scan: &mut PackScan) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == MANIFEST_NAME {
            scan.manifest_present = true;
            scan.ignored_manifest.push(format!("{prefix}{name}"));
            continue;
        }
        let full = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || crate::platform::is_reparse_point(&full) {
            continue;
        }
        let is_dir = file_type.is_dir();
        let rel = format!("{prefix}{name}");
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        scan.entries.push(PackEntry {
            rel_path: rel.clone(),
            size,
            is_dir,
        });
        if is_dir {
            scan_rec(&full, &format!("{rel}/"), scan)?;
        }
    }
    Ok(())
}

/// 逐段校验包内路径（每段都必须是合法 Latin-1 名）。
pub fn validate_pack_rel_path(rel: &str) -> Result<(), PackIssue> {
    for part in rel.split('/').filter(|part| !part.is_empty()) {
        validate_pack_name(part)?;
    }
    Ok(())
}

/// manifest 中 mtime 非法的条目（`files` 值非对象时整份忽略，对齐 CLI 宽松读取）。
/// 只校验 `scanned` 中实际存在的源路径：陈旧 manifest 条目不会被打包消费，不报告。
pub fn manifest_mtime_issues(root: &Path, scanned: &BTreeSet<String>) -> Vec<PackIssue> {
    let Ok(text) = std::fs::read_to_string(root.join(MANIFEST_NAME)) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(files) = value.get("files").and_then(|files| files.as_object()) else {
        return Vec::new();
    };
    if files.values().any(|item| !item.is_object()) {
        return Vec::new();
    }
    let mut issues = Vec::new();
    for (rel, item) in files {
        if !scanned.contains(rel) {
            continue;
        }
        let mtime = item.get("mtime");
        let valid = match mtime {
            None => true,
            Some(serde_json::Value::Number(number)) => {
                number.as_u64().is_some_and(|v| v <= u64::from(u32::MAX))
            }
            Some(_) => false,
        };
        if !valid {
            issues.push(PackIssue {
                kind: PackIssueKind::ManifestMtime,
                path: rel.clone(),
                detail: format!(
                    "manifest 中 {rel} 的 mtime 非法: {}",
                    mtime.map(json_repr).unwrap_or_else(|| "None".into())
                ),
            });
        }
    }
    issues.sort_by(|a, b| a.path.cmp(&b.path));
    issues
}

fn json_repr(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "None".into(),
        serde_json::Value::Bool(true) => "True".into(),
        serde_json::Value::Bool(false) => "False".into(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => format!("'{text}'"),
        other => other.to_string(),
    }
}

/// jobs 限制到 1..=8。
pub fn clamp_jobs(requested: usize, cpu: usize) -> usize {
    requested.clamp(1, cpu.clamp(1, 8))
}

/// 打包预检。
pub fn pack_precheck(entries: &[PackEntry], max_file_size: u64) -> PackPrecheckResult {
    let mut issues = Vec::new();
    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut total_size = 0u64;
    for (a, b) in detect_path_case_conflicts(entries) {
        issues.push(PackIssue {
            kind: PackIssueKind::CaseConflict,
            path: a.clone(),
            detail: format!("大小写冲突: {} / {}", a, b),
        });
    }
    for e in entries {
        if e.is_dir {
            dir_count += 1;
        } else {
            file_count += 1;
            total_size += e.size;
            if e.size > max_file_size {
                issues.push(PackIssue {
                    kind: PackIssueKind::TooLarge,
                    path: e.rel_path.clone(),
                    detail: "超过 4GB 限制".into(),
                });
            }
        }
        if e.rel_path.is_empty() || e.rel_path.split('/').any(str::is_empty) {
            issues.push(PackIssue {
                kind: PackIssueKind::NameInvalid,
                path: e.rel_path.clone(),
                detail: "空文件名或空路径段".into(),
            });
        } else if let Err(issue) = validate_pack_rel_path(&e.rel_path) {
            issues.push(PackIssue {
                path: if issue.path.is_empty() {
                    e.rel_path.clone()
                } else {
                    format!("{}: {}", e.rel_path, issue.path)
                },
                ..issue
            });
        }
    }
    // CLI 在写出阶段拒绝 HDR + 目录树超过 4GB；源文件合计超过上限时必然无法写出，
    // 预检提前阻断（压缩后可能变小，因此只在上限本身被突破时报告）。
    if total_size > u64::from(u32::MAX) {
        issues.push(PackIssue {
            kind: PackIssueKind::TooLarge,
            path: String::new(),
            detail: "整包超过 4GB 限制".into(),
        });
    }
    PackPrecheckResult {
        issues,
        file_count,
        dir_count,
        total_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_jobs_bounds() {
        assert_eq!(clamp_jobs(100, 4), 4);
        assert_eq!(clamp_jobs(0, 4), 1);
        assert_eq!(clamp_jobs(2, 16), 2);
    }

    #[test]
    fn case_conflict_detected() {
        let names = vec!["a.txt".into(), "A.txt".into()];
        assert!(!detect_case_conflicts(&names).is_empty());
    }

    #[test]
    fn validate_name_rejects_slash() {
        assert!(validate_pack_name("a/b").is_err());
    }

    #[test]
    fn default_jobs_is_min_cpu_8() {
        // spec: 并行度默认 min(CPU 核数, 8)
        let expected = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(8);
        assert_eq!(PackOptions::default().jobs, expected);
    }

    #[test]
    fn validate_name_rejects_non_latin1() {
        let err = validate_pack_name("中文.txt").unwrap_err();
        assert_eq!(err.kind, PackIssueKind::NonAscii);
    }

    #[test]
    fn validate_name_accepts_latin1_high_chars() {
        assert!(validate_pack_name("caf\u{e9}.txt").is_ok());
    }

    #[test]
    fn precheck_rejects_non_latin1_name() {
        let entries = vec![PackEntry {
            rel_path: "sub/中文.txt".into(),
            size: 1,
            is_dir: false,
        }];
        let r = pack_precheck(&entries, 1000);
        assert!(!r.can_start());
        assert!(r.issues.iter().any(|i| i.kind == PackIssueKind::NonAscii));
    }

    #[test]
    fn precheck_rejects_whole_package_over_4gb() {
        let entries = vec![
            PackEntry { rel_path: "a".into(), size: 3 << 30, is_dir: false },
            PackEntry { rel_path: "b".into(), size: 2 << 30, is_dir: false },
        ];
        let r = pack_precheck(&entries, u64::from(u32::MAX));
        assert!(!r.can_start());
        assert!(r
            .issues
            .iter()
            .any(|issue| issue.kind == PackIssueKind::TooLarge && issue.path.is_empty()));
    }

    #[test]
    fn precheck_rejects_empty_path_segment() {
        let entries = vec![PackEntry {
            rel_path: "a//b.txt".into(),
            size: 1,
            is_dir: false,
        }];
        let r = pack_precheck(&entries, 1000);
        assert!(!r.can_start());
        assert!(r.issues.iter().any(|issue| issue.kind == PackIssueKind::NameInvalid));
    }

    #[test]
    fn precheck_counts() {
        let entries = vec![
            PackEntry { rel_path: "dir".into(), size: 0, is_dir: true },
            PackEntry { rel_path: "file.txt".into(), size: 10, is_dir: false },
        ];
        let r = pack_precheck(&entries, 1000);
        assert_eq!(r.file_count, 1);
        assert_eq!(r.dir_count, 1);
        assert!(r.can_start());
    }

    #[test]
    fn scan_dir_skips_manifest_and_counts() {
        let root = std::env::temp_dir().join(format!(
            "wd-pak-pack-scan-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), b"abc").unwrap();
        std::fs::write(root.join("sub/b.txt"), b"defgh").unwrap();
        std::fs::write(root.join(MANIFEST_NAME), b"{}").unwrap();
        let scan = scan_dir(&root).unwrap();
        assert!(scan.manifest_present);
        assert_eq!(scan.ignored_manifest, vec![MANIFEST_NAME.to_string()]);
        assert_eq!(scan.file_count(), 2);
        assert_eq!(scan.dir_count(), 1);
        assert_eq!(scan.total_size(), 8);
        assert!(scan.entries.iter().all(|entry| !entry.rel_path.contains(MANIFEST_NAME)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn case_conflicts_are_per_directory() {
        let entries = vec![
            PackEntry { rel_path: "a/x.txt".into(), size: 1, is_dir: false },
            PackEntry { rel_path: "a/X.txt".into(), size: 1, is_dir: false },
            PackEntry { rel_path: "b/x.txt".into(), size: 1, is_dir: false },
        ];
        let conflicts = detect_path_case_conflicts(&entries);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].0.ends_with("a/X.txt") || conflicts[0].0.ends_with("a/x.txt"));
    }

    #[test]
    fn precheck_checks_every_path_component() {
        let entries = vec![PackEntry {
            rel_path: "中文/file.txt".into(),
            size: 1,
            is_dir: false,
        }];
        let r = pack_precheck(&entries, 1000);
        assert!(!r.can_start());
        assert!(r.issues.iter().any(|issue| issue.kind == PackIssueKind::NonAscii));
    }

    #[test]
    fn manifest_mtime_issues_flagged() {
        let root = std::env::temp_dir().join(format!(
            "wd-pak-pack-mtime-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(MANIFEST_NAME),
            br#"{"files":{"a.txt":{"mtime":4294967296},"b.txt":{"mtime":1},"stale.txt":{"mtime":-1}}}"#,
        )
        .unwrap();
        let scanned: BTreeSet<String> = ["a.txt".to_string(), "b.txt".to_string()]
            .into_iter()
            .collect();
        let issues = manifest_mtime_issues(&root, &scanned);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "a.txt");
        assert_eq!(issues[0].kind, PackIssueKind::ManifestMtime);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reparse_point_false_for_normal_dir() {
        let root = std::env::temp_dir().join(format!(
            "wd-pak-reparse-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(!crate::platform::is_reparse_point(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn scan_dir_skips_symlinks() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "wd-pak-pack-link-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::write(root.join("real/a.txt"), b"abc").unwrap();
        std::fs::write(root.join("file.txt"), b"defgh").unwrap();
        symlink(root.join("real"), root.join("linkdir")).unwrap();
        symlink(root.join("file.txt"), root.join("linkfile.txt")).unwrap();
        let scan = scan_dir(&root).unwrap();
        assert!(scan
            .entries
            .iter()
            .all(|entry| !entry.rel_path.starts_with("link")));
        assert_eq!(scan.file_count(), 2);
        assert_eq!(scan.dir_count(), 1);
        assert_eq!(scan.total_size(), 8);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
