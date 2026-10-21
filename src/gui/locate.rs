//! 深层定位状态模型（G2-06）。
//!
//! 只定义定位后的高亮状态，不涉及 Win32 控件。

/// 高亮状态：定位后高亮 N 秒。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightState {
    pub path: String,
    pub remaining_ms: u32,
}

impl HighlightState {
    pub fn new(path: &str, duration_ms: u32) -> Self {
        Self {
            path: path.to_string(),
            remaining_ms: duration_ms,
        }
    }

    pub fn tick(&mut self, elapsed_ms: u32) {
        self.remaining_ms = self.remaining_ms.saturating_sub(elapsed_ms);
    }

    pub fn is_active(&self) -> bool {
        self.remaining_ms > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_ticks_down() {
        let mut h = HighlightState::new("x.txt", 2000);
        assert!(h.is_active());
        h.tick(500);
        assert!(h.is_active());
        assert_eq!(h.remaining_ms, 1500);
        h.tick(2000);
        assert!(!h.is_active());
        assert_eq!(h.remaining_ms, 0);
    }
}
