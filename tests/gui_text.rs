//! 外壳文本投影的接缝测试：钉死从 `src/bin/wd-pak-gui.rs` 外提前的**逐字节**文案。
//!
//! 期望值直接抄自外提前的外壳实现，不从实现内部推导；
//! `entry_text` / `detail_text` 的 mtime 由闭包注入（Win32 格式留在可执行目标）。

use std::path::Path;

use wd_pak::browse::Column;
use wd_pak::error::PakError;
use wd_pak::gui::error_text::{error_reason, error_suggestion, error_title};
use wd_pak::gui::text::{attrib_text, detail_text, entry_text, short_middle, status_text};
use wd_pak::session::{PakSession, SessionEntry};

fn golden(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(rel)
}

fn mini() -> PakSession {
    PakSession::open(&golden("layout/etc_mini_nocompress.pak")).unwrap()
}

/// 确定性的 mtime 注入：外壳侧真实实现是 Win32 FILETIME，此处只钉投影行为。
fn mtime_local(m: u32) -> Option<String> {
    Some(format!("L:{m}"))
}

fn mtime_text(m: u32) -> String {
    format!("M:{m}")
}

fn entry(path: &str, attrib: u32, offset: u32, size: u32, org: u32, mtime: u32) -> SessionEntry {
    SessionEntry {
        path: path.into(),
        attrib,
        offset,
        size,
        org,
        mtime,
    }
}

#[test]
fn error_title_reason_suggestion_are_pinned() {
    let cases: &[(PakError, &str, &str, &str)] = &[
        (
            PakError::Truncated,
            "无法打开这个 PAK",
            "文件不足 16 字节，读不到 PAK 头部。原文件未被修改。",
            "确认文件完整、未被截断",
        ),
        (
            PakError::BadMagic(0xdead_beef),
            "这不是有效的 PAK 文件",
            "文件开头的魔数与 PAK 标识不符。原文件未被修改。",
            "确认选择的是 PAK，而非压缩包或备份",
        ),
        (
            PakError::TreeCycle(0x10),
            "包目录表损坏",
            "包目录表无法解析，文件可能已损坏或不完整。原文件未被修改。",
            "从备份恢复；不要用工具继续写入",
        ),
        (
            PakError::NameTooLong("x".into()),
            "包内文件名过长",
            "包内有不符合命名规则的条目。原文件未被修改。",
            "缩短名称后重试",
        ),
        (
            PakError::NameCaseConflict("a".into(), "A".into()),
            "目录中存在仅大小写不同的文件",
            "读取失败，原文件未被修改。",
            "改名后重试；Windows 解包会互相覆盖",
        ),
        (
            PakError::Missing("x".into()),
            "包内找不到这个条目",
            "读取失败，原文件未被修改。",
            "检查包内路径，或先在列表中定位",
        ),
        (
            PakError::NotExists("x".into()),
            "包内找不到这个条目",
            "读取失败，原文件未被修改。",
            "先解包 → 放入文件 → 重新打包",
        ),
        (
            PakError::DataOob("x".into()),
            "无法打开这个 PAK",
            "读取失败，原文件未被修改。",
            "重试一次；仍失败请复制技术详情",
        ),
    ];

    for (error, title, reason, suggestion) in cases {
        assert_eq!(error_title(error), *title, "title: {error}");
        assert_eq!(error_reason(error), *reason, "reason: {error}");
        assert_eq!(error_suggestion(error), *suggestion, "suggestion: {error}");
    }
}

#[test]
fn entry_text_directory_all_columns() {
    let dir = entry("sub", 1, 0x100, 0, 0, 123);
    assert_eq!(entry_text(&dir, Column::Name, mtime_local), "sub/");
    assert_eq!(entry_text(&dir, Column::Type, mtime_local), "目录");
    assert_eq!(entry_text(&dir, Column::Org, mtime_local), "—");
    assert_eq!(entry_text(&dir, Column::Size, mtime_local), "—");
    assert_eq!(entry_text(&dir, Column::Compression, mtime_local), "—");
    assert_eq!(entry_text(&dir, Column::Attrib, mtime_local), "0x00000001");
    assert_eq!(entry_text(&dir, Column::Offset, mtime_local), "0x00000100");
    assert_eq!(entry_text(&dir, Column::Mtime, mtime_local), "—");
    assert_eq!(entry_text(&dir, Column::Ratio, mtime_local), "—");
}

