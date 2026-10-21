//! 非 Windows 实现：与引入平台层之前的行为逐字节一致。
//!
//! 这些实现是 `commands_read` 原 `py_errno_msg` / `normpath` 的等比平移，
//! 任何改动都会破坏与 Python 参考实现的输出对拍，需 ubuntu CI 全绿才算通过。

use std::io;
use std::path::{Component, Path, PathBuf};

use super::trim_line_end;

/// 文本 stdout（Unix 直接写字节，UTF-8 终端下即正确显示）
pub type Stdout = io::Stdout;
/// 文本 stderr
pub type Stderr = io::Stderr;
/// 二进制 stdout（cat 无 `-o`）
pub type StdoutRaw = io::Stdout;

pub fn stdout() -> Stdout {
    io::stdout()
}

pub fn stderr() -> Stderr {
    io::stderr()
}

pub fn stdout_raw() -> StdoutRaw {
    io::stdout()
}

/// Python OSError 消息的 errno+strerror 部分：`[Errno {n}] {strerror}`。
/// strerror 取自 Rust io::Error（与 C 库一致），去掉 " (os error N)" 尾巴。
pub fn io_message(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => {
            let disp = e.to_string();
            let suffix = format!(" (os error {code})");
            let msg = disp.strip_suffix(&suffix).unwrap_or(&disp);
            format!("[Errno {code}] {msg}")
        }
        None => e.to_string(),
    }
}

/// POSIX 上不存在「文件被其他进程占用」这一 errno（Linux 允许 rename 覆盖
/// 已打开文件），故恒为 false。
pub fn is_sharing_violation(_e: &io::Error, _target: &Path) -> bool {
    false
}

/// POSIX 无 Windows 重解析点概念，恒为 false。
pub fn is_reparse_point(_path: &Path) -> bool {
    false
}

/// 对齐 os.path.normpath：折叠 '.'/'..'/重复斜杠，保留前导 '..'，空结果回 '.'。
pub fn normpath(p: &Path) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    let mut has_root = false;
    for c in p.components() {
        match c {
            Component::RootDir => {
                has_root = true;
                parts.clear();
            }
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(last) if last != ".." => {
                    parts.pop();
                }
                _ if !has_root => parts.push("..".into()),
                _ => {}
            },
            Component::Normal(s) => parts.push(s.to_os_string()),
            Component::Prefix(_) => {}
        }
    }
    let mut out = PathBuf::new();
    if has_root {
        out.push("/");
    }
    if parts.is_empty() {
        if !has_root {
            out.push(".");
        }
    } else {
        for part in parts {
            out.push(part);
        }
    }
    out
}

/// 读一行（不含行尾）。EOF → `Ok(None)`；非法 UTF-8 → `Err(提示)`。
///
/// 返回 `None` 与旧实现 `read_line(...).unwrap_or(0) == 0` 的 EOF 语义一致；
/// 读错误同样按 EOF 处理（保持与调整前相同的行为）。
pub fn read_line() -> Result<Option<String>, &'static str> {
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(trim_line_end(line))),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            Err("输入不是合法 UTF-8，请确认终端编码为 UTF-8")
        }
        Err(_) => Ok(None),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normpath_matches_python() {
        let cases = [
            ("/a/b/../c", "/a/c"),
            ("a/..", "."),
            ("a/../..", ".."),
            ("../a", "../a"),
            ("a//b/./c", "a/b/c"),
            ("/..", "/"),
            ("..", ".."),
            ("", "."),
            ("./a", "a"),
        ];
        for (input, expect) in cases {
            assert_eq!(normpath(Path::new(input)), Path::new(expect), "normpath({input:?})");
        }
    }

    #[test]
    fn io_message_matches_python_oserror() {
        let e = io::Error::from_raw_os_error(2);
        assert_eq!(io_message(&e), "[Errno 2] No such file or directory");
    }

    #[test]
    fn trim_line_end_strips_crlf() {
        assert_eq!(trim_line_end("a\r\n".into()), "a");
        assert_eq!(trim_line_end("a\n".into()), "a");
        assert_eq!(trim_line_end("a".into()), "a");
    }
}
