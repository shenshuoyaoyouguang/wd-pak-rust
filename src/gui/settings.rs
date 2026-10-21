//! 设置纯逻辑（G7-07）。

use crate::gui::pack::default_jobs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub restore_session: bool,
    pub minimize_to_tray: bool,
    pub log_limit: usize,
    pub log_days: usize,
    pub default_compress: bool,
    pub jobs: usize,
    pub locale: String,
    pub font_scale: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            restore_session: true,
            minimize_to_tray: false,
            log_limit: 100,
            log_days: 30,
            default_compress: true,
            jobs: default_jobs(),
            locale: "system".into(),
            font_scale: 100,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        if self.jobs == 0 || self.jobs > 8 {
            errs.push("jobs 必须在 1..=8".into());
        }
        if self.font_scale < 100 || self.font_scale > 150 {
            errs.push("字体缩放必须在 100..=150".into());
        }
        if self.log_limit == 0 {
            errs.push("日志上限不能为 0".into());
        }
        errs
    }

    pub fn reset_to_defaults(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_valid() {
        assert!(Settings::default().validate().is_empty());
    }

    #[test]
    fn default_minimize_to_tray_off() {
        // spec: 关闭主窗口时最小化到托盘，默认关
        assert!(!Settings::default().minimize_to_tray);
    }

    #[test]
    fn default_jobs_is_min_cpu_8() {
        // spec: 默认并行度 min(CPU, 8)
        let expected = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(8);
        assert_eq!(Settings::default().jobs, expected);
    }

    #[test]
    fn font_scale_range_is_100_to_150() {
        let with_scale = |font_scale| Settings {
            font_scale,
            ..Settings::default()
        };
        assert!(with_scale(100).validate().is_empty());
        assert!(with_scale(150).validate().is_empty());
        assert!(!with_scale(99).validate().is_empty());
        assert!(!with_scale(151).validate().is_empty());
    }

    #[test]
    fn invalid_jobs() {
        let settings = Settings {
            jobs: 0,
            ..Settings::default()
        };
        assert!(!settings.validate().is_empty());
    }
}
