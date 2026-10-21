//! 包会话：界面层的只读包快照。
//!
//! 只负责把 CLI 的解析结果整理成界面可直接消费的结构；
//! 不改变包内容，也不复制 CLI 的文本输出。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::PakError;
use crate::format::{DIR_BIT, RAW_BIT};
use crate::pak::PakReader;

/// 会话统计，口径与 `list`/`check` 报告头部一致。
///
/// `ratio` 与 CLI 一致：`total_stored / total_org.max(1)`；
/// 条目级存储比在 `browse` 中单独定义为“原始大小为 0 时无定义”。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionStats {
    pub files: usize,
    pub dirs: usize,
    pub root_dir_size: u32,
    pub file_size: u64,
    pub total_stored: u64,
    pub total_org: u64,
    pub ratio: f64,
    pub attribs: BTreeMap<u32, usize>,
}

/// 一个条目的只读快照；`path` 为包内相对路径（`/` 分隔）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub path: String,
    pub attrib: u32,
    pub offset: u32,
    pub size: u32,
    pub org: u32,
    pub mtime: u32,
}

impl SessionEntry {
    pub fn is_dir(&self) -> bool {
        self.attrib & DIR_BIT != 0
    }

    pub fn is_raw(&self) -> bool {
        self.attrib & RAW_BIT != 0
    }
}

/// 包文件指纹：大小 + 修改时间，用于判定快照/校验结果是否过期。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFingerprint {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

fn fingerprint_of(meta: &std::fs::Metadata) -> FileFingerprint {
    FileFingerprint {
        size: meta.len(),
        modified: meta.modified().ok(),
    }
}

/// 已打开的包会话（只读）。
#[derive(Debug)]
pub struct PakSession {
    path: PathBuf,
    fingerprint: FileFingerprint,
    reader: PakReader,
    entries: Vec<SessionEntry>,
    stats: SessionStats,
}

impl PakSession {
    /// 打开并解析包；错误原样沿用 CLI 的错误类型与文案。
    pub fn open(path: &Path) -> Result<Self, PakError> {
        Self::open_with_progress(path, |_| {}, None)
    }

    /// 打开并解析包，支持进度回调（每处理一条目调用一次）与可选取消标志。
    ///
    /// `progress` 收到已处理条目数；`cancel` 置位时在遍历期间提前返回错误，
    /// 调用方（如后台加载器）通常丢弃该错误并自行按取消处理。
    pub fn open_with_progress(
        path: &Path,
        mut progress: impl FnMut(usize),
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Self, PakError> {
        use std::sync::atomic::Ordering;

        let reader = crate::commands_read::open_pak(path)?;
        let (mut files, mut dirs) = (0usize, 0usize);
        let (mut total_stored, mut total_org) = (0u64, 0u64);
        let mut attribs: BTreeMap<u32, usize> = BTreeMap::new();
        let mut entries = Vec::new();
        let mut cancelled = false;
        reader.walk_with_progress(|rel, node| {
            if cancelled || cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                cancelled = true;
                return;
            }
            if node.is_dir() {
                dirs += 1;
            } else {
                files += 1;
                total_stored += node.entry.size as u64;
                total_org += node.entry.org as u64;
                *attribs.entry(node.entry.attrib).or_insert(0) += 1;
            }
            let e = &node.entry;
            entries.push(SessionEntry {
                path: rel.to_string(),
                attrib: e.attrib,
                offset: e.offset,
                size: e.size,
                org: e.org,
                mtime: e.mtime,
            });
            progress(entries.len());
        });
        if cancelled {
            return Err(PakError::Io(std::io::Error::other("加载已取消")));
        }
        let root_dir_size = reader.root_size;
        let file_size = reader.file_size;
        let ratio = total_stored as f64 / total_org.max(1) as f64;
        let fingerprint = fingerprint_of(&std::fs::metadata(path)?);
        Ok(Self {
            path: path.to_path_buf(),
            fingerprint,
            reader,
            entries,
            stats: SessionStats {
                files,
                dirs,
                root_dir_size,
                file_size,
                total_stored,
                total_org,
                ratio,
                attribs,
            },
        })
    }

    pub fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }

    pub fn stats(&self) -> &SessionStats {
        &self.stats
    }

    pub fn fingerprint(&self) -> FileFingerprint {
        self.fingerprint
    }

    /// 包文件自打开后是否未被外部修改（大小与修改时间都未变）。
    pub fn is_current(&self) -> bool {
        std::fs::metadata(&self.path)
            .map(|m| fingerprint_of(&m) == self.fingerprint)
            .unwrap_or(false)
    }

    /// 按包内路径读取原始字节（RAW 与 LZSS 均由读取通道还原）。
    /// 错误语义与导出（`cat`）一致：缺失为“不存在: …”，目录为“是目录”。
    pub fn read(&self, inner: &str) -> Result<Vec<u8>, PakError> {
        self.reader.read_entry_by_path(inner)
    }
}
