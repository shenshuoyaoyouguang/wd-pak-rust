//! 焦点与键盘基线（G0-04）。
//!
//! 只定义焦点区域顺序与 F6 循环逻辑，不涉及 Win32 API。

/// 界面中的焦点区域，顺序与视觉布局一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FocusZone {
    CommandBar,
    Nav,
    Tabs,
    Content,
    Detail,
    TaskPanel,
    StatusBar,
}

impl FocusZone {
    /// 按视觉顺序排列的区域列表（F6 循环顺序）。
    pub const ORDER: [FocusZone; 7] = [
        FocusZone::CommandBar,
        FocusZone::Nav,
        FocusZone::Tabs,
        FocusZone::Content,
        FocusZone::Detail,
        FocusZone::TaskPanel,
        FocusZone::StatusBar,
    ];

    /// 返回下一个区域（F6）。
    pub fn next(self) -> Self {
        let pos = Self::ORDER.iter().position(|&z| z == self).unwrap_or(0);
        Self::ORDER[(pos + 1) % Self::ORDER.len()]
    }

    /// 返回上一个区域（Shift+F6）。
    pub fn prev(self) -> Self {
        let pos = Self::ORDER.iter().position(|&z| z == self).unwrap_or(0);
        if pos == 0 {
            Self::ORDER[Self::ORDER.len() - 1]
        } else {
            Self::ORDER[pos - 1]
        }
    }

    /// 该区域是否可接收焦点（窄屏下部分区域可能不可见）。
    pub fn is_available(self, width: i32, _height: i32) -> bool {
        match self {
            FocusZone::Nav => true,
            FocusZone::Detail => width >= 1100,
            FocusZone::TaskPanel => true,
            _ => true,
        }
    }
}

/// F6 循环：跳过不可用区域。
pub fn cycle(start: FocusZone, forward: bool, width: i32, height: i32) -> FocusZone {
    let mut current = start;
    for _ in 0..FocusZone::ORDER.len() {
        current = if forward { current.next() } else { current.prev() };
        if current.is_available(width, height) {
            return current;
        }
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_wraps_around() {
        assert_eq!(FocusZone::StatusBar.next(), FocusZone::CommandBar);
    }

    #[test]
    fn prev_wraps_around() {
        assert_eq!(FocusZone::CommandBar.prev(), FocusZone::StatusBar);
    }

    #[test]
    fn cycle_skips_unavailable_detail_on_narrow() {
        let result = cycle(FocusZone::Content, true, 1000, 800);
        assert_ne!(result, FocusZone::Detail);
    }

    #[test]
    fn cycle_forward_from_command_bar() {
        let result = cycle(FocusZone::CommandBar, true, 1280, 800);
        assert_eq!(result, FocusZone::Nav);
    }
}
