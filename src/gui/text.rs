//! 外壳文本投影：列表单元格 / 详情面板 / 状态栏（G0 阶段 1 外提）。
//!
//! 自 `src/bin/wd-pak-gui.rs` 逐字搬移，只调整可见性。
//! 时间格式化（`mtime_local` / `mtime_text`）依赖 Win32 FILETIME，保留在可执行目标，
//! 由调用方以闭包注入，因此本模块不引入 `windows_sys` 依赖。

use std::path::Path;

use crate::browse::Column;
use crate::format::{DIR_BIT, RAW_BIT};
use crate::session::{PakSession, SessionEntry, SessionStats};

/// 存储比文本；目录与原始大小为 0 的条目没有比值。
fn ratio_text(entry: &SessionEntry) -> String {
    if entry.is_dir() || entry.org == 0 {
        "—".into()
    } else {
        format!("{:.3}", entry.size as f64 / entry.org as f64)
    }
}

/// 条目在列表某一列上的文本。
pub fn entry_text(
    entry: &SessionEntry,
    column: Column,
    mtime_local: impl Fn(u32) -> Option<String>,
) -> String {
    match column {
        Column::Name if entry.is_dir() => format!("{}/", entry.path),
        Column::Name => entry.path.clone(),
        Column::Type if entry.is_dir() => "目录".into(),
        Column::Type => "文件".into(),
        Column::Org if entry.is_dir() => "—".into(),
        Column::Org => entry.org.to_string(),
        Column::Size if entry.is_dir() => "—".into(),
        Column::Size => entry.size.to_string(),
        Column::Compression if entry.is_dir() => "—".into(),
        Column::Compression if entry.is_raw() => "原样".into(),
        Column::Compression => "LZSS".into(),
        Column::Attrib => format!("0x{:08x}", entry.attrib),
        Column::Offset => format!("0x{:08x}", entry.offset),
        Column::Mtime if entry.is_dir() => "—".into(),
        Column::Mtime if entry.mtime == 0 => "未设置".into(),
        Column::Mtime => mtime_local(entry.mtime).unwrap_or_else(|| entry.mtime.to_string()),
        Column::Ratio => ratio_text(entry),
    }
}

/// 树视图的缩进行（名称列显示 basename，其余列与列表视图一致）。
pub fn tree_entry_text(
    entry: &SessionEntry,
    depth: usize,
    column: Column,
    mtime_local: impl Fn(u32) -> Option<String>,
) -> String {
    if column != Column::Name {
        return entry_text(entry, column, mtime_local);
    }
    let name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
    let suffix = if entry.is_dir() { "/" } else { "" };
    format!("{}{name}{suffix}", "    ".repeat(depth))
}

/// attrib 的中文串（硬约束 4：bit31 未置位即按 LZSS 读取，0 不等于「未定义」）。
pub fn attrib_text(attrib: u32) -> String {
    let mut parts = Vec::new();
    if attrib & DIR_BIT != 0 {
        parts.push("bit0=目录");
    }
    if attrib & RAW_BIT != 0 {
        parts.push("bit31=原样");
    }
    let rest = attrib & !DIR_BIT & !RAW_BIT;
    if rest != 0 {
        parts.push("其余位按原值");
    }
    if parts.is_empty() {
        parts.push("bit0/bit31 均未置位，按 LZSS 压缩读取");
    }
    format!("0x{attrib:08x}（{}）", parts.join("，"))
}

