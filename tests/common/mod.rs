//! 跨平台对拍支持：Python 调用、文本归一化、临时目录与外部夹具探测。
//!
//! 原则：
//! - Python 参考实现走管道时统一 UTF-8（`PYTHONIOENCODING` / `PYTHONUTF8`），
//!   消除 Windows 代码页（GBK）差异；
//! - 文本流在 Windows 上归一化 CRLF 与 OS 错误前缀（Rust `[Win32 n] 固定文案`
//!   与 Python `[Errno n]` / `[WinError n]` 是设计允许的差异）；Unix 保持逐字节严格；
//! - PAK、LZSS、导出文件等二进制产物始终逐字节比较，不做任何归一化；
//! - `_pak_manifest.json` 是唯一由实现生成的文本产物：Rust 固定 LF，Python 在
//!   Windows 文本模式下写 CRLF，测试只对该文件做换行归一化；
//! - 外部夹具缺失时显式 skip；设置 `WDPAK_REQUIRE_FIXTURES=1` 可将其变为失败。

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Python 解释器探测结果缓存（避免每次调用都 spawn `--version`）。
static PYTHON: OnceLock<String> = OnceLock::new();

/// Python 解释器：`WDPAK_PYTHON` 覆盖，否则依次尝试 `python3` / `python`。
pub fn python_program() -> &'static str {
    PYTHON.get_or_init(|| {
        if let Ok(p) = std::env::var("WDPAK_PYTHON") {
            if !p.trim().is_empty() {
                return p;
            }
        }
        for candidate in ["python3", "python"] {
            if Command::new(candidate)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return candidate.to_string();
            }
        }
        "python3".to_string()
    })
}

/// 参考实现脚本：`WDPAK_REFERENCE` 覆盖，默认仓库上级的 `wd_pak_tool.py`。
pub fn reference_script() -> PathBuf {
    std::env::var_os("WDPAK_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("wd_pak_tool.py")
        })
}

/// 构造 Python 基准命令：固定管道编码为 UTF-8，保证与 Rust 的 UTF-8 输出可比。
pub fn python_command() -> Command {
    let mut cmd = Command::new(python_program());
    cmd.arg(reference_script());
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd
}

/// 测试临时目录：`WDPAK_TEST_TMP` 覆盖；Windows 用系统 temp，Unix 用 `/tmp`。
pub fn temp_root() -> PathBuf {
    let root = match std::env::var_os("WDPAK_TEST_TMP") {
        Some(p) => PathBuf::from(p),
        None => {
            #[cfg(windows)]
            {
                std::env::temp_dir().join("wd-pak-tests")
            }
            #[cfg(not(windows))]
            {
                PathBuf::from("/tmp/wdpak-tests")
            }
        }
    };
    std::fs::create_dir_all(&root).expect("创建测试临时目录失败");
    root
}

pub fn temp_path(rel: impl AsRef<Path>) -> PathBuf {
    temp_root().join(rel)
}

pub fn temp_str(rel: impl AsRef<Path>) -> String {
    temp_path(rel).to_string_lossy().into_owned()
}

/// 文本流归一化：UTF-8 解码；仅 Windows 归一化 CRLF 与 OS 错误前缀。
/// Unix 上保持原样，维持逐字节严格比较。
pub fn normalize_text(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes).into_owned();
    #[cfg(windows)]
    let s = normalize_win_os_error(&s.replace("\r\n", "\n"));
    s
}

/// Windows 上把 `[Win32 n] 固定文案`、`[Errno n] ...`、`[WinError n] ...`
/// 统一成 `[OS]`，保留其后的 `: 路径` 或 `: from -> to` 参数。
///
/// 这是测试层的平台差异归一化，不改变产品输出：Rust 的 Win32 固定文案与
/// Python 的 errno/WinError 文案本就不可能逐字相同（AGENTS.md 明确属预期）。
#[cfg(windows)]
fn normalize_win_os_error(s: &str) -> String {
    const PREFIXES: [&str; 3] = ["[Win32 ", "[WinError ", "[Errno "];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('[') {
        let after = &rest[start..];
        if !PREFIXES.iter().any(|p| after.starts_with(p)) {
            out.push_str(&rest[..=start]);
            rest = &rest[start + 1..];
            continue;
        }
        out.push_str(&rest[..start]);
        let Some(end) = after.find(']') else {
            out.push_str(after);
            return out;
        };
        out.push_str("[OS]");
        let tail = &after[end + 1..];
        match tail.find(": ") {
            Some(colon) => rest = &tail[colon..],
            None => rest = "",
        }
    }
    out.push_str(rest);
    out
}

/// `_pak_manifest.json` 比较用：Windows 上去掉 Python 文本模式产生的 CRLF，
/// 其余字节保持严格；Unix 原样返回。
pub fn normalize_manifest(bytes: &[u8]) -> Vec<u8> {
    #[cfg(windows)]
    {
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                i += 1;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        out
    }
    #[cfg(not(windows))]
    {
        bytes.to_vec()
    }
}

/// 外部夹具缺失时返回 true 并打印 skip；`WDPAK_REQUIRE_FIXTURES=1` 时改为 panic。
pub fn skip_without_fixture(path: &Path, what: &str) -> bool {
    if path.is_file() {
        return false;
    }
    let msg = format!("skip: 缺少外部夹具 {what}: {}", path.display());
    if std::env::var_os("WDPAK_REQUIRE_FIXTURES").is_some() {
        panic!("{msg}");
    }
    eprintln!("{msg}");
    true
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn win_os_error_prefix_is_canonicalized() {
        assert_eq!(
            normalize_win_os_error("错误: [Win32 2] 系统找不到指定的文件: 'a.pak'"),
            "错误: [OS]: 'a.pak'"
        );
        assert_eq!(
            normalize_win_os_error("错误: [Errno 2] No such file or directory: 'a.pak'"),
            "错误: [OS]: 'a.pak'"
        );
        assert_eq!(
            normalize_win_os_error("错误: [WinError 5] 拒绝访问。: 'a.tmp' -> 'a.pak'"),
            "错误: [OS]: 'a.tmp' -> 'a.pak'"
        );
    }
}
