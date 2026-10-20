//! 主窗口五区布局（G0-01）。
//!
//! 输入是 DIP 尺寸与折叠状态，输出各区域矩形；不调用任何窗口 API，
//! 便于在不同 DPI 与断点下单独验证。

pub const MIN_WIDTH: i32 = 960;
pub const MIN_HEIGHT: i32 = 640;

pub const COMMAND_BAR_H: i32 = 48;
pub const TAB_STRIP_H: i32 = 36;
pub const STATUS_BAR_H: i32 = 24;
pub const NAV_WIDTH: i32 = 220;
pub const NAV_COLLAPSED_WIDTH: i32 = 48;
pub const DETAIL_WIDTH: i32 = 320;
pub const TASK_PANEL_H: i32 = 160;
pub const TASK_COLLAPSED_H: i32 = 32;

pub const STANDARD_MIN_WIDTH: i32 = 1100;
pub const WIDE_MIN_WIDTH: i32 = 1440;
const TASK_COLLAPSE_BELOW: i32 = 760;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutTier {
    Compact,
    Standard,
    Wide,
}

pub fn tier(width: i32) -> LayoutTier {
    if width >= WIDE_MIN_WIDTH {
        LayoutTier::Wide
    } else if width >= STANDARD_MIN_WIDTH {
        LayoutTier::Standard
    } else {
        LayoutTier::Compact
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// 按 DPI 换算；先换算四边再求宽高，相邻区域不会因两次取整而错开 1px。
    pub fn scaled(&self, dpi: u32) -> Self {
        let (x, y) = (scale(self.x, dpi), scale(self.y, dpi));
        Self {
            x,
            y,
            w: scale(self.right(), dpi) - x,
            h: scale(self.bottom(), dpi) - y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutInput {
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellRects {
    pub command_bar: Rect,
    pub nav: Rect,
    pub tabs: Rect,
    pub content: Rect,
    pub detail: Rect,
    pub detail_overlay: Option<Rect>,
    pub task_panel: Rect,
    pub status_bar: Rect,
}

pub fn layout(input: LayoutInput) -> ShellRects {
    let layout_tier = tier(input.width);
    let (nav_w, detail_w) = match layout_tier {
        LayoutTier::Compact => (NAV_COLLAPSED_WIDTH, 0),
        LayoutTier::Standard | LayoutTier::Wide => (NAV_WIDTH, DETAIL_WIDTH),
    };
    let task_h = if input.height < TASK_COLLAPSE_BELOW {
        TASK_COLLAPSED_H
    } else {
        TASK_PANEL_H
    };

    let body_h = (input.height - COMMAND_BAR_H - STATUS_BAR_H - task_h).max(0);
    let nav_h = (input.height - COMMAND_BAR_H - STATUS_BAR_H).max(0);
    let center_w = (input.width - nav_w - detail_w).max(0);
    let content_h = (body_h - TAB_STRIP_H).max(0);
    let content_y = COMMAND_BAR_H + TAB_STRIP_H;
    let detail_overlay = match layout_tier {
        LayoutTier::Compact => Some(Rect::new(
            (input.width - DETAIL_WIDTH).max(nav_w),
            content_y,
            DETAIL_WIDTH,
            content_h,
        )),
        LayoutTier::Standard | LayoutTier::Wide => None,
    };

    let task_y = input.height - STATUS_BAR_H - task_h;
    ShellRects {
        command_bar: Rect::new(0, 0, input.width, COMMAND_BAR_H),
        nav: Rect::new(0, COMMAND_BAR_H, nav_w, nav_h),
        tabs: Rect::new(nav_w, COMMAND_BAR_H, center_w, TAB_STRIP_H),
        content: Rect::new(nav_w, content_y, center_w, content_h),
        detail: Rect::new(input.width - detail_w, COMMAND_BAR_H, detail_w, nav_h),
        detail_overlay,
        task_panel: Rect::new(nav_w, task_y, center_w, task_h),
        status_bar: Rect::new(0, input.height - STATUS_BAR_H, input.width, STATUS_BAR_H),
    }
}

pub fn scale(value: i32, dpi: u32) -> i32 {
    if dpi == 0 {
        return value;
    }
    (value as f64 * dpi as f64 / 96.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(width: i32, height: i32) -> LayoutInput {
        LayoutInput { width, height }
    }

    #[test]
    fn tier_boundaries_match_spec() {
        assert_eq!(tier(960), LayoutTier::Compact);
        assert_eq!(tier(1099), LayoutTier::Compact);
        assert_eq!(tier(1100), LayoutTier::Standard);
        assert_eq!(tier(1439), LayoutTier::Standard);
        assert_eq!(tier(1440), LayoutTier::Wide);
        assert_eq!(tier(1920), LayoutTier::Wide);
    }

    #[test]
    fn compact_tier_collapses_nav_and_uses_detail_overlay() {
        let rects = layout(input(1000, 700));

        assert_eq!(rects.nav.w, NAV_COLLAPSED_WIDTH);
        assert_eq!(rects.detail, Rect::new(1000, COMMAND_BAR_H, 0, 628));
        assert_eq!(
            rects.detail_overlay,
            Some(Rect::new(680, COMMAND_BAR_H + TAB_STRIP_H, DETAIL_WIDTH, 560))
        );
        assert_eq!(rects.content.w, 952);
    }

    #[test]
    fn layout_switches_geometry_at_breakpoints() {
        let compact = layout(input(1099, 800));
        assert_eq!(compact.nav.w, NAV_COLLAPSED_WIDTH);
        assert_eq!(compact.detail.w, 0);
        assert!(compact.detail_overlay.is_some());

        let standard = layout(input(1100, 800));
        assert_eq!(standard.nav.w, NAV_WIDTH);
        assert_eq!(standard.detail.w, DETAIL_WIDTH);
        assert_eq!(standard.detail_overlay, None);

        let wide = layout(input(1440, 800));
        assert_eq!(wide.nav.w, NAV_WIDTH);
        assert_eq!(wide.detail.w, DETAIL_WIDTH);
        assert_eq!(wide.detail_overlay, None);
    }

    #[test]
    fn standard_tier_keeps_nav_and_detail_without_overlay() {
        let rects = layout(input(1280, 800));

        assert_eq!(rects.nav.w, NAV_WIDTH);
        assert_eq!(rects.detail.w, DETAIL_WIDTH);
        assert_eq!(rects.detail_overlay, None);
    }

    #[test]
    fn wide_tier_keeps_three_columns() {
        let rects = layout(input(1600, 900));

        assert_eq!(rects.nav.w, NAV_WIDTH);
        assert_eq!(rects.detail.w, DETAIL_WIDTH);
        assert_eq!(rects.detail_overlay, None);
    }
}
