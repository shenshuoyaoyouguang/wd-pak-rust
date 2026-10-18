//! G1-03.1 / G1-06.1 / G1-07.1 工作区接缝：打开请求、标签状态与启动模式。
//!
//! 只验证纯状态模型；文件系统、对话框与窗口呈现由后续任务接线。

use std::path::PathBuf;

use wd_pak::workspace::{startup, OpenRequest, OpenSource, Startup, Workspace};

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[test]
fn open_dedupes_by_path_and_activates_existing() {
    let mut workspace = Workspace::new();
    let a = workspace.open(p("a.pak"));
    let b = workspace.open(p("b.pak"));
    assert_ne!(a, b);
    assert_eq!(workspace.active_index(), Some(b));

    let again = workspace.open(p("a.pak"));
    assert_eq!(again, a);
    assert_eq!(workspace.active_index(), Some(a));
    assert_eq!(workspace.tabs().len(), 2);
}

#[test]
fn write_busy_tab_cannot_close() {
    let mut workspace = Workspace::new();
    let index = workspace.open(p("a.pak"));
    workspace.set_write_busy(index, true);

    assert!(!workspace.close(index));
    assert_eq!(workspace.tabs().len(), 1);

    workspace.set_write_busy(index, false);
    assert!(workspace.close(index));
    assert!(workspace.tabs().is_empty());
    assert_eq!(workspace.active_index(), None);
}

#[test]
fn tabs_keep_independent_view_and_selection() {
    let mut workspace = Workspace::new();
    let a = workspace.open(p("a.pak"));
    let b = workspace.open(p("b.pak"));

    workspace.tab_mut(a).unwrap().selection.select_one(3);
    workspace.tab_mut(a).unwrap().view.query.keyword = "abc".into();

    assert!(workspace.tab(b).unwrap().selection.is_empty());
    assert!(workspace.tab(b).unwrap().view.query.keyword.is_empty());
}

#[test]
fn startup_mode_uses_config_session_and_restore_flag() {
    assert_eq!(startup(false, false, true), Startup::FirstRun);
    assert_eq!(startup(true, true, true), Startup::Restore);
    assert_eq!(startup(true, true, false), Startup::Workbench);
    assert_eq!(startup(true, false, true), Startup::Workbench);
}

#[test]
fn open_request_rejects_empty_path() {
    assert!(OpenRequest::new(&p(""), OpenSource::CommandBar).is_none());

    let request = OpenRequest::new(&p("a.pak"), OpenSource::Drop).unwrap();
    assert_eq!(request.path, p("a.pak"));
    assert_eq!(request.source, OpenSource::Drop);
}
