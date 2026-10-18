//! 写路径 —— pack / replace，逐字节对齐 Python `cmd_pack` / `cmd_replace`
//! 及其私有依赖（_preflight / _collect_files / _preload_files / _build_dir /
//! write_pak / _serialize / _apply_manifest_mtime / clone 树）。
//!
//! 与读侧的两个关键差异：
//!   * 名字域：OS 文件名（字节）→ Python str 域校验（_validate_name：长度按
//!     字符数、surrogateescape、Latin-1 可编码）→ pak 内 Latin-1 字节。
//!     无效 UTF-8 的 OS 名在 Python 中必产生 surrogate 而 Latin-1 编码失败，
//!     Rust 以字节域等价推导复刻（见 validate_os_name）。
//!   * 树模型：独立写侧树（offset 为 serialize 递推赋值的规划值，payload 区分
//!     Inline 驻内存 / FromPak 流式拷贝），不复用读侧 tree::TreeNode 快照模型。
//!
//! 怪癖复刻（design 2.10）：空目录共享 offset（自产 pak 读回 TreeCycle）、
//! 空名报"文件名过长"、相邻目录序与 manifest order 的排序键、mtime 恢复语义。

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::commands_read::{open_pak, py_os_error};
use crate::error::PakError;
use crate::format::{
    latin1, CatalogEntry, Header, CAT_SIZE, DIR_BIT, HDR_SIZE, MAGIC, MAX_NAME, RAW_BIT,
};
use crate::lzss;
use crate::manifest::MANIFEST_NAME;
use crate::tree::{NodePayload, TreeNode};

// ---------------------------------------------------------------------------
// Python 显示格式辅助（repr / OSError 消息）
// ---------------------------------------------------------------------------

/// OS 名的原始字节（Unix：OsStrExt；Windows 退化 lossy，design 2.11 R5）。
fn os_bytes(nm: &OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        nm.as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        nm.to_string_lossy().into_owned().into_bytes()
    }
}

/// repr 字符串字面量的内部（转义体）：'\\' 与控制字符转义，引号按所选字面量转义。
fn py_repr_body(out: &mut String, s: &str, quote: char) {
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            // ASCII 控制字符与 DEL：Python repr 用 \xNN（小写十六进制）
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
}

/// Python repr(字符串)：默认单引号；含 ' 且不含 " 时改用双引号（CPython 规则）。
/// （pub(crate)：OSError 消息的 filename 部分用 repr 包裹，读侧 py_os_error 复用）
pub(crate) fn py_str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    py_repr_body(&mut out, s, quote);
    out.push(quote);
    out
}

/// OS 名（字节）的 Python repr 内部（无外层引号）：
/// 有效 UTF-8 段按 repr 转义，无效字节逐个转义为 `\udcxx`
/// （surrogateescape 解码后 repr 的显示形态）。
/// 引号选择基于原始字节（surrogate 字符不含引号，等价）。
fn py_os_repr_inner(bytes: &[u8]) -> String {
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                py_repr_body(&mut out, s, quote);
                break;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                let bad_len = e.error_len().unwrap_or(rest.len() - valid);
                // valid_up_to 保证前缀有效，unwrap 不触发
                let s = std::str::from_utf8(&rest[..valid]).unwrap();
                py_repr_body(&mut out, s, quote);
                for b in &rest[valid..valid + bad_len] {
                    out.push_str(&format!("\\udc{b:02x}"));
                }
                rest = &rest[valid + bad_len..];
            }
        }
    }
    out
}

