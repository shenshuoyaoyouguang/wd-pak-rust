//! DPI 纯逻辑（G7-02/03）。

pub fn scale_px(value: i32, dpi: u32) -> i32 {
    (value as i64 * dpi as i64 / 96) as i32
}

pub fn unscale_px(value: i32, dpi: u32) -> i32 {
    (value as i64 * 96 / dpi as i64) as i32
}

pub fn scale_percent(dpi: u32) -> u32 {
    (dpi * 100) / 96
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiBreakpoint {
    pub min_width: i32,
    pub collapsed_nav: bool,
}

pub fn breakpoint_for(width: i32) -> DpiBreakpoint {
    DpiBreakpoint {
        min_width: width,
        collapsed_nav: width < 1100,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_150() {
        assert_eq!(scale_px(100, 144), 150);
    }

    #[test]
    fn scale_percent_150() {
        assert_eq!(scale_percent(144), 150);
    }

    #[test]
    fn breakpoint() {
        assert!(breakpoint_for(1000).collapsed_nav);
        assert!(!breakpoint_for(1200).collapsed_nav);
    }
}
