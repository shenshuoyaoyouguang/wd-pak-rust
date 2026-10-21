//! Windows 实现：错误码固定文案、盘符感知的 normpath、控制台宽字符 I/O。
//!
//! 设计要点（见 docs/windows-plan.md 2.2）：
//! - 错误消息用**固定中文文案**，不读系统 FormatMessage，避免随系统语言漂移；
//! - 输出/输入一律走 `WriteConsoleW` / `ReadConsoleW`，**不改控制台代码页**，
//!   因此不会污染调用方的 cmd/PowerShell 会话；
//! - 仅当句柄确实是控制台时才做宽字符转换，重定向到文件/管道时保持原始字节。

use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT, INVALID_FILE_ATTRIBUTES,
};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, ReadConsoleW, WriteConsoleW, STD_ERROR_HANDLE,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

use super::trim_line_end;

/// 文本 stdout：控制台走宽字符，重定向走原始字节。
/// 句柄以 `usize` 保存而非 `HANDLE`：`commands_write` 的输出参数要求 `Send`，
/// 裸指针不满足；句柄值本身跨线程使用是安全的（`std::io::Stdout` 亦为 `Send`）。
pub struct Stdout {
    inner: io::Stdout,
    console: Option<usize>,
}

/// 文本 stderr
pub struct Stderr {
    inner: io::Stderr,
    console: Option<usize>,
}

/// 二进制 stdout（cat 无 `-o`）：不做任何编码转换
pub struct StdoutRaw {
    inner: io::Stdout,
}

pub fn stdout() -> Stdout {
    Stdout {
        inner: io::stdout(),
        console: console_handle(STD_OUTPUT_HANDLE),
    }
}

pub fn stderr() -> Stderr {
    Stderr {
        inner: io::stderr(),
        console: console_handle(STD_ERROR_HANDLE),
    }
}

