//! 平台隔离层：所有与操作系统相关的 I/O 行为收口于此。
//!
//! 业务模块只依赖本模块导出的函数与类型，不在 commands_* / interactive / main
//! 里出现 `cfg(windows)`：
//!
//! - [`io_message`]：IO 错误 → 消息（Unix `[Errno n] strerror`；Windows `[Win32 n] 固定文案`）
//! - [`normpath`]：路径规范化（Windows 保留盘符与 UNC 前缀，Unix 行为不变）
//! - [`stdout`] / [`stderr`]：文本输出（Windows 直写控制台宽字符，无代码页副作用）
//! - [`stdout_raw`]：二进制输出（cat 无 `-o` 时原样写字节，不做编码转换）
//! - [`read_line`]：交互输入（Windows 控制台走 `ReadConsoleW`，不受代码页影响）
//! - [`is_sharing_violation`]：判断「目标文件被其他进程占用」
//!
//! Windows 侧不使用 `SetConsoleOutputCP`：控制台代码页是**控制台级**状态，
//! 改了会影响调用方的 cmd/PowerShell 会话，异常退出时还无法恢复。改用
//! `WriteConsoleW`/`ReadConsoleW` 直写宽字符，等价于 Python 3 的做法。

#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub use unix::{io_message, is_sharing_violation, normpath, read_line, stderr, stdout, stdout_raw};
#[cfg(not(windows))]
pub use unix::{Stderr, Stdout, StdoutRaw};

#[cfg(windows)]
pub use windows::{
    io_message, is_sharing_violation, normpath, read_line, stderr, stdout, stdout_raw, Stderr,
    Stdout, StdoutRaw,
};

/// 用户配置目录：Windows 用 `%APPDATA%\wd-pak`，Unix 用 XDG 配置目录。
pub fn app_config_dir() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        if let Some(base) = std::env::var_os("APPDATA") {
            return std::path::PathBuf::from(base).join("wd-pak");
        }
        if let Some(base) = std::env::var_os("USERPROFILE") {
            return std::path::PathBuf::from(base).join(".wd-pak");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
            return std::path::PathBuf::from(base).join("wd-pak");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home).join(".config").join("wd-pak");
        }
    }
    std::path::PathBuf::from(".wd-pak")
}

/// 去掉行尾 `\r\n` / `\n`（两个平台共用的输入收尾处理）
/// 路径身份键：normalize + Windows 大小写折叠，用于去重/串行判定。
/// 业务模块一律走此函数，不直接写 `#[cfg(windows)]`。
pub fn path_key(path: &std::path::Path) -> String {
    let normalized = normpath(path);
    #[cfg(windows)]
    {
        normalized.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized.to_string_lossy().into_owned()
    }
}

pub(crate) fn trim_line_end(mut s: String) -> String {
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    s
}