#[test]
fn entry_text_lzss_file_all_columns() {
    let file = entry("a.txt", 0, 0x200, 100, 400, 12345);
    assert_eq!(entry_text(&file, Column::Name, mtime_local), "a.txt");
    assert_eq!(entry_text(&file, Column::Type, mtime_local), "文件");
    assert_eq!(entry_text(&file, Column::Org, mtime_local), "400");
    assert_eq!(entry_text(&file, Column::Size, mtime_local), "100");
    assert_eq!(entry_text(&file, Column::Compression, mtime_local), "LZSS");
    assert_eq!(entry_text(&file, Column::Attrib, mtime_local), "0x00000000");
    assert_eq!(entry_text(&file, Column::Offset, mtime_local), "0x00000200");
    assert_eq!(entry_text(&file, Column::Mtime, mtime_local), "L:12345");
    assert_eq!(entry_text(&file, Column::Ratio, mtime_local), "0.250");
}

#[test]
fn entry_text_raw_file_all_columns() {
    let file = entry("b.bin", 0x8000_0000, 0x300, 500, 500, 12345);
    assert_eq!(entry_text(&file, Column::Name, mtime_local), "b.bin");
    assert_eq!(entry_text(&file, Column::Type, mtime_local), "文件");
    assert_eq!(entry_text(&file, Column::Org, mtime_local), "500");
    assert_eq!(entry_text(&file, Column::Size, mtime_local), "500");
    assert_eq!(entry_text(&file, Column::Compression, mtime_local), "原样");
    assert_eq!(entry_text(&file, Column::Attrib, mtime_local), "0x80000000");
    assert_eq!(entry_text(&file, Column::Offset, mtime_local), "0x00000300");
    assert_eq!(entry_text(&file, Column::Mtime, mtime_local), "L:12345");
    assert_eq!(entry_text(&file, Column::Ratio, mtime_local), "1.000");
}

#[test]
fn entry_text_org_zero_ratio_is_dash() {
    let file = entry("c.bin", 0, 0x400, 0, 0, 12345);
    assert_eq!(entry_text(&file, Column::Org, mtime_local), "0");
    assert_eq!(entry_text(&file, Column::Size, mtime_local), "0");
    assert_eq!(entry_text(&file, Column::Compression, mtime_local), "LZSS");
    assert_eq!(entry_text(&file, Column::Ratio, mtime_local), "—");
}

#[test]
fn entry_text_mtime_zero_is_unset() {
    let file = entry("d.bin", 0, 0x500, 10, 20, 0);
    assert_eq!(entry_text(&file, Column::Mtime, mtime_local), "未设置");
}

#[test]
fn entry_text_mtime_falls_back_when_local_conversion_fails() {
    let file = entry("e.bin", 0, 0x600, 10, 20, 999);
    assert_eq!(
        entry_text(&file, Column::Mtime, |_| None),
        "999",
        "本地时间转换失败时退回 Unix 秒"
    );
}

#[test]
fn attrib_text_variants() {
    assert_eq!(
        attrib_text(0),
        "0x00000000（bit0/bit31 均未置位，按 LZSS 压缩读取）"
    );
    assert_eq!(attrib_text(1), "0x00000001（bit0=目录）");
    assert_eq!(attrib_text(0x8000_0000), "0x80000000（bit31=原样）");
    assert_eq!(
        attrib_text(0x8000_0001),
        "0x80000001（bit0=目录，bit31=原样）"
    );
    assert_eq!(attrib_text(0x2), "0x00000002（其余位按原值）");
    assert_eq!(
        attrib_text(0x8000_0002),
        "0x80000002（bit31=原样，其余位按原值）"
    );
}

