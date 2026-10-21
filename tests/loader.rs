//! G1-03.3 后台加载接缝：解析离开界面线程，取消与失败可观察。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use wd_pak::loader::{LoadResult, Loader};

fn golden(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(rel)
}

fn wait(loader: &mut Loader) -> LoadResult {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(result) = loader.poll() {
            return result;
        }
        assert!(Instant::now() < deadline, "加载超时");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn loads_pak_off_the_main_thread_with_cli_counts() {
    let mut loader = Loader::start(golden("layout/etc_mini_nocompress.pak"));
    assert!(loader.path().ends_with("etc_mini_nocompress.pak"));

    match wait(&mut loader) {
        LoadResult::Ready(session) => {
            assert_eq!(session.stats().files, 16);
            assert_eq!(session.stats().dirs, 0);
        }
        other => panic!("期望加载成功，得到 {other:?}"),
    }
    // 结果只交付一次
    assert!(loader.poll().is_none());
}

#[test]
fn progress_reports_parsed_entry_count() {
    let mut loader = Loader::start(golden("layout/etc_mini_nocompress.pak"));

    assert!(matches!(wait(&mut loader), LoadResult::Ready(_)));
    // golden 包 16 个文件、0 个目录；完成前应能看到已解析项数
    assert_eq!(loader.progress(), Some(16));
}

#[test]
fn cancel_wins_even_if_parsing_finished() {
    let mut loader = Loader::start(golden("layout/etc_mini_nocompress.pak"));
    loader.cancel();

    assert!(matches!(wait(&mut loader), LoadResult::Cancelled));
    assert!(loader.poll().is_none());
}

#[test]
fn cancel_joins_worker_before_reporting_cancelled() {
    let mut loader = Loader::start(golden("layout/etc_mini_nocompress.pak"));
    loader.cancel();

    assert!(matches!(wait(&mut loader), LoadResult::Cancelled));
    assert!(loader.is_finished(), "取消后必须完成 join，不能留下后台解析线程");
}

#[test]
fn missing_file_reports_failed_with_path() {
    let path = std::env::temp_dir().join("wd_pak_loader_missing.pak");
    let _ = std::fs::remove_file(&path);

    let mut loader = Loader::start(&path);

    match wait(&mut loader) {
        LoadResult::Failed(error) => {
            let text = error.to_string();
            assert!(
                text.contains("wd_pak_loader_missing.pak"),
                "错误应带路径: {text}"
            );
        }
        other => panic!("期望加载失败，得到 {other:?}"),
    }
}
