//! 统一错误模型 —— 变体与中文消息对齐 Python 版 wd_pak_tool.py 的 PakError。
//! 消息文本保持一致，便于双实现行为对照与 golden 测试断言。

use thiserror::Error;

/// PAK 工具统一错误。
#[derive(Debug, Error)]
pub enum PakError {
    /// 文件头部不足 16 字节
    #[error("文件太小，不是 PAK")]
    Truncated,

    /// 魔数不符
    #[error("魔数不对: 0x{0:08x} (期望 0x11223344)")]
    BadMagic(u32),

    /// 目录表读取越界
    #[error("目录表越界: offset=0x{offset:x} size={size}")]
    CatalogOob { offset: u64, size: u64 },

    /// 目录表大小不是 64 的倍数
    #[error("目录表大小 {size} 不是 64 的倍数 (offset=0x{offset:x})")]
    CatalogBadSize { offset: u64, size: u64 },

    /// 文件数据读取越界
    #[error("读取越界: {0}")]
    DataOob(String),

    /// RAW 文件存储数据少于声明的原始大小
    #[error("数据不足: {name} 存储 {stored} < 原始 {org} 字节")]
    DataShort { name: String, stored: u64, org: u64 },

    /// 解压结果与声明的原始大小不符
    #[error("解压大小不符: {name} 得到 {got} != 声明 {declared} 字节（数据损坏）")]
    SizeMismatch { name: String, got: u64, declared: u64 },

    /// 目录嵌套超过 64 层
    #[error("目录嵌套过深 (offset=0x{0:x})")]
    TreeTooDeep(u64),

    /// 同一目录表被重复引用（循环/损坏）
    #[error("目录表被重复引用，疑似损坏/循环 (offset=0x{0:x})")]
    TreeCycle(u64),

    /// 包内非法文件名（空 / '.' '..' / 含 '/' '\\' '\\0'）
    #[error("包内非法文件名: '{0}'（疑似路径穿越或损坏）")]
    BadName(String),

    /// 文件名超过 43 字节
    #[error("文件名过长: '{0}' (最多 43 字节)")]
    NameTooLong(String),

    /// 文件名含非 Latin-1 字符（OS 文件名打包时触发）
    #[error("文件名含非 ASCII 字符: '{0}'（pak 内文件名必须为 ASCII）")]
    NameNonAscii(String),

    /// 写侧 OS 文件名含 '/' '\\' '\\0'（读侧 BadName 消息不同，不复用）
    #[error("非法文件名: '{0}'")]
    BadOsName(String),

    /// 同目录大小写冲突
    #[error("同目录大小写冲突: {0} / {1} （Windows 解包会互相覆盖丢数据，请先改名）")]
    NameCaseConflict(String, String),

    /// 目标路径是目录
    #[error("{0} 是目录")]
    IsDir(String),

    /// 打包源不是目录
    #[error("不是目录: {0}")]
    NotDir(String),

    /// 包内不存在该路径
    #[error("包内不存在: {0}（如需新增文件请先 extract，放入文件后 pack）")]
    NotExists(String),

    /// manifest 数据非法（承载完整中文消息）
    #[error("{0}")]
    Manifest(String),

    /// Python OSError 域（承载 `[Errno N] ...: '<path>'` 完整消息）。
    /// CLI 模式与其余 PakError 同样输出 `错误: {e}`（Display 无差别）；
    /// 交互模式按 Python 语义区分：OSError → `出错: {e}`（非 PakError 分支）。
    #[error("{0}")]
    Os(String),

    /// IO 错误（消息格式统一由平台层决定：Unix `[Errno n] strerror`，
    /// Windows `[Win32 n] 固定文案`）
    #[error("{}", crate::platform::io_message(.0))]
    Io(#[from] std::io::Error),
}