pub fn stdout_raw() -> StdoutRaw {
    StdoutRaw { inner: io::stdout() }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.console {
            Some(h) => {
                write_console(h, buf)?;
                Ok(buf.len())
            }
            None => self.inner.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.console {
            Some(h) => {
                write_console(h, buf)?;
                Ok(buf.len())
            }
            None => self.inner.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Write for StdoutRaw {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 句柄是控制台时返回其 handle（`GetConsoleMode` 对文件/管道会失败）
fn console_handle(kind: u32) -> Option<usize> {
    let h = unsafe { GetStdHandle(kind) };
    let mut mode = 0u32;
    if unsafe { GetConsoleMode(h, &mut mode) } != 0 {
        Some(h as usize)
    } else {
        None
    }
}

/// UTF-8 字节 → 宽字符直写控制台（`WriteConsoleW` 可能部分写入，需循环）
fn write_console(handle: usize, buf: &[u8]) -> io::Result<()> {
    let h = handle as HANDLE;
    let text = String::from_utf8_lossy(buf);
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut start = 0usize;
    while start < wide.len() {
        let mut written = 0u32;
        let ok = unsafe {
            WriteConsoleW(
                h,
                wide[start..].as_ptr(),
                (wide.len() - start) as u32,
                &mut written,
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            break;
        }
        start += written as usize;
    }
    Ok(())
}

/// Win32 错误码 → 固定中文文案（不随系统语言变化）。
/// 未收录的码回退为系统文本（此时才可能带本地化），并在消息中保留码值。
pub fn io_message(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => match win32_text(code) {
            Some(text) => format!("[Win32 {code}] {text}"),
            None => format!("[Win32 {code}] {}", strip_os_suffix(e)),
        },
        None => e.to_string(),
    }
}

fn win32_text(code: i32) -> Option<&'static str> {
    Some(match code {
        2 => "系统找不到指定的文件",
        3 => "系统找不到指定的路径",
        5 => "拒绝访问",
        13 => "数据无效",
        15 => "系统找不到指定的驱动器",
        32 => "另一个程序正在使用此文件，进程无法访问",
        33 => "另一个程序已锁定文件的一部分，进程无法访问",
        112 => "磁盘空间不足",
        123 => "文件名、目录名或卷标语法不正确",
        145 => "目录不是空的",
        183 => "当文件已存在时，无法创建该文件",
        206 => "文件名或扩展名太长",
        _ => return None,
    })
}

/// 去掉 io::Error Display 的 " (os error N)" 尾巴
fn strip_os_suffix(e: &io::Error) -> String {
    let disp = e.to_string();
    match e.raw_os_error() {
        Some(code) => {
            let suffix = format!(" (os error {code})");
            disp.strip_suffix(&suffix).unwrap_or(&disp).to_string()
        }
        None => disp,
    }
}

/// Windows 上「目标被占用」表现为共享冲突（32/33）；拒绝访问（5）在目标
/// 已存在时同样按「被占用或不可写」处理（只读文件也走 5）。
pub fn is_sharing_violation(e: &io::Error, target: &Path) -> bool {
    match e.raw_os_error() {
        Some(32) | Some(33) => true,
        Some(5) => target.exists(),
        _ => false,
    }
}

/// 路径是否为 Windows 重解析点（符号链接、目录 junction、云占位符等）。
/// `GetFileAttributesW` 返回链接本身的属性，不跟随目标；查询失败按非重解析点处理。
pub fn is_reparse_point(path: &Path) -> bool {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let attrs = unsafe { GetFileAttributesW(wide.as_ptr()) };
    attrs != INVALID_FILE_ATTRIBUTES && attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// 对齐 ntpath.normpath：保留盘符/UNC 前缀，折叠 '.'/'..' 与重复分隔符。
///
/// 绝对路径上多余的 '..' 被丢弃（`C:\a\..\..\b` → `C:\b`），与 ntpath 一致。
pub fn normpath(p: &Path) -> PathBuf {
    let mut prefix: Option<std::ffi::OsString> = None;
    let mut has_root = false;
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for c in p.components() {
        match c {
            Component::Prefix(pr) => prefix = Some(pr.as_os_str().to_os_string()),
            Component::RootDir => {
                has_root = true;
                parts.clear();
            }
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(last) if last != ".." => {
                    parts.pop();
                }
                // 盘符相对路径（`C:a\..`）或绝对路径上，多余 '..' 直接丢弃
                _ if prefix.is_none() && !has_root => parts.push("..".into()),
                _ => {}
            },
            Component::Normal(s) => parts.push(s.to_os_string()),
        }
    }
    let mut out = PathBuf::new();
    if let Some(pr) = &prefix {
        out.push(pr);
    }
    if has_root {
        out.push("\\");
    }
    if parts.is_empty() {
        if prefix.is_none() && !has_root {
            out.push(".");
        }
    } else {
        for part in parts {
            out.push(part);
        }
    }
    out
}

/// 读一行（不含行尾）。控制台走 `ReadConsoleW`（UTF-16 直接解码，与代码页无关）；
/// 输入被重定向时按 UTF-8 解析，非法字节返回提示。EOF → `Ok(None)`。
pub fn read_line() -> Result<Option<String>, &'static str> {
    let h = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let mut mode = 0u32;
    if unsafe { GetConsoleMode(h, &mut mode) } != 0 {
        read_console_line(h)
    } else {
        read_piped_line()
    }
}

fn read_console_line(h: HANDLE) -> Result<Option<String>, &'static str> {
    let mut buf = vec![0u16; 1024];
    let mut acc: Vec<u16> = Vec::new();
    loop {
        let mut read = 0u32;
        let ok = unsafe {
            ReadConsoleW(
                h,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut read,
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err("读取控制台输入失败");
        }
        if read == 0 {
            return Ok(None); // Ctrl+Z / EOF
        }
        acc.extend_from_slice(&buf[..read as usize]);
        if acc.contains(&u16::from(b'\n')) {
            break;
        }
        if acc.len() > (1 << 20) {
            return Err("输入过长");
        }
    }
    Ok(Some(trim_line_end(String::from_utf16_lossy(&acc))))
}

fn read_piped_line() -> Result<Option<String>, &'static str> {
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
    fn win32_table_covers_common_codes() {
        assert_eq!(
            io_message(&io::Error::from_raw_os_error(2)),
            "[Win32 2] 系统找不到指定的文件"
        );
        assert_eq!(
            io_message(&io::Error::from_raw_os_error(32)),
            "[Win32 32] 另一个程序正在使用此文件，进程无法访问"
        );
        // 未收录码走回退路径，仍带码值
        let msg = io_message(&io::Error::from_raw_os_error(9999));
        assert!(msg.starts_with("[Win32 9999] "), "{msg}");
    }

    #[test]
    fn normpath_keeps_drive_prefix() {
        let cases = [
            (r"C:\a\b\..\c", r"C:\a\c"),
            (r"C:/a/./b", r"C:\a\b"),
            (r"C:\a\..\..\b", r"C:\b"),
            (r"\\server\share\a\..\b", r"\\server\share\b"),
            (r"a\..\..", r".."),
            ("", "."),
        ];
        for (input, expect) in cases {
            assert_eq!(normpath(Path::new(input)), Path::new(expect), "normpath({input:?})");
        }
    }

    #[test]
    fn trim_line_end_strips_crlf() {
        assert_eq!(trim_line_end("a\r\n".into()), "a");
        assert_eq!(trim_line_end("a\n".into()), "a");
        assert_eq!(trim_line_end("a".into()), "a");
    }
}