/// 详情面板文本：无包 / 未选中（包统计）/ 选中条目。
///
/// `mtime_text` 由调用方注入 Win32 本地时间格式化。
pub fn detail_text(
    session: Option<&PakSession>,
    selected: Option<&SessionEntry>,
    mtime_text: impl Fn(u32) -> String,
) -> String {
    let Some(session) = session else {
        return "未打开包\n\n点击“打开 PAK”".into();
    };
    if let Some(entry) = selected {
        let kind = if entry.is_dir() { "目录" } else { "文件" };
        let compression = if entry.is_dir() {
            "—"
        } else if entry.is_raw() {
            "原样 (RAW)"
        } else {
            "LZSS"
        };
        let ratio = ratio_text(entry);
        let name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
        // 目录没有独立的大小语义，按规格显示 “—”，不能用 0 臆造。
        let org = if entry.is_dir() {
            "—".into()
        } else {
            format!("{} 字节", entry.org)
        };
        let size = if entry.is_dir() {
            "—".into()
        } else {
            format!("{} 字节", entry.size)
        };
        return format!(
            "名称: {name}\n包内路径: {}\n类型: {kind}\n原始大小: {org}\n存储大小: {size}\n压缩: {compression}\n存储比: {ratio}\nattrib: {}\noffset: 0x{:08x}\nmtime: {}",
            entry.path,
            attrib_text(entry.attrib),
            entry.offset,
            mtime_text(entry.mtime)
        );
    }

    let stats = session.stats();
    let distribution = stats
        .attribs
        .iter()
        .map(|(attrib, count)| format!("0x{attrib:08x}x{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "包统计\n文件: {}\n目录: {}\nrootDirSize: {}\nfileSize: {}\n存储/原始: {} / {}\n存储比: {:.3}\nattrib 分布: {}\n\n选择一个条目查看详情",
        stats.files,
        stats.dirs,
        stats.root_dir_size,
        stats.file_size,
        stats.total_stored,
        stats.total_org,
        stats.ratio,
        distribution
    )
}

/// 多选时按文件统计的（文件数，原始大小合计，存储大小合计）；目录不计入大小。
fn selected_file_totals(selected: &[&SessionEntry]) -> (usize, u64, u64) {
    let mut files = 0;
    let mut org = 0u64;
    let mut stored = 0u64;
    for entry in selected.iter().filter(|entry| !entry.is_dir()) {
        files += 1;
        org += u64::from(entry.org);
        stored += u64::from(entry.size);
    }
    (files, org, stored)
}

/// 详情面板：多选汇总；0/1 项委托给 [`detail_text`]。
pub fn detail_text_multi(
    session: Option<&PakSession>,
    selected: &[&SessionEntry],
    mtime_text: impl Fn(u32) -> String,
) -> String {
    if selected.len() <= 1 {
        return detail_text(session, selected.first().copied(), mtime_text);
    }
    let (files, org, stored) = selected_file_totals(selected);
    let dirs = selected.len() - files;
    format!(
        "已选 {} 项（文件 {} / 目录 {}）\n合计原始: {org} 字节\n合计存储: {stored} 字节\n\n多选时仅保留“导出所选”操作",
        selected.len(),
        files,
        dirs
    )
}

/// 中间省略的长路径。
pub fn short_middle(value: &str, limit: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= limit || limit < 8 {
        return value.into();
    }
    let head = limit / 2 - 1;
    let tail = limit - head - 1;
    format!(
        "{}…{}",
        chars[..head].iter().collect::<String>(),
        chars[chars.len() - tail..].iter().collect::<String>()
    )
}

/// 状态栏公共前缀：路径与包统计。
fn status_prefix(path: &Path, stats: &SessionStats) -> String {
    format!(
        "{} | {} 个文件 / {} 个目录 | 存储 {} / 原始 {}（存储比 {:.3}）",
        short_middle(&path.display().to_string(), 60),
        stats.files,
        stats.dirs,
        stats.total_stored,
        stats.total_org,
        stats.ratio
    )
}

/// 状态栏文本（三段）。
pub fn status_text(
    path: Option<&Path>,
    session: Option<&PakSession>,
    selected: Option<&SessionEntry>,
) -> String {
    let Some(path) = path else {
        return "未打开包".into();
    };
    let Some(session) = session else {
        return path.display().to_string();
    };
    let selected = selected
        .map(|entry| entry.path.clone())
        .unwrap_or_else(|| "未选中".into());
    format!("{} | {selected}", status_prefix(path, session.stats()))
}

/// 状态栏：多选时以汇总替代单项路径。
pub fn status_text_multi(
    path: Option<&Path>,
    session: Option<&PakSession>,
    selected: &[&SessionEntry],
) -> String {
    if selected.len() <= 1 {
        return status_text(path, session, selected.first().copied());
    }
    let (Some(path), Some(session)) = (path, session) else {
        return status_text(path, session, None);
    };
    let (_, org, stored) = selected_file_totals(selected);
    format!(
        "{} | 已选 {} 项 / 合计原始 {} / 存储 {}",
        status_prefix(path, session.stats()),
        selected.len(),
        org,
        stored
    )
}
