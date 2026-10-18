//! CLI 入口 —— 参数定义与 Python 版 wd_pak_tool.py argparse 完全一致（兼容性硬约束）。
//! 业务模块位于库 crate wd_pak（模块划分见 src/lib.rs），
//! 本文件只保留参数解析、子命令分发与顶层错误处理。
//!
//! --version 手动处理（disable_version_flag）：Python argparse action='version'
//! 输出纯版本号 `1.0`，而 clap 自动 flag 会输出 `wd-pak 1.0`。

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use wd_pak::error::PakError;

/// 问道服务端 PAK 打包/解包工具
#[derive(Parser)]
#[command(name = "wd-pak", version = "1.0", about = "问道服务端 PAK 打包/解包工具", disable_version_flag = true)]
struct Cli {
    /// 显示版本号
    #[arg(long)]
    version: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

/// 子命令定义（对齐 Python argparse：list/find/extract/pack/replace/cat/check）
#[derive(Subcommand)]
enum Command {
    /// 列出 pak 内容
    List {
        pak: PathBuf,
        /// 显示详细信息
        #[arg(short = 'l', long)]
        long: bool,
    },
    /// 按关键字搜索包内文件名
    Find {
        pak: PathBuf,
        keyword: String,
    },
    /// 解包到目录
    Extract {
        pak: PathBuf,
        outdir: PathBuf,
    },
    /// 从目录打包
    Pack {
        indir: PathBuf,
        pak: PathBuf,
        /// 不压缩（原样存储，快）
        #[arg(long)]
        no_compress: bool,
        /// 并行压缩进程数（默认=CPU 核数，1=单线程）
        #[arg(long, default_value_t = default_jobs())]
        jobs: usize,
    },
    /// 替换包内单个文件（其余文件字节原样保留）
    Replace {
        pak: PathBuf,
        inner: String,
        newfile: PathBuf,
        /// 不压缩（原样存储，快）
        #[arg(long)]
        no_compress: bool,
        /// 替换前生成 .bak
        #[arg(long)]
        backup: bool,
    },
    /// 导出单个文件（默认到 stdout）
    Cat {
        pak: PathBuf,
        inner: String,
        /// 保存到文件
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
    },
    /// 校验 pak
    Check {
        pak: PathBuf,
        /// 全量解压验证
        #[arg(long)]
        deep: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    // 对齐 argparse action='version'：解析到即打印退出（先于子命令分发）
    if cli.version {
        println!("1.0");
        return;
    }
    match cli.command {
        // 无子命令进入交互模式（对齐 Python 行为；实现见 wd_pak::interactive）
        None => wd_pak::interactive::interactive(),
        Some(cmd) => {
            if let Err(e) = run(cmd) {
                eprintln!("错误: {e}");
                std::process::exit(2);
            }
        }
    }
}

/// 分发子命令（对齐 Python args.func(args)）。
fn run(cmd: Command) -> Result<(), PakError> {
    use wd_pak::commands_read as cr;
    match cmd {
        Command::List { pak, long } => {
            cr::cmd_list(&pak, long, &mut std::io::stdout().lock(), &mut std::io::stderr())
        }
        Command::Find { pak, keyword } => {
            cr::cmd_find(&pak, &keyword, &mut std::io::stdout().lock(), &mut std::io::stderr())
        }
        Command::Extract { pak, outdir } => {
            cr::cmd_extract(&pak, &outdir, &mut std::io::stderr())
        }
        Command::Pack { indir, pak, no_compress, jobs } => {
            wd_pak::commands_write::cmd_pack(&indir, &pak, !no_compress, jobs, &mut std::io::stderr())
        }
        Command::Replace { pak, inner, newfile, no_compress, backup } => {
            wd_pak::commands_write::cmd_replace(&pak, &inner, &newfile, !no_compress, backup, &mut std::io::stderr())
        }
        Command::Cat { pak, inner, out } => {
            cr::cmd_cat(&pak, &inner, out.as_deref(), &mut std::io::stderr())
        }
        Command::Check { pak, deep } => {
            // check 发现问题：统计报告已写出 stdout，退出码 1（对齐 Python sys.exit(1)）
            if cr::cmd_check(&pak, deep, &mut std::io::stdout().lock())? {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

/// 并行压缩默认线程数（对齐 Python: min(8, os.cpu_count() or 1)）。
fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8)
}

