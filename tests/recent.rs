//! G1-02.1 最近打开模型接缝：去重、排序、上限与固定。
//!
//! 期望值来自规格 §3.1、§4.3 的行为描述，时间由测试注入，避免依赖真实时钟。

use std::path::PathBuf;

use wd_pak::recent::{RecentEntry, RecentList, DEFAULT_LIMIT, MAX_LIMIT};

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn opened(path: &str, size: u64, at: u64) -> RecentEntry {
    RecentEntry {
        path: p(path),
        name: path.into(),
        size,
        entry_count: Some(1),
        last_opened: at,
        pinned: false,
    }
}

#[test]
fn records_newest_first_and_dedupes_by_path() {
    let mut list = RecentList::new(DEFAULT_LIMIT);
    list.record(opened("a.pak", 10, 100));
    list.record(opened("b.pak", 20, 200));
    list.record(opened("a.pak", 11, 300));

    let items = list.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].path, p("a.pak"));
    assert_eq!(items[0].size, 11);
    assert_eq!(items[0].last_opened, 300);
    assert_eq!(items[1].path, p("b.pak"));
}

#[test]
fn limit_is_clamped_to_spec_range() {
    assert_eq!(RecentList::new(DEFAULT_LIMIT).limit(), 10);
    assert_eq!(RecentList::new(100).limit(), MAX_LIMIT);
    assert_eq!(RecentList::new(0).limit(), 0);
}

#[test]
fn zero_limit_records_nothing() {
    let mut list = RecentList::new(0);
    list.record(opened("a.pak", 10, 100));

    assert!(list.items().is_empty());
}

#[test]
fn pinned_entry_survives_limit_and_sorts_first() {
    let mut list = RecentList::new(2);
    list.record(opened("a.pak", 10, 100));
    list.record(opened("b.pak", 20, 200));
    assert!(list.set_pinned(&p("a.pak"), true));

    list.record(opened("c.pak", 30, 300));

    let items = list.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].path, p("a.pak"));
    assert!(items[0].pinned);
    assert_eq!(items[1].path, p("c.pak"));
}

#[test]
fn re_recording_a_pinned_path_keeps_it_pinned() {
    let mut list = RecentList::new(DEFAULT_LIMIT);
    list.record(opened("a.pak", 10, 100));
    assert!(list.set_pinned(&p("a.pak"), true));

    list.record(opened("a.pak", 20, 300));

    assert_eq!(list.items().len(), 1);
    assert!(list.items()[0].pinned);
    assert_eq!(list.items()[0].size, 20);
    assert_eq!(list.items()[0].last_opened, 300);
}

#[test]
fn limit_holds_even_when_every_entry_is_pinned() {
    let mut list = RecentList::new(2);
    for (path, at) in [("a.pak", 100), ("b.pak", 200), ("c.pak", 300)] {
        let mut entry = opened(path, 10, at);
        entry.pinned = true;
        list.record(entry);
    }

    assert_eq!(list.items().len(), 2);
    assert_eq!(list.items()[0].path, p("c.pak"));
    assert_eq!(list.items()[1].path, p("b.pak"));
}

#[test]
fn remove_and_clear_drop_entries() {
    let mut list = RecentList::new(DEFAULT_LIMIT);
    list.record(opened("a.pak", 10, 100));
    list.record(opened("b.pak", 20, 200));

    assert!(list.remove(&p("a.pak")));
    assert!(!list.remove(&p("missing.pak")));
    assert_eq!(list.items().len(), 1);

    list.clear();
    assert!(list.items().is_empty());
}
