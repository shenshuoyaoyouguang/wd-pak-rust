//! 导出与预览纯逻辑（G3）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 导出范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportScope {
    #[default]
    Selected,
    All,
}

/// 导出冲突决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportConflict {
    #[default]
    Pending,
    Replace,
    Skip,
    Rename,
}

/// 导出选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    pub target_dir: PathBuf,
    pub scope: ExportScope,
    pub overwrite: bool,
    pub keep_structure: bool,
}

impl ExportOptions {
    pub fn new(target_dir: PathBuf) -> Self {
        Self {
            target_dir,
            scope: ExportScope::Selected,
            overwrite: false,
            keep_structure: true,
        }
    }
}

/// 导出计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPlan {
    pub entries: Vec<String>,
    pub target_dir: PathBuf,
    pub keep_structure: bool,
}

impl ExportPlan {
    pub fn new(entries: Vec<String>, target_dir: PathBuf, keep_structure: bool) -> Self {
        Self {
            entries,
            target_dir,
            keep_structure,
        }
    }

    pub fn target_path(&self, inner: &str) -> PathBuf {
        if self.keep_structure {
            self.target_dir.join(inner.replace('/', std::path::MAIN_SEPARATOR_STR))
        } else {
            self.target_dir
                .join(Path::new(inner).file_name().unwrap_or_default())
        }
    }
}

/// 默认导出文件名（basename）。
pub fn default_export_name(inner: &str) -> String {
    Path::new(inner)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| inner.to_string())
}

/// 预览类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewKind {
    #[default]
    Text,
    Hex,
}

/// 预览数据（1 MB 截断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewData {
    pub content: String,
    pub truncated: bool,
    pub original_size: usize,
    pub limit: usize,
    pub kind: PreviewKind,
}

pub const PREVIEW_LIMIT: usize = 1024 * 1024;

/// 十六进制视图最多渲染的源字节数：1 MB 数据会膨胀成约 5 MB 文本，
/// 一次性塞进 EDIT 会明显卡顿；文本视图仍按 `PREVIEW_LIMIT`。
pub const HEX_PREVIEW_LIMIT: usize = 128 * 1024;

/// 前 8 KB 中出现 NUL，或控制字节占比超过 10%，视为二进制。
pub fn looks_binary(data: &[u8]) -> bool {
    let sample = &data[..data.len().min(8 * 1024)];
    if sample.is_empty() {
        return false;
    }
    let mut control = 0usize;
    for &byte in sample {
        if byte == 0 {
            return true;
        }
        if byte < 0x09 || (0x0E..0x20).contains(&byte) || byte == 0x7F {
            control += 1;
        }
    }
    control * 10 > sample.len()
}

/// 采样是否为可读 UTF-8；非 UTF-8（如 GBK）默认走十六进制，避免乱码文本。
fn text_looks_utf8(data: &[u8]) -> bool {
    let sample = &data[..data.len().min(8 * 1024)];
    if std::str::from_utf8(sample).is_ok() {
        return true;
    }
    // 采样截断可能落在多字节字符中间，允许末尾少量 replacement。
    let lossy = String::from_utf8_lossy(sample);
    let total = lossy.chars().count().max(1);
    let bad = lossy.chars().filter(|&c| c == '\u{FFFD}').count();
    bad * 20 <= total
}

/// 按内容选择默认预览：二进制或非 UTF-8 走十六进制，其余走文本。
pub fn preview_for(data: &[u8], limit: usize) -> PreviewData {
    if looks_binary(data) || !text_looks_utf8(data) {
        preview_hex(data, limit.min(HEX_PREVIEW_LIMIT))
    } else {
        preview_truncate(data, limit)
    }
}

pub fn preview_truncate(data: &[u8], limit: usize) -> PreviewData {
    let original_size = data.len();
    let truncated = original_size > limit;
    let slice = if truncated { &data[..limit] } else { data };
    PreviewData {
        content: String::from_utf8_lossy(slice).into_owned(),
        truncated,
        original_size,
        limit,
        kind: PreviewKind::Text,
    }
}

