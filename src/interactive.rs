//! 交互模式 —— 逐字节复刻 Python wd_pak_tool.py 的 `_ask` + `interactive`
//! （无子命令运行时的 REPL）。
//!
//! 与 CLI 模式的三处行为差异（实测 Python 确认）：
//! 1. 错误输出走 **stdout**（CLI 走 stderr 且 exit 2）；
//! 2. PakError → `错误: {e}`，OSError 等其他异常 → `出错: {e}`
//!    （Rust 以 PakError::Os / PakError::Io 变体对应 Python 的 OSError 域；
//!    写 pak 失败等被 Python 包为 PakError 的场景仍归 `错误:`）；
//! 3. check 发现问题时 Python `sys.exit(1)` 的 SystemExit 继承 BaseException、
//!    穿透 except Exception，交互进程直接以 exit 1 终止（不打印 `已退出`）。
//!
//! 命令逻辑 100% 复用 commands_read / commands_write，不重复实现；
//! cmd_* 的统计/进度行仍走 err 参数（= stderr），与 CLI 一致。

use std::io::Write;
use std::path::Path;

use crate::platform;

use crate::commands_read as cr;
use crate::commands_write as cw;
use crate::error::PakError;

/// 交互模式输出（对齐 Python print 走 stdout）。文案含中文，Windows 上由
/// platform::stdout 转宽字符直写控制台，不受代码页影响；写失败忽略
/// （Python 同场景抛 BrokenPipeError，这里不引入 panic）。
macro_rules! say {
    ($($arg:tt)*) => {{
        let mut out = crate::platform::stdout();
        let _ = writeln!(out, $($arg)*);
    }};
}

/// 主菜单（Python 三引号字符串原文；print 追加换行 → 首尾均有空行）
const MENU: &str = "\n 1) 查看包内文件列表        2) 解包到目录\n 3) 从目录打包              4) 替换包内单个文件\n 5) 导出单个文件            6) 校验 pak\n 7) 搜索文件名              0) 退出";

/// Python `_ask`：input(prompt).strip()，EOF → ''；再依次剥两端双引号、单引号
/// （顺序敏感：`"a'"` → strip('"') → `a'` → strip("'") → `a`）。
/// prompt 无换行写出并立即 flush（对齐 input 的 prompt 特性）。
fn ask(prompt: &str) -> String {
    loop {
        {
            let mut out = platform::stdout();
            let _ = out.write_all(prompt.as_bytes());
            let _ = out.flush();
        }
        match platform::read_line() {
            // EOF 对齐 EOFError → ''
            Ok(None) => return String::new(),
            Ok(Some(line)) => {
                return line.trim().trim_matches('"').trim_matches('\'').to_string()
            }
            // 编码问题（如输入被重定向且不是 UTF-8）：提示后重新询问，
            // 不再像旧实现那样把 Err 当 EOF 直接退出程序
            Err(msg) => {
                let mut err = platform::stderr();
                let _ = writeln!(err, "{msg}");
            }
        }
    }
}

/// 并行压缩线程数（对齐 Python 交互模式：min(8, os.cpu_count() or 1)，不问用户）。
fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8)
}

/// os.path.basename 等价（入参已将 '\\' 换 '/'）：取最后一段；尾随 '/' 或空 → ''。
fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or("")
}

/// 交互模式入口（Python `interactive`）。
pub fn interactive() {
    say!("问道 PAK 工具 v1.0 —— 交互模式");
    loop {
        say!("{MENU}");
        let choice = ask("请选择: ");
        if matches!(choice.as_str(), "0" | "q" | "exit" | "quit" | "") {
            say!("已退出");
            return;
        }
        if let Err(e) = dispatch(&choice) {
            match &e {
                PakError::Os(_) | PakError::Io(_) => say!("出错: {e}"),
                _ => say!("错误: {e}"),
            }
        }
    }
}

