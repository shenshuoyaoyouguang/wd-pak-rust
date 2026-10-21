//! G2 浏览与检索接缝：列、排序、查询、选中、树投影与定位。
//!
//! 列默认值与排序规则来自规格 §2.4；查询语义与 CLI `find` 对齐
//! （`src/commands_read.rs` 的 `rel.to_lowercase().contains(kw)`）。

use wd_pak::browse::{
    dedupe_requests, default_visible, locate, locate_nearest, sorted_indices, Column,
    CompressionFilter, KindFilter, LocateRequest, LocateSource, Query, Scope, Selection, SortDir,
    SortSpec, TreeState, ViewMode,
};
use wd_pak::format::DIR_BIT;
use wd_pak::session::SessionEntry;

fn entry(path: &str, size: u32, org: u32, attrib: u32) -> SessionEntry {
    SessionEntry {
        path: path.into(),
        attrib,
        offset: 0,
        size,
        org,
        mtime: 0,
    }
}

fn paths<'a>(indices: &[usize], entries: &'a [SessionEntry]) -> Vec<&'a str> {
    indices.iter().map(|&i| entries[i].path.as_str()).collect()
}

fn row_paths<'a>(tree: &TreeState, entries: &'a [SessionEntry]) -> Vec<&'a str> {
    tree.rows(entries)
        .iter()
        .map(|row| entries[row.index].path.as_str())
        .collect()
}

#[test]
fn column_defaults_match_spec() {
    let defaults: Vec<bool> = Column::ALL
        .iter()
        .map(|c| default_visible(*c, ViewMode::List))
        .collect();
    assert_eq!(
        defaults,
        vec![true, true, true, true, true, false, false, true, false]
    );

    assert!(!default_visible(Column::Type, ViewMode::Tree));
    assert!(default_visible(Column::Name, ViewMode::Tree));
}

#[test]
fn sorts_directories_first_then_selected_column() {
    let entries = vec![
        entry("b.txt", 2, 2, 0),
        entry("dir", 0, 0, DIR_BIT),
        entry("a.txt", 1, 1, 0),
    ];

    let asc = sorted_indices(
        &entries,
        SortSpec {
            key: Column::Size,
            dir: SortDir::Asc,
        },
    );
    assert_eq!(paths(&asc, &entries), vec!["dir", "a.txt", "b.txt"]);

    let desc = sorted_indices(
        &entries,
        SortSpec {
            key: Column::Size,
            dir: SortDir::Desc,
        },
    );
    assert_eq!(paths(&desc, &entries), vec!["dir", "b.txt", "a.txt"]);
}

fn browse_fixture() -> Vec<SessionEntry> {
    vec![
        entry("100heis.list", 10, 10, 0),
        entry("sub/cfg.txt", 5, 5, 0),
        entry("sub/deep/x.bin", 3, 1, 0x8000_0000),
        entry("dir", 0, 0, DIR_BIT),
    ]
}

#[test]
fn query_matches_full_path_case_insensitively_and_skips_content() {
    let entries = browse_fixture();

    let heis = Query {
        keyword: "HEIS".into(),
        ..Default::default()
    };
    assert_eq!(heis.apply(&entries), vec![0]);

    let sub = Query {
        keyword: "sub".into(),
        ..Default::default()
    };
    assert_eq!(sub.apply(&entries), vec![1, 2]);

    let content = Query {
        keyword: "hello".into(),
        ..Default::default()
    };
    assert!(content.apply(&entries).is_empty());

    let empty = Query::default();
    assert_eq!(empty.apply(&entries), vec![0, 1, 2, 3]);
}

#[test]
fn query_combines_scope_kind_and_compression() {
    let entries = browse_fixture();

    let scoped = Query {
        keyword: String::new(),
        scope: Scope::Directory("sub".into()),
        kind: KindFilter::Files,
        compression: CompressionFilter::Lzss,
    };
    assert_eq!(scoped.apply(&entries), vec![1]);

    let raw = Query {
        keyword: String::new(),
        scope: Scope::Package,
        kind: KindFilter::Files,
        compression: CompressionFilter::Raw,
    };
    assert_eq!(raw.apply(&entries), vec![2]);
}

#[test]
fn ctrl_toggle_and_shift_range_track_primary() {
    let visible = [0, 1, 2, 3, 4, 5];
    let mut selection = Selection::default();

    selection.select_one(2);
    selection.toggle(5);
    assert!(selection.contains(2));
    assert!(selection.contains(5));
    assert_eq!(selection.primary(), Some(5));

    selection.toggle(2);
    assert!(!selection.contains(2));
    assert_eq!(selection.items().collect::<Vec<_>>(), vec![5]);

    selection.extend_to(&visible, 2);
    assert_eq!(selection.items().collect::<Vec<_>>(), vec![2, 3, 4, 5]);
    assert_eq!(selection.primary(), Some(2));
}

