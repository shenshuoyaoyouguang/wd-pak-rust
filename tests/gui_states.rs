use wd_pak::gui::states::{
    ConflictDecision, Notification, NotificationCenter, NotificationKind, NotificationState,
    PrecheckState, SessionState, TaskKind, TaskState, ValidationState, WriteTarget, needs_wait,
    write_conflicts,
};

#[test]
fn session_state_predicates() {
    assert!(SessionState::Ready.is_ready());
    assert!(SessionState::Loading.is_loading());
    assert!(!SessionState::Error("x".into()).is_ready());
}

#[test]
fn task_state_terminal_and_active() {
    assert!(TaskState::Done.is_terminal());
    assert!(TaskState::Failed("e".into()).is_terminal());
    assert!(TaskState::Cancelled.is_terminal());
    assert!(!TaskState::Running.is_terminal());
    assert!(TaskState::Queued.is_active());
    assert!(TaskState::Running.is_active());
    assert!(!TaskState::Done.is_active());
}

#[test]
fn task_kind_write_classification() {
    assert!(TaskKind::Replace.is_write());
    assert!(TaskKind::Pack.is_write());
    assert!(TaskKind::Unpack.is_write());
    assert!(!TaskKind::Export.is_write());
    assert!(!TaskKind::ShallowCheck.is_write());
}

#[test]
fn validation_invalidate_on_write_and_fingerprint() {
    let mut v = ValidationState::Passed;
    v.invalidate_on_write();
    assert_eq!(v, ValidationState::Expired);

    let mut v = ValidationState::Problems(3);
    v.invalidate_on_fingerprint_change();
    assert_eq!(v, ValidationState::Expired);

    // Unchecked / Running / Cancelled / Expired 不会被重复标过期（保持原状）
    let mut v = ValidationState::Unchecked;
    v.invalidate_on_write();
    assert_eq!(v, ValidationState::Unchecked);

    let mut v = ValidationState::Running;
    v.invalidate_on_write();
    assert_eq!(v, ValidationState::Expired);

    let mut v = ValidationState::Cancelled;
    v.invalidate_on_write();
    assert_eq!(v, ValidationState::Cancelled);

    let mut v = ValidationState::Expired;
    v.invalidate_on_write();
    assert_eq!(v, ValidationState::Expired);
}

#[test]
fn precheck_can_start_only_when_passed() {
    assert!(!PrecheckState::Unchecked.can_start());
    assert!(!PrecheckState::Checking.can_start());
    assert!(PrecheckState::Passed.can_start());
    assert!(!PrecheckState::Blocked(vec!["x".into()]).can_start());
}

#[test]
fn conflict_decision_equality() {
    assert_eq!(ConflictDecision::Pending, ConflictDecision::Pending);
    assert_ne!(ConflictDecision::Replace, ConflictDecision::Skip);
}

#[test]
fn notification_center_dedup_and_merge() {
    let mut center = NotificationCenter::new();
    let id1 = center.push(NotificationKind::TaskDone, "done a", Some("a.pak"));
    let id2 = center.push(NotificationKind::TaskDone, "done a (updated)", Some("a.pak"));
    // 合并：同一 id
    assert_eq!(id1, id2);
    assert_eq!(center.items().len(), 1);
    assert_eq!(center.items()[0].title, "done a (updated)");
    assert_eq!(center.unread_count(), 1);

    // 不同 target 不合并
    let id3 = center.push(NotificationKind::TaskDone, "done b", Some("b.pak"));
    assert_ne!(id1, id3);
    assert_eq!(center.items().len(), 2);

    // 不同 kind 不合并
    let id4 = center.push(NotificationKind::TaskFailed, "fail a", Some("a.pak"));
    assert_ne!(id1, id4);
    assert_eq!(center.items().len(), 3);
}

#[test]
fn notification_center_mark_read_and_clear() {
    let mut center = NotificationCenter::new();
    let id = center.push(NotificationKind::TaskDone, "done", None);
    center.mark_read(id);
    assert_eq!(center.unread_count(), 0);
    assert_eq!(center.items()[0].state, NotificationState::Read);
    center.clear_read();
    assert!(center.items().is_empty());
}

#[test]
fn notification_center_mark_all_read() {
    let mut center = NotificationCenter::new();
    center.push(NotificationKind::TaskDone, "a", None);
    center.push(NotificationKind::TaskFailed, "b", None);
    center.mark_all_read();
    assert_eq!(center.unread_count(), 0);
}

#[test]
fn notification_center_invalidate_target() {
    let mut center = NotificationCenter::new();
    center.push(NotificationKind::TaskDone, "a", Some("a.pak"));
    center.invalidate_target("a.pak");
    assert_eq!(center.items()[0].state, NotificationState::Invalid);
}

#[test]
fn notification_new_state_unread() {
    let n = Notification::new(1, NotificationKind::Environment, "env", None);
    assert_eq!(n.state, NotificationState::Unread);
    assert!(n.target.is_none());
}

#[test]
fn write_conflicts_same_pak() {
    let a = WriteTarget {
        pak: "x.pak".into(),
        output: None,
    };
    let b = WriteTarget {
        pak: "x.pak".into(),
        output: Some("out".into()),
    };
    assert!(write_conflicts(&a, &b));
}

#[test]
fn write_conflicts_same_output() {
    let a = WriteTarget {
        pak: "a.pak".into(),
        output: Some("out".into()),
    };
    let b = WriteTarget {
        pak: "b.pak".into(),
        output: Some("out".into()),
    };
    assert!(write_conflicts(&a, &b));
}

#[test]
fn write_conflicts_none() {
    let a = WriteTarget {
        pak: "a.pak".into(),
        output: None,
    };
    let b = WriteTarget {
        pak: "b.pak".into(),
        output: Some("out2".into()),
    };
    assert!(!write_conflicts(&a, &b));
}

#[test]
fn needs_wait_checks_queue() {
    let q = vec![WriteTarget {
        pak: "x.pak".into(),
        output: None,
    }];
    let cand = WriteTarget {
        pak: "y.pak".into(),
        output: None,
    };
    assert!(!needs_wait(&q, &cand));
    let cand2 = WriteTarget {
        pak: "x.pak".into(),
        output: None,
    };
    assert!(needs_wait(&q, &cand2));
}
