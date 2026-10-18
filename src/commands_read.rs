//! 读路径命令 —— list / find / extract / cat / check，逐行对齐 Python cmd_*。
//!
//! 输出去向（对齐 Python）：check 的报告全部走 stdout（含问题列表与"校验通过"）；
//! 其他命令的统计/进度/警告走 stderr，条目名走 stdout（cat 无 -o 时二进制透明）。
//! check 发现问题返回 Ok(true)（main 映射 exit 1）；PakError 由 main 映射 exit 2。
//!
//! I/O 错误消息转 Python OSError 风格（`[Errno {n}] {strerror}: '{path}'`），
//! 与 Python main 的 `except OSError` 分支输出一致。

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::PakError;
use crate::manifest::{FileStat, Manifest, MANIFEST_NAME};
use crate::pak::PakReader;

/// writeln! 包装：写 stdout/stderr 失败转 Python print 的 OSError 消息风格
/// （无 filename，如 broken pipe 时的 `[Errno 32] Broken pipe`）。
macro_rules! wln {
    ($dst:expr, $($arg:tt)*) => {
        writeln!($dst, $($arg)*).map_err(write_err_to_py)
    };
}

/// 列出内容（Python cmd_list）。
pub fn cmd_list(
    pak_path: &Path,
    long: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<(), PakError> {
    let pak = open_pak(pak_path)?;
    let (mut files, mut dirs) = (0usize, 0usize);
    for (rel, node) in pak.walk() {
        let isdir = node.is_dir();
        if isdir {
            dirs += 1;
        } else {
            files += 1;
        }
        if long {
            // 'DIR ' 带尾空格是 Python 原文（DIR 行因此有两个空格，FILE 行一个）
            let e = &node.entry;
            wln!(
                out,
                "{} {rel:<60} attrib=0x{:08x} off=0x{:08x} size={} org={} mt={}",
                if isdir { "DIR " } else { "FILE" },
                e.attrib,
                e.offset,
                e.size,
                e.org,
                e.mtime
            )?;
        } else {
            wln!(out, "{rel}{}", if isdir { "/" } else { "" })?;
        }
    }
    wln!(
        err,
        "# {files} files, {dirs} dirs, rootDirSize={}",
        pak.root_size
    )?;
    Ok(())
}

/// 按关键字搜索文件名（Python cmd_find）：大小写不敏感子串，匹配项含目录。
pub fn cmd_find(
    pak_path: &Path,
    keyword: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<(), PakError> {
    let pak = open_pak(pak_path)?;
    let kw = keyword.to_lowercase();
    let mut hits = 0usize;
    for (rel, node) in pak.walk() {
        if rel.to_lowercase().contains(&kw) {
            hits += 1;
            wln!(out, "{rel}{}", if node.is_dir() { "/" } else { "" })?;
        }
    }
    wln!(err, "# 匹配 {hits} 项")?;
    Ok(())
}

/// 解包到目录（Python cmd_extract）：含 manifest 生成与大小写冲突警告。
pub fn cmd_extract(pak_path: &Path, outdir: &Path, err: &mut impl Write) -> Result<(), PakError> {
    let pak = open_pak(pak_path)?;
    create_dir(outdir)?;
    let mut manifest = Manifest {
        pak: pak_basename(pak_path),
        files: Vec::new(),
        order: Vec::new(),
    };
    let mut low: HashMap<String, String> = HashMap::new();
    for (rel, node) in pak.walk() {
        manifest.order.push(rel.clone());
        let target = outdir.join(&rel);
        if node.is_dir() {
            create_dir(&target)?;
            continue;
        }
        // 大小写冲突警告（仅警告不中断，后到者覆盖 lower 表，对齐 Python）
        let lk = rel.to_lowercase();
        if let Some(prev) = low.get(&lk) {
            if prev != &rel {
                wln!(err, "# 警告: 大小写冲突 {rel} / {prev}")?;
            }
        }
        low.insert(lk, rel.clone());
        if let Some(parent) = target.parent() {
            create_dir(parent)?;
        }
        let data = pak.read_file(node)?;
        write_file(&target, &data)?;
        manifest.files.push((
            rel,
            FileStat {
                attrib: node.entry.attrib,
                mtime: node.entry.mtime,
                stored: node.entry.size as u64,
                orig: node.entry.org as u64,
            },
        ));
    }
    let mf_bytes = manifest.to_bytes()?;
    write_file(&outdir.join(MANIFEST_NAME), &mf_bytes)?;
    wln!(
        err,
        "# 解包完成: {} 个文件 -> {}",
        manifest.files.len(),
        outdir.display()
    )?;
    Ok(())
}

/// 导出单个文件（Python cmd_cat）：默认二进制写 stdout，-o 落盘并回报绝对路径。
pub fn cmd_cat(
    pak_path: &Path,
    inner: &str,
    out_file: Option<&Path>,
    err: &mut impl Write,
) -> Result<(), PakError> {
    let pak = open_pak(pak_path)?;
    let node = pak
        .find(inner)
        // 注意：cat 的不存在消息是 `不存在: {inner}`；
        // PakError::NotExists（`包内不存在: ...`）是 Python cmd_replace 专用消息
        .ok_or_else(|| PakError::Manifest(format!("不存在: {inner}")))?;
    if node.is_dir() {
        return Err(PakError::IsDir(inner.to_string()));
    }
    let data = pak.read_file(node)?;
    match out_file {
        Some(path) => {
            write_file(path, &data)?;
            wln!(
                err,
                "# 已导出 {} 字节 -> {}",
                data.len(),
                abspath(path).display()
            )?;
        }
        None => {
            // 二进制透明写 stdout（对齐 sys.stdout.buffer.write）；不走控制台宽字符
            // 转换，否则 Windows 上会把二进制当文本转码。
            // BufWriter 避开行缓冲的逐行 flush
            let mut w = std::io::BufWriter::new(crate::platform::stdout_raw());
            w.write_all(&data).map_err(write_err_to_py)?;
            w.flush().map_err(write_err_to_py)?;
        }
    }
    Ok(())
}

/// 校验 pak（Python cmd_check）。报告全走 stdout；发现问题返回 Ok(true)（exit 1）。
pub fn cmd_check(pak_path: &Path, deep: bool, out: &mut impl Write) -> Result<bool, PakError> {
    let pak = open_pak(pak_path)?;
    let mut nfiles = 0usize;
    let mut ndirs = 0usize;
    let mut attribs: BTreeMap<u32, usize> = BTreeMap::new();
    let mut problems: Vec<String> = Vec::new();
    let (mut total_stored, mut total_org) = (0u64, 0u64);
    for (rel, node) in pak.walk() {
        let e = &node.entry;
        if node.is_dir() {
            ndirs += 1;
            continue;
        }
        nfiles += 1;
        *attribs.entry(e.attrib).or_insert(0) += 1;
        total_stored += e.size as u64;
        total_org += e.org as u64;
        if e.offset as u64 + e.size as u64 > pak.file_size {
            problems.push(format!("越界: {rel} off=0x{:x} size={}", e.offset, e.size));
        }
        if deep {
            match pak.read_file(node) {
                Ok(data) => {
                    // read_file 内部已做长度校验，此分支与 Python 同为不可达保险
                    if data.len() as u64 != e.org as u64 {
                        problems.push(format!(
                            "解压大小不符: {rel} {} != {}",
                            data.len(),
                            e.org
                        ));
                    }
                }
                Err(ex) => problems.push(format!("解压失败: {rel}: {ex}")),
            }
        }
    }
    wln!(out, "# {}", pak_path.display())?;
    wln!(
        out,
        "# files={nfiles} dirs={ndirs} rootDirSize={} fileSize={}",
        pak.root_size,
        pak.file_size
    )?;
    let ratio = total_stored as f64 / total_org.max(1) as f64;
    wln!(out, "# stored={total_stored} org={total_org} ratio={ratio:.3}")?;
    let dist = attribs
        .iter()
        .map(|(a, c)| format!("0x{a:08x}x{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    wln!(out, "# attrib 分布: {dist}")?;
    if !problems.is_empty() {
        wln!(out, "# 问题 {} 个:", problems.len())?;
        for p in problems.iter().take(50) {
            wln!(out, "  {p}")?;
        }
        return Ok(true);
    }
    wln!(out, "# 校验通过{}", if deep { " (含全量解压)" } else { "" })?;
    Ok(false)
}

// ---------------------------------------------------------------------------
// I/O 辅助：错误消息对齐 Python OSError 风格
// ---------------------------------------------------------------------------

/// 打开 pak；io 失败改写为带路径的 Python OSError 消息（如 `[Errno 2] ...: 'x'`）。
pub(crate) fn open_pak(pak_path: &Path) -> Result<PakReader, PakError> {
    PakReader::open(pak_path).map_err(|e| io_error_to_py(e, pak_path))
}

/// 仅转换 io 类错误（PakError 其余变体消息本就与 Python 一致，原样透传）。
fn io_error_to_py(e: PakError, path: &Path) -> PakError {
    match &e {
        PakError::Io(io) if io.raw_os_error().is_some() => py_os_error(io, path),
        _ => e,
    }
}

/// Python OSError 风格消息：Unix 为 `[Errno {n}] {strerror}: {repr(path)}`，
/// Windows 为 `[Win32 {n}] {固定文案}: {repr(path)}`（见 crate::platform）。
/// filename 按 Python repr 规则包裹（默认单引号；含 ' 不含 " 时双引号）——
/// 交互模式对拍实测：路径含 `'` 时 Python 输出 `"/non'exist.pak"`。
pub(crate) fn py_os_error(e: &std::io::Error, path: &Path) -> PakError {
    PakError::Os(format!(
        "{}: {}",
        crate::platform::io_message(e),
        crate::commands_write::py_str_repr(&path.display().to_string())
    ))
}

/// 写 stdout/stderr 失败转 Python OSError 消息（print 无 filename，
/// 如 broken pipe 时的 `[Errno 32] Broken pipe`）。
fn write_err_to_py(e: std::io::Error) -> PakError {
    PakError::Os(crate::platform::io_message(&e))
}

/// 创建目录（对齐 os.makedirs(exist_ok=True)；路径是已存在文件时同样报 FileExists）。
pub(crate) fn create_dir(path: &Path) -> Result<(), PakError> {
    if path.as_os_str().is_empty() {
        // 对齐 Python os.makedirs('') 的 FileNotFoundError（Rust 对空路径静默成功）
        return Err(PakError::Manifest(
            "[Errno 2] No such file or directory: ''".into(),
        ));
    }
    std::fs::create_dir_all(path).map_err(|e| py_os_error(&e, path))
}

/// 写文件（对齐 open(path,'wb')+write；io 失败转 Python OSError 消息）。
pub(crate) fn write_file(path: &Path, data: &[u8]) -> Result<(), PakError> {
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(data))
        .map_err(|e| py_os_error(&e, path))
}

// ---------------------------------------------------------------------------
// 路径辅助：对齐 os.path.abspath / normpath（不解析符号链接、不要求存在）
// ---------------------------------------------------------------------------

/// os.path.basename 等价（pak 文件名入 manifest 的 pak 字段）。
fn pak_basename(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 对齐 os.path.abspath = normpath(join(cwd, p))。
fn abspath(p: &Path) -> PathBuf {
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(p)
    };
    crate::platform::normpath(&joined)
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// abspath：绝对路径仅规范化，相对路径基于 cwd（不解析符号链接）。
    /// normpath 本身的用例见 crate::platform（按平台各自断言）。
    #[test]
    fn abspath_basic() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(abspath(Path::new("x/y")), cwd.join("x/y"));
        #[cfg(unix)]
        {
            assert_eq!(abspath(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
            // trailing slash 折叠
            assert_eq!(abspath(Path::new("/tmp/a/")), PathBuf::from("/tmp/a"));
        }
        #[cfg(windows)]
        {
            // 盘符必须保留：丢盘符会让 cat -o 打印出不可用的路径
            assert_eq!(abspath(Path::new(r"C:\a\b\..\c")), PathBuf::from(r"C:\a\c"));
            assert_eq!(abspath(Path::new("C:/a/./b/")), PathBuf::from(r"C:\a\b"));
            assert!(abspath(Path::new("x/y")).is_absolute());
        }
    }

    /// OSError 风格转换（文案按平台：Unix errno+strerror，Windows 固定 Win32 文案）
    #[test]
    fn py_os_error_format() {
        let e = std::io::Error::from_raw_os_error(2);
        #[cfg(unix)]
        let expect = "[Errno 2] No such file or directory: '/nonexist.pak'";
        #[cfg(windows)]
        let expect = "[Win32 2] 系统找不到指定的文件: '/nonexist.pak'";
        assert_eq!(py_os_error(&e, Path::new("/nonexist.pak")).to_string(), expect);
    }
}