/// 单次选择分发（命令错误原样上抛，由主循环统一打印）。
fn dispatch(choice: &str) -> Result<(), PakError> {
    match choice {
        "1" => {
            let pak = ask("pak 路径: ");
            let long = ask("显示详细信息? y/N: ").to_lowercase() == "y";
            cr::cmd_list(Path::new(&pak), long, &mut platform::stdout(), &mut platform::stderr())
        }
        "2" => {
            let pak = ask("pak 路径: ");
            let outdir = ask("输出目录: ");
            cr::cmd_extract(Path::new(&pak), Path::new(&outdir), &mut platform::stderr())
        }
        "3" => {
            let indir = ask("输入目录: ");
            let pak = ask("输出 pak 路径: ");
            let compress = ask("不压缩(更快)? y/N: ").to_lowercase() != "y";
            cw::cmd_pack(
                Path::new(&indir),
                Path::new(&pak),
                compress,
                default_jobs(),
                &mut platform::stderr(),
            )
        }
        "4" => {
            let pak = ask("pak 路径: ");
            let inner = ask("包内路径(如 100heis.list): ");
            let newfile = ask("新文件路径: ");
            let compress = ask("不压缩(更快)? y/N: ").to_lowercase() != "y";
            // backup 恒 True（Python 交互模式不询问）
            cw::cmd_replace(
                Path::new(&pak),
                &inner,
                Path::new(&newfile),
                compress,
                true,
                &mut platform::stderr(),
            )
        }
        "5" => {
            let pak = ask("pak 路径: ");
            let inner = ask("包内路径: ");
            // Python：`_ask(...) or basename(inner.replace('\\','/'))`；
            // 结果为空串时 cmd_cat 的 `if out:` 为假 → 二进制写 stdout
            let out = {
                let raw = ask("保存到文件(回车=同名): ");
                if raw.is_empty() {
                    basename(&inner.replace('\\', "/")).to_string()
                } else {
                    raw
                }
            };
            let out_opt = if out.is_empty() { None } else { Some(Path::new(&out)) };
            cr::cmd_cat(Path::new(&pak), &inner, out_opt, &mut platform::stderr())
        }
        "6" => {
            let pak = ask("pak 路径: ");
            let deep = ask("全量解压校验(慢)? y/N: ").to_lowercase() == "y";
            match cr::cmd_check(Path::new(&pak), deep, &mut platform::stdout()) {
                // Python cmd_check 末尾 sys.exit(1)：SystemExit 穿透交互层直接终止
                Ok(true) => std::process::exit(1),
                r => r.map(|_| ()),
            }
        }
        "7" => {
            let pak = ask("pak 路径: ");
            let keyword = ask("关键字: ");
            cr::cmd_find(Path::new(&pak), &keyword, &mut platform::stdout(), &mut platform::stderr())
        }
        _ => {
            say!("无效选择，请重试");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// 单元测试：_ask 后处理 / basename / jobs 域
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// 引号剥离顺序（对齐 Python .strip('"').strip("'")）
    #[test]
    fn quote_strip_order() {
        let f = |s: &str| s.trim().trim_matches('"').trim_matches('\'').to_string();
        assert_eq!(f("  x  "), "x"); // 先剥空白
        assert_eq!(f("\"pak.pak\""), "pak.pak"); // 双引号
        assert_eq!(f("'pak.pak'"), "pak.pak"); // 单引号
        assert_eq!(f("\"a'\""), "a"); // 先双后单：`"a'"` → `a'` → `a`
        assert_eq!(f("\"\"x\"\""), "x"); // 多层同引号
        assert_eq!(f("\"'\""), ""); // `"'"` → `'` → 空
        assert_eq!(f("a\"b"), "a\"b"); // 内部引号不动
    }

    /// basename（inner 已反斜杠换正斜杠后的语义）
    #[test]
    fn basename_semantics() {
        assert_eq!(basename("100heis.list"), "100heis.list");
        assert_eq!(basename("dir/100heis.list"), "100heis.list");
        assert_eq!(basename("dir/sub/a.bin"), "a.bin");
        assert_eq!(basename("dir/"), "");
        assert_eq!(basename(""), "");
    }

    /// jobs 域对齐 min(8, cpu or 1)
    #[test]
    fn jobs_domain() {
        let j = default_jobs();
        assert!((1..=8).contains(&j), "jobs={j}");
    }
}