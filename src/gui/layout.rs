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

const DETAIL_BREAKPOINT: i32 = 1100;
const TASK_COLLAPSE_BELOW: i32 = 760;

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
    pub task_panel: Rect,
    pub status_bar: Rect,
}

pub fn layout(input: LayoutInput) -> ShellRects {
    let nav_w = if input.width < DETAIL_BREAKPOINT {
        NAV_COLLAPSED_WIDTH
    } else {
        NAV_WIDTH
    };
    let detail_w = if input.width >= DETAIL_BREAKPOINT {
        DETAIL_WIDTH
    } else {
        0
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

    let task_y = input.height - STATUS_BAR_H - task_h;
    ShellRects {
        command_bar: Rect::new(0, 0, input.width, COMMAND_BAR_H),
        nav: Rect::new(0, COMMAND_BAR_H, nav_w, nav_h),
        tabs: Rect::new(nav_w, COMMAND_BAR_H, center_w, TAB_STRIP_H),
        content: Rect::new(nav_w, COMMAND_BAR_H + TAB_STRIP_H, center_w, content_h),
        detail: Rect::new(input.width - detail_w, COMMAND_BAR_H, detail_w, nav_h),
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
