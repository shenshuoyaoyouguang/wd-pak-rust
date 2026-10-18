//! 目录树解析 —— 逐行对齐 Python `parse_tree` 的防御顺序与错误语义。
//!
//! 防御要点（与 Python 版一致，且全部以 u64 算术防 u32 回绕）：
//!   1. 嵌套深度 > 64        -> PakError::TreeTooDeep(offset)
//!   2. (offset,size) 重复引用 -> PakError::TreeCycle(offset)（跨整树传递的 seen 集合）
//!   3. size 非 64 倍数        -> PakError::CatalogBadSize
//!   4. offset+size 超文件长   -> PakError::CatalogOob（Python 靠 read 不足隐式实现，
//!      两个 u32 在 u32 域相加可能回绕绕过检查，此处必须显式 u64 比较）
//!   5. 条目名非法            -> PakError::BadName / NameTooLong（format.rs 解码时校验）

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};

use crate::error::PakError;
use crate::format::{CatalogEntry, CAT_SIZE, DIR_BIT};

/// 目录最大嵌套深度（根表为第 0 层）
pub const MAX_DEPTH: usize = 64;

/// 节点数据语义：目录（子节点列表）或文件（数据区定位三元组，值为 entry 字段快照）。
#[derive(Debug)]
pub enum NodePayload {
    /// 目录：offset/size 指向子目录表
    Dir(Vec<TreeNode>),
    /// 文件：offset/size 指向数据区，org 为原始（解压后）大小
    File { offset: u32, size: u32, org: u32 },
}

/// 目录树节点：64 字节目录项 + 解析后的数据语义。
#[derive(Debug)]
pub struct TreeNode {
    pub entry: CatalogEntry,
    pub payload: NodePayload,
}

impl TreeNode {
    /// 是否目录节点（与 entry.is_dir() 恒一致，构造时保证）。
    pub fn is_dir(&self) -> bool {
        matches!(self.payload, NodePayload::Dir(_))
    }
}

/// 解析目录表（对应 Python parse_tree）。
///
/// * `f`        —— 已打开的 pak 文件（任意可读可寻址流，便于单元测试注入）
/// * `offset`/`size` —— 目录表位置与字节长度
/// * `depth`    —— 当前嵌套深度（根调用传 0）
/// * `seen`     —— 已引用过的 (offset,size) 集合，跨整个解析过程传递
/// * `file_len` —— pak 文件总长（u64 越界检查依据；Python 靠 read 不足隐式实现）
///
/// 目录条目（bit0=1）递归解析其 offset/size 指向的子目录表；文件条目止于叶节点。
/// 条目按出现顺序逐个解码，名字校验先于其子表递归（对齐 Python 循环内次序）。
pub fn parse_tree<R: Read + Seek>(
    f: &mut R,
    offset: u64,
    size: u64,
    depth: usize,
    seen: &mut HashSet<(u64, u64)>,
    file_len: u64,
) -> Result<Vec<TreeNode>, PakError> {
    if depth > MAX_DEPTH {
        return Err(PakError::TreeTooDeep(offset));
    }
    // insert 返回 false 即已存在（Python: in 检查 + add，语义相同）
    if !seen.insert((offset, size)) {
        return Err(PakError::TreeCycle(offset));
    }
    if size % CAT_SIZE as u64 != 0 {
        return Err(PakError::CatalogBadSize { offset, size });
    }
    // u64 域越界检查：u32 域相加回绕（如 0xFFFFFFF0+0x40=0x30）会被此检查拦截
    if offset.saturating_add(size) > file_len {
        return Err(PakError::CatalogOob { offset, size });
    }
    f.seek(SeekFrom::Start(offset))?;
    let mut blob = vec![0u8; size as usize];
    f.read_exact(&mut blob)?; // 预检通过后仅并发截断才可能失败 -> Io

    let mut nodes = Vec::with_capacity((size / CAT_SIZE as u64) as usize);
    for chunk in blob.chunks_exact(CAT_SIZE) {
        let mut raw = [0u8; CAT_SIZE];
        raw.copy_from_slice(chunk);
        let entry = CatalogEntry::from_bytes(&raw)?; // BadName/NameTooLong 在此抛出
        if entry.attrib & DIR_BIT != 0 {
            let children = parse_tree(
                f,
                entry.offset as u64,
                entry.size as u64,
                depth + 1,
                seen,
                file_len,
            )?;
            nodes.push(TreeNode {
                entry,
                payload: NodePayload::Dir(children),
            });
        } else {
            nodes.push(TreeNode {
                payload: NodePayload::File {
                    offset: entry.offset,
                    size: entry.size,
                    org: entry.org,
                },
                entry,
            });
        }
    }
    Ok(nodes)
}