#[test]
fn select_all_and_move_primary_stay_within_visible() {
    let visible = [1, 3, 7];
    let mut selection = Selection::default();

    selection.select_all(&visible);
    assert_eq!(selection.items().collect::<Vec<_>>(), vec![1, 3, 7]);
    assert_eq!(selection.primary(), Some(1));

    selection.move_primary(&visible, 1);
    selection.move_primary(&visible, 1);
    selection.move_primary(&visible, 1);
    assert_eq!(selection.primary(), Some(7));

    selection.move_primary(&visible, -3);
    assert_eq!(selection.primary(), Some(1));
}

fn tree_fixture() -> Vec<SessionEntry> {
    vec![
        entry("a", 0, 0, DIR_BIT),
        entry("a/x.txt", 1, 1, 0),
        entry("b.txt", 2, 2, 0),
        entry("c", 0, 0, DIR_BIT),
    ]
}

#[test]
fn tree_rows_start_with_first_level_expanded() {
    let entries = tree_fixture();
    let mut tree = TreeState::new();

    let rows = tree.rows(&entries);
    assert_eq!(
        row_paths(&tree, &entries),
        vec!["a", "a/x.txt", "c", "b.txt"]
    );
    assert!(rows[0].has_children);
    assert!(!rows[2].has_children);

    tree.collapse("a");
    assert_eq!(row_paths(&tree, &entries), vec!["a", "c", "b.txt"]);

    tree.expand("a");
    assert_eq!(
        row_paths(&tree, &entries),
        vec!["a", "a/x.txt", "c", "b.txt"]
    );

    tree.collapse_all(&entries);
    assert_eq!(row_paths(&tree, &entries), vec!["a", "c", "b.txt"]);

    tree.expand_all(&entries);
    assert_eq!(
        row_paths(&tree, &entries),
        vec!["a", "a/x.txt", "c", "b.txt"]
    );
}

#[test]
fn repeat_shift_click_keeps_the_original_anchor() {
    let visible = [0, 1, 2, 3, 4, 5];
    let mut selection = Selection::default();

    selection.select_one(0);
    selection.extend_to(&visible, 3);
    assert_eq!(selection.items().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(selection.primary(), Some(3));

    selection.extend_to(&visible, 1);
    assert_eq!(selection.items().collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(selection.primary(), Some(1));
}

#[test]
fn type_column_is_not_sortable() {
    assert!(!Column::Type.sortable());
    assert!(Column::Name.sortable());
    assert!(Column::Size.sortable());
}

#[test]
fn locate_finds_exact_path_and_falls_back_to_parent() {
    let entries = tree_fixture();

    assert_eq!(locate(&entries, "a/x.txt"), Some(1));
    assert_eq!(locate(&entries, "a\\x.txt"), Some(1));
    assert_eq!(locate(&entries, "/a/x.txt/"), Some(1));
    assert_eq!(locate(&entries, "a/missing.txt"), None);

    assert_eq!(locate_nearest(&entries, "a/missing.txt"), Some(0));
    assert_eq!(locate_nearest(&entries, "a/x/y/z.bin"), Some(0));
    assert_eq!(locate_nearest(&entries, "z/y"), None);
    // 文件不能充当父目录兜底；精确命中文件仍然有效
    assert_eq!(locate_nearest(&entries, "b.txt"), Some(2));
    assert_eq!(locate_nearest(&entries, "b.txt/x.bin"), None);
}

#[test]
fn locate_request_normalizes_dedupes_and_keeps_source() {
    let first = LocateRequest::new("\\sub\\a.txt", LocateSource::Search).unwrap();
    assert_eq!(first.path, "sub/a.txt");
    assert!(first.highlight);

    assert!(LocateRequest::new("/", LocateSource::Task).is_none());

    let duplicate = LocateRequest::new("/sub/a.txt/", LocateSource::Validation).unwrap();
    let other = LocateRequest::new("other.bin", LocateSource::Task).unwrap();
    let requests = dedupe_requests(&[first, duplicate, other]);

    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "sub/a.txt");
    assert_eq!(requests[0].source, LocateSource::Search);
    assert_eq!(requests[1].path, "other.bin");
    assert_eq!(requests[1].source, LocateSource::Task);
}
