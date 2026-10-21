//! G0-07 环境自检接缝：目录可写、长路径状态、缩放百分比与最近错误。
//!
//! 长路径用注入探针判定，测试不依赖真实系统设置；期望文案来自规格
//! `docs/windows-ui-plan.md` §4.6 的“已启用 / 未启用”。

use std::io;
use std::path::{Path, PathBuf};

use wd_pak::diagnostics::{dir_writable, long_paths_enabled, scale_percent, ErrorLog};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wd_pak_diag_{}_{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn writable_dir_reports_ok_and_leaves_no_probe() {
    let dir = temp_dir("writable");

    let (ok, detail) = dir_writable(&dir);
    assert!(ok, "{detail}");
    assert!(std::fs::read_dir(&dir).map(|entries| entries.count()).unwrap_or(0) == 0);
}

#[test]
fn unwritable_dir_reports_reason() {
    let dir = temp_dir("unwritable");
    std::fs::create_dir_all(&dir).unwrap();
    // 用普通文件充当父目录，制造必然失败的写入（同 tests/config.rs 的手法）
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "x").unwrap();

    let (ok, detail) = dir_writable(&blocker.join("sub"));
    assert!(!ok);
    assert!(detail.contains("blocker"), "原因应含路径: {detail}");
}

#[test]
fn long_path_state_follows_injected_probe() {
    let ok = long_paths_enabled(&mut |_| Ok(()));
    assert!(ok.0);
    assert!(ok.1.contains("已启用"), "{:?}", ok.1);

    let failed = long_paths_enabled(&mut |_| Err(io::Error::from_raw_os_error(206)));
    assert!(!failed.0);
    assert!(failed.1.contains("未启用"), "{:?}", failed.1);
}

#[test]
fn scale_percent_maps_dpi() {
    assert_eq!(scale_percent(96), 100);
    assert_eq!(scale_percent(120), 125);
    assert_eq!(scale_percent(144), 150);
    assert_eq!(scale_percent(192), 200);
    // dpi 取不到时按 100% 处理（与界面 dpi 回退 96 一致）
    assert_eq!(scale_percent(0), 100);
}

#[test]
fn error_log_keeps_only_last_five_newest_first() {
    let mut log = ErrorLog::new(5);
    for at in 1..=7u64 {
        log.record(at, "无法打开这个 PAK", &format!("第 {at} 条"));
    }

    assert_eq!(log.items().len(), 5);
    assert_eq!(log.items()[0].detail, "第 7 条");
    assert_eq!(log.items()[4].detail, "第 3 条");
    assert!(log.items().windows(2).all(|pair| pair[0].at >= pair[1].at));

    let mut empty = ErrorLog::new(5);
    empty.record(1, "标题", "详情");
    assert_eq!(empty.items()[0].title, "标题");
}

#[test]
fn probe_path_is_longer_than_windows_limit() {
    let mut seen: Option<PathBuf> = None;
    long_paths_enabled(&mut |path: &Path| {
        seen = Some(path.to_path_buf());
        Ok(())
    });

    let seen = seen.expect("探针应收到路径");
    assert!(seen.to_string_lossy().len() > 260, "{seen:?}");
}