/// Python repr(JSON 值)（mtime 校验错误消息 `{mt!r}` 的显示形态）。
fn py_repr(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(), // 整数十进制直出；浮点最短往返（≈Python repr）
        Value::String(s) => py_str_repr(s),
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(py_repr).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(o) => {
            let items: Vec<String> = o
                .iter()
                .map(|(k, v)| format!("{}: {}", py_str_repr(k), py_repr(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// 带双 filename 的 OSError 消息（os.replace 类）：`[Errno N] msg: {repr(a)} -> {repr(b)}`。
fn py_oserror2(e: &io::Error, from: &Path, to: &Path) -> String {
    format!(
        "{}: {} -> {}",
        crate::commands_read::py_errno_msg(e),
        py_str_repr(&from.display().to_string()),
        py_str_repr(&to.display().to_string())
    )
}

// ---------------------------------------------------------------------------
// OS 文件名校验（Python _validate_name 的字节域等价实现）
// ---------------------------------------------------------------------------

/// OS 名校验，对齐 Python `_validate_name`（str 域）的三步规则与报错顺序：
/// 1) 空名或字符数 > 43 → NameTooLong（空名报"过长"是 Python 缺陷，复刻）
/// 2) 含 '/' '\\\\' '\\0' → BadOsName
/// 3) 非 Latin-1（存在无效 UTF-8 字节，或有效字符 > U+00FF）→ NameNonAscii
///
/// Python 的字符数 = 有效 UTF-8 字符数 + 无效字节数（surrogateescape 每字节
/// 一字符），此处逐段解码等价计算；三条非法字符均为 ASCII，字节域判断等价。
fn validate_os_name(bytes: &[u8]) -> Result<(), PakError> {
    let mut nchars = 0usize;
    let mut invalid = false;
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                nchars += s.chars().count();
                break;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                let bad_len = e.error_len().unwrap_or(rest.len() - valid);
                nchars += std::str::from_utf8(&rest[..valid]).unwrap().chars().count();
                nchars += bad_len;
                invalid = true;
                rest = &rest[valid + bad_len..];
            }
        }
    }
    if bytes.is_empty() || nchars > MAX_NAME {
        return Err(PakError::NameTooLong(py_os_repr_inner(bytes)));
    }
    if bytes.contains(&b'/') || bytes.contains(&b'\\') || bytes.contains(&0) {
        return Err(PakError::BadOsName(py_os_repr_inner(bytes)));
    }
    if invalid || !std::str::from_utf8(bytes).unwrap().chars().all(|c| c as u32 <= 0xFF) {
        return Err(PakError::NameNonAscii(py_os_repr_inner(bytes)));
    }
    Ok(())
}

/// OS 名 → str（build_dir 的 rel 拼接用）。无效 UTF-8 → NameNonAscii
/// （Python 中该名必在 latin1 编码时同点报错）。
fn os_name_to_str(nm: &OsStr) -> Result<String, PakError> {
    match nm.to_str() {
        Some(s) => Ok(s.to_string()),
        None => Err(PakError::NameNonAscii(py_os_repr_inner(&os_bytes(nm)))),
    }
}

/// Latin-1 域内的 str → pak 名字节（每字符 1 字节；调用前已通过校验）。
fn latin1_encode(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u8).collect()
}

// ---------------------------------------------------------------------------
// 写侧树
// ---------------------------------------------------------------------------

/// 文件数据来源。
enum FileData {
    /// 新内容驻内存（pack 全部文件 / replace 的目标文件）。
    Inline(Vec<u8>),
    /// 从源 pak 流式拷贝（replace 的非目标文件）：源 pak 内定位。
    FromPak { offset: u64, size: u64 },
}

enum WritePayload {
    Dir(Vec<WriteNode>),
    File(FileData),
}

/// 写侧树节点（offset 为 serialize 递推赋值的规划值，u64 防回绕）。
struct WriteNode {
    /// 最终 attrib（pack 生成；replace 克隆原值或目标改写）
    attrib: u32,
    /// stored 字节数（目录 = 64×children.len()；文件 = blob/src_size）
    size: u64,
    /// 原始字节数（目录 = 64×children.len()；文件 = raw 长度）
    org: u64,
    /// Unix 时间戳（pack 由 manifest 恢复；replace 保留原值）
    mtime: u32,
    /// pak 内名字（Latin-1 字节）
    name: Vec<u8>,
    /// 数据区 offset（serialize 第一遍赋值）
    offset: u64,
    payload: WritePayload,
}

/// 节点序列化后的总字节数（对齐 Python _node_size：
/// 目录 = 子目录表 + 全部子孙数据；文件 = stored size）。
fn node_size(e: &WriteNode) -> u64 {
    match &e.payload {
        WritePayload::Dir(children) => {
            CAT_SIZE as u64 * children.len() as u64 + children.iter().map(node_size).sum::<u64>()
        }
        WritePayload::File(_) => e.size,
    }
}

/// 全树总字节数（不含 16 字节头；= serialize 完成时的最终 running - HDR_SIZE）。
fn tree_total(nodes: &[WriteNode]) -> u64 {
    CAT_SIZE as u64 * nodes.len() as u64 + nodes.iter().map(node_size).sum::<u64>()
}

/// 树中文件节点数（对齐 Python nfiles 统计）。
fn count_files(nodes: &[WriteNode]) -> usize {
    nodes
        .iter()
        .map(|e| match &e.payload {
            WritePayload::Dir(children) => count_files(children),
            WritePayload::File(_) => 1,
        })
        .sum()
}

/// 写出前的树校验（对齐 Python write_pak 开头 flatten 全树 _encode_name）：
/// 名字字节域校验 + size/org 的 u32 域检查（Python 超限时 struct.error 裸崩溃，
/// Rust 良性收紧为报错；消息沿用 design 2.5.4 措辞）。
fn check_tree(nodes: &[WriteNode]) -> Result<(), PakError> {
    for e in nodes {
        CatalogEntry::validate_name(&e.name)?;
        if e.size > u32::MAX as u64 || e.org > u32::MAX as u64 {
            return Err(PakError::Manifest("写出 pak 失败: 超过 4GB 上限".into()));
        }
        if let WritePayload::Dir(children) = &e.payload {
            check_tree(children)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// preflight / collect_files（对齐 _preflight / _collect_files）
// ---------------------------------------------------------------------------

/// 预检（对齐 Python `_preflight` + os.walk 语义）：
/// 每层按「目录（read_dir 序）+ 文件（read_dir 序）」校验名字并检测同层大小写
/// 冲突；`_pak_manifest.json` 名跳过校验与冲突登记，但其为**目录**时仍递归内容
/// （对齐 os.walk 无条件递归，与 _collect_files/_build_dir 的跳过形成怪癖差异）。
fn preflight(dir: &Path, prefix: &str) -> Result<(), PakError> {
    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = os_name_to_str(&entry.file_name())?;
        let full = entry.path();
        // os.path.isdir 语义：stat 失败（断链 symlink 等）按非目录处理
        let is_dir = std::fs::metadata(&full).map(|m| m.is_dir()).unwrap_or(false);
        if is_dir {
            dirs.push((name, full));
        } else {
            files.push(name);
        }
    }
    // 名字校验 + 大小写冲突（dirs+files 拼接序；消息中后到者在前、先见者在后）
    let mut lower: HashMap<String, String> = HashMap::new();
    for name in dirs.iter().map(|(n, _)| n).chain(files.iter()) {
        if name == MANIFEST_NAME {
            continue;
        }
        validate_os_name(name.as_bytes())?;
        let lk = name.to_lowercase();
        if let Some(first) = lower.get(&lk) {
            return Err(PakError::NameCaseConflict(
                format!("{prefix}{name}"),
                format!("{prefix}{first}"),
            ));
        }
        lower.insert(lk, name.clone());
    }
    // 递归子目录（含名为 _pak_manifest.json 的目录 —— os.walk 无条件递归）
    for (name, full) in &dirs {
        preflight(full, &format!("{prefix}{name}/"))?;
    }
    Ok(())
}

/// 收集待打包文件（对齐 Python `_collect_files`）：每层 read_dir →
/// 按名字原始字节序排序（= Python sorted(key=utf-8-surrogateescape 编码)）→
/// 依序目录递归展开 / 文件入列表（跳过 _pak_manifest.json，含目录形态）。
/// 返回顺序决定压缩进度顺序与错误报告顺序。
fn collect_files(dir: &Path) -> Result<Vec<PathBuf>, PakError> {
    let mut items: Vec<(Vec<u8>, PathBuf, bool)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name() == MANIFEST_NAME {
            continue;
        }
        let bytes = os_bytes(&entry.file_name());
        let full = entry.path();
        let is_dir = std::fs::metadata(&full).map(|m| m.is_dir()).unwrap_or(false);
        items.push((bytes, full, is_dir));
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut files = Vec::new();
    for (_, full, is_dir) in items {
        if is_dir {
            files.extend(collect_files(&full)?);
        } else {
            files.push(full);
        }
    }
    Ok(files)
}

// ---------------------------------------------------------------------------
// preload_files（对齐 _preload_files：并行压缩 + 进度输出）
// ---------------------------------------------------------------------------

/// 预处理产物：blob 驻内存（RAW 原文或压缩流）+ attrib + 原始长度。
struct Prepared {
    blob: Vec<u8>,
    attrib: u32,
    org: u64,
}

/// 单文件处理（= Python _load_raw / _load_compressed）。
fn load_one(path: &Path, compress: bool) -> Result<Prepared, io::Error> {
    let raw = std::fs::read(path)?;
    if compress {
        let org = raw.len() as u64;
        Ok(Prepared {
            blob: lzss::compress(&raw),
            attrib: 0,
            org,
        })
    } else {
        let org = raw.len() as u64;
        Ok(Prepared {
            blob: raw,
            attrib: RAW_BIT,
            org,
        })
    }
}

/// 预读/预压缩（对齐 Python `_preload_files`）：
/// * compress=false → 顺序读文件（无进度输出）
/// * compress=true 且 jobs>1 且文件数>1 → rayon 专用池并行压缩；
///   进度经 channel 回传主线程按文件序重放（等价 Python ex.map 的有序消费，
///   消除多线程完成时序交错，stderr 行序确定）
/// * 其余 → 顺序迭代
///
/// 进度（仅压缩模式）：`# 压缩进度 {done}/{total}`，done%500==0 或 done==total。
fn preload_files(
    files: &[PathBuf],
    compress: bool,
    jobs: usize,
    err: &mut (impl Write + Send),
) -> Result<HashMap<PathBuf, Prepared>, PakError> {
    let total = files.len();
    let mut out: HashMap<PathBuf, Prepared> = HashMap::with_capacity(total);
    if !compress {
        for path in files {
            let p = load_one(path, false)?;
            out.insert(path.clone(), p);
        }
        return Ok(out);
    }
    /// 进度行（仅压缩模式）：`# 压缩进度 {done}/{total}`，done%500==0 或 done==total。
    fn progress(done: usize, total: usize, err: &mut impl Write) -> Result<(), PakError> {
        if done % 500 == 0 || done == total {
            writeln!(err, "# 压缩进度 {done}/{total}")?;
        }
        Ok(())
    }
    // 并行分支（Python: compress and jobs > 1 and total > 1；池构建失败回退顺序）
    if jobs > 1 && total > 1 {
        if let Ok(pool) = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.min(8))
            .build()
        {
            let (tx, rx) = std::sync::mpsc::channel::<(usize, Result<Prepared, io::Error>)>();
            // Receiver 非 Sync，Mutex 包裹以满足 scope 闭包的 Send 约束
            //（消费循环仅在调用线程持锁，无竞争）
            let rx = std::sync::Mutex::new(rx);
            pool.scope(|s| -> Result<(), PakError> {
                for (i, path) in files.iter().enumerate() {
                    let tx = tx.clone();
                    s.spawn(move |_| {
                        let _ = tx.send((i, load_one(path, true)));
                    });
                }
                drop(tx);
                // 主线程按文件序消费（乱序完成者暂存 pending）
                let rx = rx.lock().unwrap(); // 持锁期间无 panic，不会中毒
                let mut pending: HashMap<usize, Result<Prepared, io::Error>> = HashMap::new();
                let mut next = 0usize;
                while let Ok((i, r)) = rx.recv() {
                    pending.insert(i, r);
                    while let Some(r) = pending.remove(&next) {
                        let done = next + 1;
                        next = done;
                        progress(done, total, err)?;
                        match r {
                            Ok(p) => {
                                out.insert(files[done - 1].clone(), p);
                            }
                            Err(e) => return Err(PakError::Io(e)),
                        }
                    }
                }
                Ok(())
            })?;
            return Ok(out);
        }
    }
    // 顺序分支（对齐 Python 单线程路径，含 total==1 / jobs==1 / 池构建失败回退）
    for (i, path) in files.iter().enumerate() {
        let p = load_one(path, true)?;
        out.insert(path.clone(), p);
        progress(i + 1, total, err)?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// build_dir（对齐 _build_dir：树构建与 order 保序）
// ---------------------------------------------------------------------------

/// 递归扫描目录构建写侧树（对齐 Python `_build_dir`）：
/// 每层 read_dir 原始序收集（跳过 _pak_manifest.json，含目录形态）后按
/// `(order.get(rel, 1<<30), 名字原始字节序)` 排序；目录 meta：attrib=DIR_BIT、
/// size=org=64×children.len()、mtime=0；文件 meta 来自 prepared，mtime=0。
fn build_dir(
    dir: &Path,
    prepared: &mut HashMap<PathBuf, Prepared>,
    order: &HashMap<String, usize>,
    prefix: &str,
) -> Result<Vec<WriteNode>, PakError> {
    struct Item {
        name: Vec<u8>,
        name_str: String,
        full: PathBuf,
        is_dir: bool,
    }
    let mut items = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name() == MANIFEST_NAME {
            continue;
        }
        let name_str = os_name_to_str(&entry.file_name())?;
        let full = entry.path();
        let is_dir = std::fs::metadata(&full).map(|m| m.is_dir()).unwrap_or(false);
        items.push(Item {
            name: os_bytes(&entry.file_name()),
            name_str,
            full,
            is_dir,
        });
    }
    // 排序键：(order 索引或 1<<30, 名字原始字节)；名字唯一保证全序确定
    items.sort_by(|a, b| {
        let ka = (
            order
                .get(&format!("{prefix}{}", a.name_str))
                .copied()
                .unwrap_or(1 << 30),
            &a.name,
        );
        let kb = (
            order
                .get(&format!("{prefix}{}", b.name_str))
                .copied()
                .unwrap_or(1 << 30),
            &b.name,
        );
        ka.cmp(&kb)
    });
    let mut nodes = Vec::with_capacity(items.len());
    for item in items {
        if item.is_dir {
            let children = build_dir(
                &item.full,
                prepared,
                order,
                &format!("{prefix}{}/", item.name_str),
            )?;
            let sz = CAT_SIZE as u64 * children.len() as u64;
            nodes.push(WriteNode {
                attrib: DIR_BIT,
                size: sz,
                org: sz,
                mtime: 0,
                name: latin1_encode(&item.name_str),
                offset: 0,
                payload: WritePayload::Dir(children),
            });
        } else {
            let p = prepared
                .remove(&item.full)
                .ok_or_else(|| PakError::Manifest(format!("内部错误：预压缩数据缺失: {}", item.full.display())))?;
            let org = p.org;
            let attrib = p.attrib;
            nodes.push(WriteNode {
                attrib,
                size: p.blob.len() as u64,
                org,
                mtime: 0,
                name: latin1_encode(&item.name_str),
                offset: 0,
                payload: WritePayload::File(FileData::Inline(p.blob)),
            });
        }
    }
    Ok(nodes)
}

// ---------------------------------------------------------------------------
// manifest 消费（对齐 _load_manifest / order 提取 / _apply_manifest_mtime）
// ---------------------------------------------------------------------------

/// pack 专用 manifest 读入（宽松对齐 Python `_load_manifest`）：
/// * 文件缺失 / JSON 语法错误 → None（静默）
/// * files 任一条目值非 object → None（Python 此时在 _apply_manifest_mtime
///   AttributeError 裸崩溃，Rust 良性收紧为整份忽略，design 2.10.2 #4）
/// * order 非 array / 元素非 str → 该元素不参与（Python 崩溃或无效果）
fn load_manifest_value(indir: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(indir.join(MANIFEST_NAME)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    if let Some(files) = v.get("files") {
        let m = files.as_object()?;
        for (_, item) in m {
            if !item.is_object() {
                return None;
            }
        }
    }
    Some(v)
}

/// mtime 恢复（对齐 Python `_apply_manifest_mtime`）：
/// 树序遍历，文件 rel 命中 files 时取 mtime 值校验后写入节点；
/// 非法 → `manifest 中 {rel} 的 mtime 非法: {py_repr}`。
/// Rust 拒绝 Bool（Python 接受 True=1，design 2.11 R11 良性收紧）。
fn apply_manifest_mtime(v: &Value, nodes: &mut [WriteNode]) -> Result<(), PakError> {
    let Some(files) = v.get("files").and_then(Value::as_object) else {
        return Ok(());
    };
    rec_mtime(nodes, files, "")
}

fn rec_mtime(
    nodes: &mut [WriteNode],
    files: &Map<String, Value>,
    prefix: &str,
) -> Result<(), PakError> {
    for e in nodes {
        let rel = format!("{prefix}{}", latin1(&e.name));
        match &mut e.payload {
            WritePayload::Dir(children) => rec_mtime(children, files, &format!("{rel}/"))?,
            WritePayload::File(_) => {
                if let Some(item) = files.get(&rel) {
                    // 缺失按 0（Python .get('mtime', 0)），合法域 [0, u32::MAX]
                    let mt = item.get("mtime");
                    let val = match mt {
                        None => 0u32,
                        Some(m) => match m.as_u64() {
                            Some(u) if u <= u32::MAX as u64 => u as u32,
                            _ => {
                                return Err(PakError::Manifest(format!(
                                    "manifest 中 {rel} 的 mtime 非法: {}",
                                    py_repr(m)
                                )))
                            }
                        },
                    };
                    e.mtime = val;
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// write_pak（对齐 write_pak / _serialize：递推布局 + .tmp 原子写）
// ---------------------------------------------------------------------------

/// 深度优先序列化（对齐 Python `_serialize`）：
/// 第一遍递推 offset（空目录 node_size=0 不推进 → 相邻空目录共享 offset，
/// 自产 pak 读回 TreeCycle，怪癖复刻）；第二遍写目录表块；第三遍依序写数据
/// （顺序流，无 seek——递归落点即当前位置）。
/// FromPak 拷贝短读 → DataOob（Python 静默残缺，良性收紧，design 2.10.2 #5）。
fn serialize<W: Write>(
    w: &mut W,
    nodes: &mut [WriteNode],
    base: u64,
    src: &mut Option<&mut File>,
) -> Result<(), PakError> {
    let mut running = base + CAT_SIZE as u64 * nodes.len() as u64;
    for e in nodes.iter_mut() {
        e.offset = running;
        running += node_size(e);
    }
    for e in nodes.iter() {
        let entry = CatalogEntry {
            attrib: e.attrib,
            offset: e.offset as u32,
            size: e.size as u32,
            org: e.org as u32,
            mtime: e.mtime,
            name: e.name.clone(),
        };
        w.write_all(&entry.to_bytes()?)?;
    }
    for e in nodes.iter_mut() {
        let offset = e.offset;
        match &mut e.payload {
            WritePayload::Dir(children) => serialize(w, children, offset, src)?,
            WritePayload::File(FileData::Inline(blob)) => w.write_all(blob)?,
            WritePayload::File(FileData::FromPak {
                offset: src_off,
                size: src_size,
            }) => {
                let src = src.as_deref_mut().ok_or_else(|| {
                    PakError::Manifest("写出 pak 失败: 内部错误：树含 FromPak 节点但无源 pak".into())
                })?;
                src.seek(SeekFrom::Start(*src_off))?;
                let copied = io::copy(&mut src.take(*src_size), w)?;
                if copied != *src_size {
                    return Err(PakError::DataOob(latin1(&e.name)));
                }
            }
        }
    }
    Ok(())
}

/// 原子写出 pak（对齐 Python `write_pak`）：
/// 名字/域校验 → 总长与 root_size 的 u32 域检查 → 写 `{out}.tmp` → rename。
/// 任一 IO 失败：删除 .tmp（忽略删除错误）→ `写出 pak 失败: {OSError形态}`。
/// src_path：FromPak 节点的数据来源（replace 场景；pack 为 None）。
fn write_pak(
    root: &mut [WriteNode],
    out_path: &Path,
    src_path: Option<&Path>,
) -> Result<(), PakError> {
    check_tree(root)?;
    if HDR_SIZE as u64 + tree_total(root) > u32::MAX as u64 {
        return Err(PakError::Manifest("写出 pak 失败: 超过 4GB 上限".into()));
    }
    let root_size = CAT_SIZE as u64 * root.len() as u64;
    if root_size > u32::MAX as u64 {
        return Err(PakError::Manifest("写出 pak 失败: 超过 4GB 上限".into()));
    }
    // tmp = "{out}.tmp"（字符串拼接语义，非 set_extension）
    let mut tmp_os = out_path.as_os_str().to_os_string();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    let result = write_to_tmp(root, &tmp, out_path, src_path, root_size as u32);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp); // FR-38：失败不残留 .tmp
        return Err(e);
    }
    Ok(())
}

/// 写 tmp + rename 的全部步骤（错误已在各阶段包装为最终消息形态）。
fn write_to_tmp(
    root: &mut [WriteNode],
    tmp: &Path,
    out_path: &Path,
    src_path: Option<&Path>,
    root_size: u32,
) -> Result<(), PakError> {
    let f = File::create(tmp).map_err(|e| {
        PakError::Manifest(format!("写出 pak 失败: {}", py_os_error(&e, tmp)))
    })?;
    let mut w = BufWriter::with_capacity(1 << 20, f);
    w.write_all(
        &Header {
            magic: MAGIC,
            root_dir_size: root_size,
            unused1: 0,
            unused2: 0,
        }
        .to_bytes(),
    )
    .map_err(write_io_err)?;
    let mut src = match src_path {
        Some(p) => Some(
            File::open(p).map_err(|e| {
                PakError::Manifest(format!("写出 pak 失败: {}", py_os_error(&e, p)))
            })?,
        ),
        None => None,
    };
    serialize(&mut w, root, HDR_SIZE as u64, &mut src.as_mut()).map_err(|e| match e {
        // serialize 内部 IO 错误统一包为写出失败（DataOob 保留原变体）
        PakError::Io(io) => PakError::Manifest(format!(
            "写出 pak 失败: {}",
            crate::commands_read::py_errno_msg(&io)
        )),
        other => other,
    })?;
    w.flush().map_err(write_io_err)?;
    drop(w);
    std::fs::rename(tmp, out_path).map_err(|e| {
        PakError::Manifest(format!("写出 pak 失败: {}", py_oserror2(&e, tmp, out_path)))
    })?;
    Ok(())
}

/// 写出阶段 IO 错误（write/flush，无 filename）的统一包装。
fn write_io_err(e: io::Error) -> PakError {
    PakError::Manifest(format!(
        "写出 pak 失败: {}",
        crate::commands_read::py_errno_msg(&e)
    ))
}

// ---------------------------------------------------------------------------
// cmd_pack
// ---------------------------------------------------------------------------

/// 打包（FR-30~38；步骤顺序对齐 Python cmd_pack）。
pub fn cmd_pack(
    indir: &Path,
    pak_path: &Path,
    compress: bool,
    jobs: usize,
    err: &mut (impl Write + Send),
) -> Result<(), PakError> {
    // 1) indir 非目录 → NotDir（os.path.isdir 语义：stat 失败按非目录）
    let is_dir = std::fs::metadata(indir).map(|m| m.is_dir()).unwrap_or(false);
    if !is_dir {
        return Err(PakError::NotDir(indir.display().to_string()));
    }
    // 2) manifest（原始 Value：mtime 语义校验延后到 apply）
    let mf = load_manifest_value(indir);
    // 3) order 映射（重复 rel 后者覆盖，对齐 Python dict 赋值）
    let order: HashMap<String, usize> = mf
        .as_ref()
        .and_then(|v| v.get("order"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .enumerate()
                .filter_map(|(i, x)| x.as_str().map(|s| (s.to_string(), i)))
                .collect()
        })
        .unwrap_or_default();
    // 4~5) 收集 + 预检（Python 顺序：collect 在前）
    let files = collect_files(indir)?;
    preflight(indir, "")?;
    // 6) 预读/预压缩
    let mut prepared = preload_files(&files, compress, jobs, err)?;
    // 7) 树构建
    let mut root = build_dir(indir, &mut prepared, &order, "")?;
    // 8) mtime 恢复
    if let Some(v) = &mf {
        apply_manifest_mtime(v, &mut root)?;
    }
    // 9) 写出
    write_pak(&mut root, pak_path, None)?;
    // 10) 汇总
    let nfiles = count_files(&root);
    writeln!(err, "# 打包完成: {nfiles} 个文件 -> {}", pak_path.display())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_replace
// ---------------------------------------------------------------------------

/// 克隆读侧树为写侧树（对齐 Python clone：元数据级拷贝；
/// 文件节点剥数据记源位置，目录递归）。
fn clone_tree(nodes: &[TreeNode]) -> Vec<WriteNode> {
    nodes
        .iter()
        .map(|n| {
            let payload = match &n.payload {
                NodePayload::Dir(children) => WritePayload::Dir(clone_tree(children)),
                NodePayload::File { offset, size, .. } => WritePayload::File(FileData::FromPak {
                    offset: *offset as u64,
                    size: *size as u64,
                }),
            };
            WriteNode {
                attrib: n.entry.attrib,
                size: n.entry.size as u64,
                org: n.entry.org as u64,
                mtime: n.entry.mtime,
                name: n.entry.name.clone(),
                offset: 0,
                payload,
            }
        })
        .collect()
}

/// 在写侧树中按 inner 分段定位目标节点（对齐 Python cmd_replace 的逐段下钻；
/// find 已成功则理论必达，失败报 `路径解析失败`）。
fn locate<'a>(nodes: &'a mut [WriteNode], parts: &[&str]) -> Option<&'a mut WriteNode> {
    let mut cur: &mut [WriteNode] = nodes;
    for (k, p) in parts.iter().enumerate() {
        let n = cur.iter_mut().find(|e| latin1(&e.name) == *p)?;
        if k + 1 == parts.len() {
            return Some(n);
        }
        cur = match &mut n.payload {
            WritePayload::Dir(children) => children,
            _ => return None,
        };
    }
    None
}

/// 替换（FR-39~46，NFR-04/10/11；步骤对齐 Python cmd_replace）。
pub fn cmd_replace(
    pak_path: &Path,
    inner: &str,
    newfile: &Path,
    compress: bool,
    backup: bool,
    err: &mut (impl Write + Send),
) -> Result<(), PakError> {
    let reader = open_pak(pak_path)?;
    let inner_norm = inner.trim_matches('/').replace('\\', "/");
    let Some(node) = reader.find(&inner_norm) else {
        return Err(PakError::NotExists(inner_norm));
    };
    if node.is_dir() {
        return Err(PakError::IsDir(inner_norm));
    }
    // 新文件内容（Python open+read；io 失败 → main 的 OSError 分支
    // `错误: [Errno N] ...: 'path'`，Rust 以 py_os_error 对齐同格式）
    let raw = std::fs::read(newfile).map_err(|e| py_os_error(&e, newfile))?;
    let raw_len = raw.len() as u64;
    let (blob, attrib) = if compress {
        (lzss::compress(&raw), 0u32)
    } else {
        (raw, RAW_BIT)
    };
    // 克隆树 + 定位目标 + 改写（mtime 保留原值）
    let mut new_root = clone_tree(&reader.root);
    let parts: Vec<&str> = inner_norm.split('/').collect();
    let stored_len = blob.len() as u64;
    let tgt = locate(&mut new_root, &parts).ok_or_else(|| {
        PakError::Manifest(format!("路径解析失败: {inner_norm}"))
    })?;
    tgt.attrib = attrib;
    tgt.size = stored_len;
    tgt.org = raw_len;
    tgt.payload = WritePayload::File(FileData::Inline(blob));
    // 备份（对齐 shutil.copy2：内容 + 权限 + mtime）
    if backup {
        let mut bak = pak_path.as_os_str().to_os_string();
        bak.push(".bak");
        let bak = PathBuf::from(bak);
        std::fs::copy(pak_path, &bak)?;
        copy_mtime(pak_path, &bak)?;
        writeln!(err, "# 已备份: {}", bak.display())?;
    }
    drop(reader); // 释放源 pak 句柄（Linux 下 rename 覆盖已打开文件虽可行，对齐 Python 先关）
    write_pak(&mut new_root, pak_path, Some(pak_path))?;
    writeln!(
        err,
        "# 已替换 {inner_norm}: {raw_len} 字节 (存储 {stored_len} 字节)"
    )?;
    Ok(())
}

/// 补齐 fs::copy 不复制的 mtime（shutil.copy2 = copyfile + copystat）。
fn copy_mtime(from: &Path, to: &Path) -> Result<(), PakError> {
    let mtime = std::fs::metadata(from)?.modified()?;
    let f = File::options().write(true).open(to)?;
    f.set_times(std::fs::FileTimes::new().set_modified(mtime))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 单元测试：显示格式锚点 + 校验规则域
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_repr_anchors() {
        assert_eq!(py_str_repr("abc"), "'abc'");
        assert_eq!(py_str_repr("it's"), "\"it's\""); // 含 ' 不含 " → 双引号
        assert_eq!(py_str_repr("a\"b"), "'a\"b'"); // 含 " 不含 ' → 单引号，" 原样
        assert_eq!(py_str_repr("a'b\"c"), "'a\\'b\"c'"); // 两者皆含 → 转义 '
        assert_eq!(py_str_repr("back\\slash"), "'back\\\\slash'");
        assert_eq!(py_str_repr("nl\nx"), "'nl\\nx'");
        assert_eq!(py_str_repr("ctl\x01"), "'ctl\\x01'");
        assert_eq!(py_str_repr("caf\u{e9}"), "'caf\u{e9}'"); // Latin-1 高位原样
        // JSON 值域
        assert_eq!(py_repr(&Value::Null), "None");
        assert_eq!(py_repr(&Value::Bool(true)), "True");
        assert_eq!(py_repr(&Value::Bool(false)), "False");
        assert_eq!(py_repr(&serde_json::json!(1.5)), "1.5");
        assert_eq!(py_repr(&serde_json::json!(-1)), "-1");
        assert_eq!(py_repr(&serde_json::json!(4294967296u64)), "4294967296");
        assert_eq!(py_repr(&serde_json::json!("x")), "'x'");
        assert_eq!(py_repr(&serde_json::json!([1, "a"])), "[1, 'a']");
        assert_eq!(py_repr(&serde_json::json!({"a": 1})), "{'a': 1}");
    }

    #[test]
    fn py_os_repr_inner_surrogate() {
        // 无效字节 \xff → \udcff（Python surrogateescape 后 repr 的形态）
        assert_eq!(py_os_repr_inner(b"a\xff"), "a\\udcff");
        assert_eq!(py_os_repr_inner(b"\xff\xff"), "\\udcff\\udcff");
        // 有效 UTF-8 的 é（C3 A9）按字符原样
        assert_eq!(py_os_repr_inner("café".as_bytes()), "caf\u{e9}");
    }

    #[test]
    fn validate_os_name_rules() {
        // 合法：普通名 / 恰 43 字符 / Latin-1 高位字符名（UTF-8 字节但字符 ≤ U+00FF）
        assert!(validate_os_name(b"abc").is_ok());
        assert!(validate_os_name(&[b'x'; 43]).is_ok());
        assert!(validate_os_name("caf\u{e9}.list".as_bytes()).is_ok());
        // 顺序 1：空名/超长（44 字符）→ NameTooLong；空名同样报"过长"（Python 缺陷）
        assert!(matches!(
            validate_os_name(b""),
            Err(PakError::NameTooLong(_))
        ));
        assert!(matches!(
            validate_os_name(&[b'x'; 44]),
            Err(PakError::NameTooLong(_))
        ));
        // CJK 名 8 字符不超长 → 落到非 ASCII（顺序：长度先于 latin1）
        assert!(matches!(
            validate_os_name("太长.repeat".as_bytes()),
            Err(PakError::NameNonAscii(_))
        ));
        // 顺序 2：非法字符（\ 合法 UTF-8）
        assert!(matches!(
            validate_os_name(b"a\\b"),
            Err(PakError::BadOsName(_))
        ));
        // 无效 UTF-8 字节（Python surrogate → latin1 必失败）→ NameNonAscii
        assert!(matches!(
            validate_os_name(b"a\xff"),
            Err(PakError::NameNonAscii(_))
        ));
        // 44 个无效字节：字符数（=字节数）超限 → NameTooLong 先于 NameNonAscii
        assert!(matches!(
            validate_os_name(&[0xff; 44]),
            Err(PakError::NameTooLong(_))
        ));
    }

    #[test]
    fn py_oserror_format() {
        let e = io::Error::from_raw_os_error(21);

        assert_eq!(
            py_oserror2(&e, Path::new("a.tmp"), Path::new("a")),
            "[Errno 21] Is a directory: 'a.tmp' -> 'a'"
        );
    }

    /// 空目录怪癖：相邻空目录共享 offset（node_size=0 不推进 running）。
    #[test]
    fn empty_dirs_share_offset() {
        let mk_dir = |name: &str| WriteNode {
            attrib: DIR_BIT,
            size: 0,
            org: 0,
            mtime: 0,
            name: name.as_bytes().to_vec(),
            offset: 0,
            payload: WritePayload::Dir(vec![]),
        };
        let mut root = vec![mk_dir("d1"), mk_dir("d2")];
        serialize(&mut Vec::new(), &mut root, 16, &mut None).unwrap();
        // 根表 2×64 后：d1 与 d2 的 offset 相同（Python 怪癖复刻）
        assert_eq!(root[0].offset, 16 + 128);
        assert_eq!(root[1].offset, 16 + 128);
        assert_eq!(tree_total(&root), 128);
    }

    /// mtime 校验值域（R11：Rust 拒绝 Bool；域外/浮点/字符串/None 报非法）。
    #[test]
    fn mtime_value_domain() {
        let mut nodes = [WriteNode {
            attrib: RAW_BIT,
            size: 1,
            org: 1,
            mtime: 0,
            name: b"a.txt".to_vec(),
            offset: 0,
            payload: WritePayload::File(FileData::Inline(b"x".to_vec())),
        }];
        let mf = |mt: Value| {
            serde_json::json!({"files": {"a.txt": {"mtime": mt}}})
        };
        // 合法：整数域内（含边界）
        for ok in [0u64, 123, 4294967295] {
            apply_manifest_mtime(&mf(serde_json::json!(ok)), &mut nodes).unwrap();
            assert_eq!(nodes[0].mtime, ok as u32);
        }
        // mtime 键缺失 → 0
        apply_manifest_mtime(
            &serde_json::json!({"files": {"a.txt": {}}}),
            &mut nodes,
        )
        .unwrap();
        assert_eq!(nodes[0].mtime, 0);
        // 非法：浮点 / 字符串 / None / 负数 / 超域 / Bool（R11 收紧）
        for (bad, msg) in [
            (serde_json::json!(1.5), "1.5"),
            (serde_json::json!("x"), "'x'"),
            (serde_json::json!(null), "None"),
            (serde_json::json!(-1), "-1"),
            (serde_json::json!(4294967296u64), "4294967296"),
            (serde_json::json!(true), "True"),
            (serde_json::json!([1]), "[1]"),
        ] {
            let e = apply_manifest_mtime(&mf(bad.clone()), &mut nodes).unwrap_err();
            assert_eq!(
                e.to_string(),
                format!("manifest 中 a.txt 的 mtime 非法: {msg}"),
                "mtime={bad}"
            );
        }
        // rel 未命中 files → mtime 不动
        nodes[0].mtime = 7;
        apply_manifest_mtime(&serde_json::json!({"files": {"other": {"mtime": 9}}}), &mut nodes)
            .unwrap();
        assert_eq!(nodes[0].mtime, 7);
    }

    /// files 条目值非 object → 整份 manifest 忽略（design 2.10.2 #4）。
    #[test]
    fn manifest_malformed_files_ignored() {
        let dir = std::env::temp_dir().join("wd_pak_write_mf");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // files 值为 int：Python AttributeError 裸崩，Rust 整份忽略
        std::fs::write(dir.join(MANIFEST_NAME), br#"{"files": {"a": 5}, "order": ["a"]}"#)
            .unwrap();
        assert!(load_manifest_value(&dir).is_none());
        // 合法结构 → Some
        std::fs::write(
            dir.join(MANIFEST_NAME),
            br#"{"files": {"a": {"mtime": 1}}, "order": ["a"]}"#,
        )
        .unwrap();
        assert!(load_manifest_value(&dir).is_some());
        // 坏 JSON / 缺失 → None
        std::fs::write(dir.join(MANIFEST_NAME), b"{oops").unwrap();
        assert!(load_manifest_value(&dir).is_none());
        std::fs::remove_file(dir.join(MANIFEST_NAME)).unwrap();
        assert!(load_manifest_value(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}