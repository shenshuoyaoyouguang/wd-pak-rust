//! 主题与视觉基线（G0-03）。
//!
//! 只定义主题解析与语义色映射，不涉及 Win32 API。

/// 用户选择的主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    Light,
    Dark,
    #[default]
    System,
}

/// 系统实际主题（由 OS 提供）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTheme {
    Light,
    Dark,
}

/// 有效主题（解析后的最终主题）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveTheme {
    Light,
    Dark,
}

impl ThemeChoice {
    /// 根据用户选择与系统主题解析最终有效主题。
    pub fn resolve(self, system: SystemTheme) -> EffectiveTheme {
        match self {
            ThemeChoice::Light => EffectiveTheme::Light,
            ThemeChoice::Dark => EffectiveTheme::Dark,
            ThemeChoice::System => match system {
                SystemTheme::Light => EffectiveTheme::Light,
                SystemTheme::Dark => EffectiveTheme::Dark,
            },
        }
    }
}

/// 语义色角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticColor {
    Window,
    WindowText,
    ButtonFace,
    ButtonText,
    Highlight,
    HighlightText,
    Warning,
    Error,
    Success,
}

/// 语义色在亮/暗主题下的 RGB 值（简化版，实际由 Win32 系统色提供）。
impl SemanticColor {
    pub fn rgb(self, theme: EffectiveTheme) -> (u8, u8, u8) {
        match (self, theme) {
            (SemanticColor::Window, EffectiveTheme::Light) => (255, 255, 255),
            (SemanticColor::Window, EffectiveTheme::Dark) => (32, 32, 32),
            (SemanticColor::WindowText, EffectiveTheme::Light) => (0, 0, 0),
            (SemanticColor::WindowText, EffectiveTheme::Dark) => (240, 240, 240),
            (SemanticColor::Warning, _) => (255, 193, 7),
            (SemanticColor::Error, _) => (220, 53, 69),
            (SemanticColor::Success, _) => (40, 167, 69),
            _ => (128, 128, 128),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_light() {
        assert_eq!(
            ThemeChoice::Light.resolve(SystemTheme::Dark),
            EffectiveTheme::Light
        );
    }

    #[test]
    fn resolve_dark() {
        assert_eq!(
            ThemeChoice::Dark.resolve(SystemTheme::Light),
            EffectiveTheme::Dark
        );
    }

    #[test]
    fn resolve_system_follows_os() {
        assert_eq!(
            ThemeChoice::System.resolve(SystemTheme::Light),
            EffectiveTheme::Light
        );
        assert_eq!(
            ThemeChoice::System.resolve(SystemTheme::Dark),
            EffectiveTheme::Dark
        );
    }

    #[test]
    fn semantic_color_differs_by_theme() {
        let light = SemanticColor::Window.rgb(EffectiveTheme::Light);
        let dark = SemanticColor::Window.rgb(EffectiveTheme::Dark);
        assert_ne!(light, dark);
    }

    #[test]
    fn warning_color_same_in_both_themes() {
        let light = SemanticColor::Warning.rgb(EffectiveTheme::Light);
        let dark = SemanticColor::Warning.rgb(EffectiveTheme::Dark);
        assert_eq!(light, dark);
    }
}
