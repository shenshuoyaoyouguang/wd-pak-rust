//! DPI 纯逻辑（G7-02/03）。

pub fn scale_px(value: i32, dpi: u32) -> i32 {
    (value as i64 * dpi as i64 / 96) as i32
}

pub fn unscale_px(value: i32, dpi: u32) -> i32 {
    // dpi 未知（0）时不做缩放：未缩放的输入原样返回。
    if dpi == 0 {
        return value;
    }
    (value as i64 * 96 / dpi as i64) as i32
}

pub use crate::diagnostics::scale_percent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiBreakpoint {
    pub collapsed_nav: bool,
}

pub fn breakpoint_for(width: i32) -> DpiBreakpoint {
    DpiBreakpoint {
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
    fn scale_percent_falls_back_to_100_for_unknown_dpi() {
        assert_eq!(scale_percent(0), 100);
    }

    #[test]
    fn unscale_unknown_dpi_is_identity() {
        assert_eq!(unscale_px(150, 0), 150);
    }

    #[test]
    fn breakpoint() {
        assert!(breakpoint_for(1000).collapsed_nav);
        assert!(!breakpoint_for(1200).collapsed_nav);
    }
}
