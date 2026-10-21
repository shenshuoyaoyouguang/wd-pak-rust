//! 错误对话框的标题 / 原因 / 建议映射（G0 阶段 1 外提）。
//!
//! 自 `src/bin/wd-pak-gui.rs` 逐字搬移，只调整可见性；无平台依赖。

use crate::error::PakError;

/// 标题与建议逐条对应 `docs/windows-ui-plan.md` §4.7 的错误映射表；
/// 不同错误不得合并成同一句话（硬约束 2）。
pub fn error_title(error: &PakError) -> &'static str {
    match error {
        PakError::BadMagic(_) => "这不是有效的 PAK 文件",
        PakError::CatalogOob { .. }
        | PakError::CatalogBadSize { .. }
        | PakError::TreeTooDeep(_)
        | PakError::TreeCycle(_) => "包目录表损坏",
        PakError::NameTooLong(_) => "包内文件名过长",
        PakError::NameNonAscii(_) => "包内文件名含非 ASCII 字符",
        PakError::BadName(_) => "包内文件名非法",
        PakError::NameCaseConflict(..) => "目录中存在仅大小写不同的文件",
        PakError::Missing(_) | PakError::NotExists(_) => "包内找不到这个条目",
        _ => "无法打开这个 PAK",
    }
}

pub fn error_reason(error: &PakError) -> &'static str {
    match error {
        PakError::Truncated => "文件不足 16 字节，读不到 PAK 头部。原文件未被修改。",
        PakError::BadMagic(_) => "文件开头的魔数与 PAK 标识不符。原文件未被修改。",
        PakError::CatalogOob { .. }
        | PakError::CatalogBadSize { .. }
        | PakError::TreeTooDeep(_)
        | PakError::TreeCycle(_) => "包目录表无法解析，文件可能已损坏或不完整。原文件未被修改。",
        PakError::NameTooLong(_) | PakError::NameNonAscii(_) | PakError::BadName(_) => {
            "包内有不符合命名规则的条目。原文件未被修改。"
        }
        _ => "读取失败，原文件未被修改。",
    }
}

pub fn error_suggestion(error: &PakError) -> &'static str {
    match error {
        PakError::Truncated => "确认文件完整、未被截断",
        PakError::BadMagic(_) => "确认选择的是 PAK，而非压缩包或备份",
        PakError::CatalogOob { .. }
        | PakError::CatalogBadSize { .. }
        | PakError::TreeTooDeep(_)
        | PakError::TreeCycle(_) => "从备份恢复；不要用工具继续写入",
        PakError::NameTooLong(_) => "缩短名称后重试",
        PakError::NameNonAscii(_) => "改为 ASCII 名称后重试",
        PakError::NameCaseConflict(..) => "改名后重试；Windows 解包会互相覆盖",
        PakError::Missing(_) => "检查包内路径，或先在列表中定位",
        PakError::NotExists(_) => "先解包 → 放入文件 → 重新打包",
        _ => "重试一次；仍失败请复制技术详情",
    }
}
