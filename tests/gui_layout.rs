//! G0-01 窗口五区布局与 DPI 换算的纯逻辑接缝。
//!
//! 尺寸与断点来自 `docs/windows-ui-plan.md` §5.1；任务面板位于内容区底部（§2.5）。

use wd_pak::gui::layout::{layout, LayoutInput, Rect};

fn input(width: i32, height: i32) -> LayoutInput {
    LayoutInput { width, height }
}

#[test]
fn default_size_places_zones_without_overlap() {
    let rects = layout(input(1280, 800));

    assert_eq!(rects.command_bar, Rect::new(0, 0, 1280, 48));
    assert_eq!(rects.nav, Rect::new(0, 48, 220, 728));
    assert_eq!(rects.tabs, Rect::new(220, 48, 740, 36));
    assert_eq!(rects.content, Rect::new(220, 84, 740, 532));
    assert_eq!(rects.detail, Rect::new(960, 48, 320, 728));
    assert_eq!(rects.task_panel, Rect::new(220, 616, 740, 160));
    assert_eq!(rects.status_bar, Rect::new(0, 776, 1280, 24));

    assert_eq!(rects.command_bar.bottom(), rects.nav.y);
    assert_eq!(rects.nav.bottom(), rects.status_bar.y);
    assert_eq!(rects.task_panel.x, rects.nav.right());
    assert_eq!(rects.task_panel.right(), rects.detail.x);
    assert_eq!(rects.task_panel.y, rects.content.bottom());
    assert_eq!(rects.detail.bottom(), rects.status_bar.y);
}

#[test]
fn narrow_and_short_window_collapses_navigation_detail_and_task_panel() {
    let rects = layout(input(1000, 700));

    assert_eq!(rects.nav.w, 48);
    assert_eq!(rects.nav.h, 628);
    assert_eq!(rects.detail, Rect::new(1000, 48, 0, 628));
    assert_eq!(rects.tabs, Rect::new(48, 48, 952, 36));
    assert_eq!(rects.content, Rect::new(48, 84, 952, 560));
    assert_eq!(rects.task_panel, Rect::new(48, 644, 952, 32));
    assert_eq!(rects.status_bar, Rect::new(0, 676, 1000, 24));
}

#[test]
fn minimum_size_keeps_all_zones_inside_client_area() {
    let rects = layout(input(960, 640));

    for rect in [
        rects.command_bar,
        rects.nav,
        rects.tabs,
        rects.content,
        rects.detail,
        rects.task_panel,
        rects.status_bar,
    ] {
        assert!(rect.x >= 0 && rect.y >= 0);
        assert!(rect.right() <= 960);
        assert!(rect.bottom() <= 640);
    }
    assert_eq!(rects.content.x, rects.nav.right());
    assert_eq!(rects.task_panel.x, rects.nav.right());
    assert_eq!(rects.task_panel.bottom(), rects.status_bar.y);
}

#[test]
fn wide_window_keeps_navigation_and_detail_open() {
    let rects = layout(input(1600, 900));

    assert_eq!(rects.nav.w, 220);
    assert_eq!(rects.nav.h, 828);
    assert_eq!(rects.detail.w, 320);
    assert_eq!(rects.detail.h, 828);
    assert_eq!(rects.task_panel, Rect::new(220, 716, 1060, 160));
    assert_eq!(rects.tabs.w, 1060);
}

#[test]
fn dip_to_pixels_rounds_at_target_dpi() {
    assert_eq!(Rect::new(0, 0, 48, 24).scaled(144), Rect::new(0, 0, 72, 36));
    assert_eq!(
        Rect::new(220, 48, 740, 568).scaled(144),
        Rect::new(330, 72, 1110, 852)
    );
    assert_eq!(Rect::new(1, 1, 1, 1).scaled(120), Rect::new(1, 1, 1, 1));
}