pub fn preview_hex(data: &[u8], limit: usize) -> PreviewData {
    let original_size = data.len();
    let truncated = original_size > limit;
    let slice = if truncated { &data[..limit] } else { data };
    let mut out = String::with_capacity(slice.len().saturating_mul(5));
    for (line, chunk) in slice.chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", line * 16));
        for i in 0..16 {
            if let Some(byte) = chunk.get(i) {
                out.push_str(&format!("{byte:02x} "));
            } else {
                out.push_str("   ");
            }
            if i == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        for &byte in chunk {
            out.push(if (0x20..0x7F).contains(&byte) {
                byte as char
            } else {
                '.'
            });
        }
        out.push_str("|\n");
    }
    PreviewData {
        content: out,
        truncated,
        original_size,
        limit,
        kind: PreviewKind::Hex,
    }
}

/// 计划内的路径去重键（Windows 大小写不敏感）。
pub fn plan_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// 为冲突的导出目标生成不重复的 `名称 (1).扩展名`；`taken` 记录本次计划内已占用路径。
pub fn unique_target(target: &Path, taken: &mut HashSet<String>) -> PathBuf {
    if !target.exists() && taken.insert(plan_key(target)) {
        return target.to_path_buf();
    }
    let parent = target.parent().unwrap_or_else(|| Path::new(""));
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    let extension = target
        .extension()
        .map(|s| s.to_string_lossy().into_owned());
    for ordinal in 1..=9999 {
        let name = match &extension {
            Some(extension) => format!("{stem} ({ordinal}).{extension}"),
            None => format!("{stem} ({ordinal})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() && taken.insert(plan_key(&candidate)) {
            return candidate;
        }
    }
    target.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_basename() {
        assert_eq!(default_export_name("a/b/c.txt"), "c.txt");
        assert_eq!(default_export_name("x.pak"), "x.pak");
    }

    #[test]
    fn preview_truncate_flags() {
        let data = vec![b'a'; 10];
        let p = preview_truncate(&data, 5);
        assert!(p.truncated);
        assert_eq!(p.content.len(), 5);
        assert_eq!(p.original_size, 10);
    }

    #[test]
    fn preview_no_truncate() {
        let data = b"hi".to_vec();
        let p = preview_truncate(&data, 5);
        assert!(!p.truncated);
        assert_eq!(p.content, "hi");
    }

    #[test]
    fn export_plan_target_path() {
        let plan = ExportPlan::new(vec!["a/b.txt".into()], PathBuf::from("/out"), true);
        let p = plan.target_path("a/b.txt");
        assert!(p.to_string_lossy().contains("a"));
    }

    #[test]
    fn binary_detection() {
        assert!(!looks_binary(b"plain text\nline two"));
        assert!(looks_binary(b"abc\0def"));
        assert!(looks_binary(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]));
    }

    #[test]
    fn preview_for_picks_hex_for_binary() {
        assert_eq!(preview_for(b"hello", 1024).kind, PreviewKind::Text);
        let binary = preview_for(&[0u8, 1, 2, 3], 1024);
        assert_eq!(binary.kind, PreviewKind::Hex);
        assert!(binary.content.contains("00000000"));
    }

    #[test]
    fn hex_dump_has_offset_and_ascii() {
        let preview = preview_hex(b"AB", 1024);
        assert!(preview.content.starts_with("00000000  41 42 "));
        assert!(preview.content.contains("|AB|"));
    }

    #[test]
    fn hex_truncates_at_limit() {
        let preview = preview_hex(&[0u8; 32], 16);
        assert!(preview.truncated);
        assert_eq!(preview.limit, 16);
        assert!(!preview.content.contains("00000010"));
    }

    #[test]
    fn unique_target_avoids_duplicates() {
        let mut taken = HashSet::new();
        let first = unique_target(Path::new("out/example.txt"), &mut taken);
        assert_eq!(first, PathBuf::from("out/example.txt"));
        let second = unique_target(Path::new("out/example.txt"), &mut taken);
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "example (1).txt"
        );
    }
}
