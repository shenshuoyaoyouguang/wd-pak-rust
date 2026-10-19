//! 状态栏状态模型（G2-07）。
//!
//! 只定义状态栏的文本投影与校验徽标状态，不涉及 Win32 控件。

use crate::gui::states::ValidationState;

/// 校验徽标四态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationBadge {
    Unchecked,
    Passed,
    Problems(usize),
    Expired,
}

impl Default for ValidationBadge {
    fn default() -> Self {
        ValidationBadge::Unchecked
    }
}

impl ValidationBadge {
    pub fn from_state(state: &ValidationState) -> Self {
        match state {
            ValidationState::Unchecked | ValidationState::Running | ValidationState::Cancelled => {
                ValidationBadge::Unchecked
            }
            ValidationState::Passed => ValidationBadge::Passed,
            ValidationState::Problems(n) => ValidationBadge::Problems(*n),
            ValidationState::Expired => ValidationBadge::Expired,
        }
    }

    pub fn label(&self) -> String {
        match self {
            ValidationBadge::Unchecked => "未校验".into(),
            ValidationBadge::Passed => "校验通过".into(),
            ValidationBadge::Problems(n) => format!("发现问题 {n}"),
            ValidationBadge::Expired => "结果已过期".into(),
        }
    }
}

/// 状态栏三段信息。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusBarState {
    pub path: String,
    pub files: usize,
    pub dirs: usize,
    pub stored: u64,
    pub org: u64,
    pub ratio: String,
    pub selected: String,
    pub validation: ValidationBadge,
    pub task_count: usize,
}

impl StatusBarState {
    /// 中间省略的路径（超过 limit 字符时中间省略）。
    pub fn short_path(&self, limit: usize) -> String {
        let chars: Vec<char> = self.path.chars().collect();
        if chars.len() <= limit || limit < 8 {
            return self.path.clone();
        }
        let head = limit / 2 - 1;
        let tail = limit - head - 1;
        format!(
            "{}…{}",
            chars[..head].iter().collect::<String>(),
            chars[chars.len() - tail..].iter().collect::<String>()
        )
    }

    /// 完整状态文本（三段）。
    pub fn full_text(&self) -> String {
        format!(
            "{} | {} 个文件 / {} 个目录 | 存储 {} / 原始 {}，存储比 {} | {}",
            self.short_path(60),
            self.files,
            self.dirs,
            self.stored,
            self.org,
            self.ratio,
            self.selected
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_from_validation_state() {
        assert_eq!(
            ValidationBadge::from_state(&ValidationState::Passed),
            ValidationBadge::Passed
        );
        assert_eq!(
            ValidationBadge::from_state(&ValidationState::Problems(3)),
            ValidationBadge::Problems(3)
        );
        assert_eq!(
            ValidationBadge::from_state(&ValidationState::Expired),
            ValidationBadge::Expired
        );
    }

    #[test]
    fn badge_label() {
        assert_eq!(ValidationBadge::Problems(5).label(), "发现问题 5");
        assert_eq!(ValidationBadge::Expired.label(), "结果已过期");
    }

    #[test]
    fn short_path_omits_middle() {
        let state = StatusBarState {
            path: "C:\\very\\long\\path\\to\\some\\pak\\file.pak".into(),
            ..Default::default()
        };
        let short = state.short_path(20);
        assert!(short.contains('…'));
        assert!(short.chars().count() <= 20);
    }

    #[test]
    fn short_path_no_omit_when_short() {
        let state = StatusBarState {
            path: "short.pak".into(),
            ..Default::default()
        };
        assert_eq!(state.short_path(60), "short.pak");
    }

    #[test]
    fn full_text_contains_three_segments() {
        let state = StatusBarState {
            path: "test.pak".into(),
            files: 16,
            dirs: 0,
            stored: 100,
            org: 200,
            ratio: "0.500".into(),
            selected: "file.txt".into(),
            ..Default::default()
        };
        let text = state.full_text();
        assert!(text.contains("test.pak"));
        assert!(text.contains("16 个文件"));
        assert!(text.contains("file.txt"));
    }
}
