//! G1-04 包会话接缝：从界面层视角打开包，取得只读快照。
//!
//! 期望值取自 CLI 在 golden 包上的实际输出（`list -l` 与头部统计），
//! 不从实现内部推导。

use std::io::Write;
use std::path::{Path, PathBuf};

fn golden(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(rel)
}

fn temp_pak(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wd_pak_session_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join("mini.pak");
    std::fs::copy(golden("layout/etc_mini_nocompress.pak"), &dst).unwrap();
    dst
}

#[test]
fn opens_pak_and_reports_cli_equivalent_counts() {
    let session =
        wd_pak::session::PakSession::open(&golden("layout/etc_mini_nocompress.pak")).unwrap();

    let stats = session.stats();
    assert_eq!(stats.files, 16);
    assert_eq!(stats.dirs, 0);
    assert_eq!(stats.root_dir_size, 1024);
    assert_eq!(stats.file_size, 636_053);
    assert_eq!(stats.total_stored, 635_013);
    assert_eq!(stats.total_org, 635_013);
    assert!((stats.ratio - 1.0).abs() < 1e-9);
    assert_eq!(stats.attribs.get(&0x8000_0000), Some(&16));
}

#[test]
fn snapshot_exposes_entry_metadata_in_catalog_order() {
    let session =
        wd_pak::session::PakSession::open(&golden("layout/etc_mini_nocompress.pak")).unwrap();

    let entries = session.entries();
    assert_eq!(entries.len(), 16);

    let first = &entries[0];
    assert_eq!(first.path, "100heis.list");
    assert!(!first.is_dir());
    assert!(first.is_raw());
    assert_eq!(first.attrib, 0x8000_0000);
    assert_eq!(first.offset, 0x410);
    assert_eq!(first.size, 7291);
    assert_eq!(first.org, 7291);
    assert_eq!(first.mtime, 0);

    // Latin-1 路径按原字节映射保留，且按目录表顺序排在最后。
    let last = &entries[15];
    assert_eq!(last.path, "café.list");
    assert_eq!(last.offset, 0x9ad29);
    assert_eq!(last.size, 1900);
}

#[test]
fn reads_raw_and_compressed_entries_as_original_bytes() {
    let raw = wd_pak::session::PakSession::open(&golden("layout/etc_mini_nocompress.pak")).unwrap();
    assert_eq!(
        raw.read("100heis.list").unwrap(),
        std::fs::read(golden("layout/etc_mini/100heis.list")).unwrap()
    );

    let packed =
        wd_pak::session::PakSession::open(&golden("layout/etc_mini_compress.pak")).unwrap();
    assert_eq!(
        packed.read("Font1.ttf").unwrap(),
        std::fs::read(golden("layout/etc_mini/Font1.ttf")).unwrap()
    );
}

#[test]
fn fingerprint_marks_session_stale_after_external_change() {
    let pak = temp_pak("stale");
    let session = wd_pak::session::PakSession::open(&pak).unwrap();
    assert!(session.is_current());

    std::fs::OpenOptions::new()
        .append(true)
        .open(&pak)
        .unwrap()
        .write_all(&[0])
        .unwrap();

    assert!(!session.is_current());
    drop(session);
    std::fs::remove_dir_all(pak.parent().unwrap()).unwrap();
}