#[test]
fn detail_text_without_session() {
    assert_eq!(
        detail_text(None, None, mtime_text),
        "未打开包\n\n点击“打开 PAK”"
    );
}

#[test]
fn detail_text_session_without_selection_reports_stats() {
    let session = mini();
    let expected = "包统计\n文件: 16\n目录: 0\nrootDirSize: 1024\nfileSize: 636053\n存储/原始: 635013 / 635013\n存储比: 1.000\nattrib 分布: 0x80000000x16\n\n选择一个条目查看详情";
    assert_eq!(detail_text(Some(&session), None, mtime_text), expected);
}

#[test]
fn detail_text_selected_directory() {
    let session = mini();
    let dir = entry("sub", 1, 0x100, 0, 0, 123);
    let expected = "名称: sub\n包内路径: sub\n类型: 目录\n原始大小: —\n存储大小: —\n压缩: —\n存储比: —\nattrib: 0x00000001（bit0=目录）\noffset: 0x00000100\nmtime: M:123";
    assert_eq!(detail_text(Some(&session), Some(&dir), mtime_text), expected);
}

#[test]
fn detail_text_selected_attrib_zero() {
    let session = mini();
    let file = entry("z.bin", 0, 0x0, 10, 20, 5);
    let expected = "名称: z.bin\n包内路径: z.bin\n类型: 文件\n原始大小: 20 字节\n存储大小: 10 字节\n压缩: LZSS\n存储比: 0.500\nattrib: 0x00000000（bit0/bit31 均未置位，按 LZSS 压缩读取）\noffset: 0x00000000\nmtime: M:5";
    assert_eq!(detail_text(Some(&session), Some(&file), mtime_text), expected);
}

#[test]
fn detail_text_nested_entry_shows_leaf_name_once() {
    let session = mini();
    let file = entry("sub/z.bin", 0, 0x0, 10, 20, 5);
    let text = detail_text(Some(&session), Some(&file), mtime_text);
    assert!(text.starts_with("名称: z.bin\n包内路径: sub/z.bin\n"));
    assert!(!text.contains("名称: sub/z.bin"));
}

#[test]
fn status_text_without_path() {
    assert_eq!(status_text(None, None, None), "未打开包");
}

#[test]
fn status_text_path_without_session() {
    let path = Path::new("some/path.pak");
    assert_eq!(status_text(Some(path), None, None), "some/path.pak");
}

#[test]
fn status_text_session_without_selection() {
    let session = mini();
    let expected = "test.pak | 16 个文件 / 0 个目录 | 存储 635013 / 原始 635013（存储比 1.000） | 未选中";
    assert_eq!(
        status_text(Some(Path::new("test.pak")), Some(&session), None),
        expected
    );
}

#[test]
fn status_text_session_with_selection() {
    let session = mini();
    let selected = entry("a.txt", 0, 0x200, 100, 400, 12345);
    let expected = "test.pak | 16 个文件 / 0 个目录 | 存储 635013 / 原始 635013（存储比 1.000） | a.txt";
    assert_eq!(
        status_text(Some(Path::new("test.pak")), Some(&session), Some(&selected)),
        expected
    );
}

#[test]
fn short_middle_boundaries() {
    // 短于 limit：原样返回
    assert_eq!(short_middle("short", 60), "short");
    // limit < 8：原样返回
    assert_eq!(short_middle("abcdefghij", 7), "abcdefghij");
    // 恰好等于 limit：原样返回
    assert_eq!(short_middle("abcdefgh", 8), "abcdefgh");
    // > limit：head=3, tail=4 → "abc…ghij"
    assert_eq!(short_middle("abcdefghij", 8), "abc…ghij");
    // UTF-8 按字符计数
    assert_eq!(short_middle("abcdefghij", 8).chars().count(), 8);
}

#[test]
fn status_text_truncates_long_path_in_middle() {
    let session = mini();
    let path = "x".repeat(80);
    let text = status_text(Some(Path::new(&path)), Some(&session), None);
    assert!(text.contains('…'));
    assert!(text.ends_with("| 未选中"));
}
