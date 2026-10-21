//! 账户/关于纯逻辑（G7-08）。

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutInfo {
    pub app_name: String,
    pub version: String,
    pub build: String,
    pub system: String,
    pub dpi: u32,
    pub path_support: bool,
}

impl AboutInfo {
    pub fn new(version: &str, dpi: u32, path_support: bool) -> Self {
        Self {
            app_name: "wd-pak".into(),
            version: version.to_string(),
            build: "release".into(),
            system: "Windows".into(),
            dpi,
            path_support,
        }
    }

    pub fn summary(&self) -> String {
        format!("{} {} ({})", self.app_name, self.version, self.build)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccountProfile {
    pub name: String,
    pub team: String,
    pub note: String,
}

impl AccountProfile {
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            "未设置".into()
        } else {
            self.name.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_summary() {
        let a = AboutInfo::new("1.0", 144, true);
        assert_eq!(a.summary(), "wd-pak 1.0 (release)");
        assert_eq!(a.dpi, 144);
        assert!(a.path_support);
    }

    #[test]
    fn account_display_unset_shows_local_placeholder() {
        let p = AccountProfile::default();
        assert_eq!(p.display_name(), "未设置");
    }

    #[test]
    fn account_display_uses_operator_name() {
        let p = AccountProfile {
            name: "张三".into(),
            ..Default::default()
        };
        assert_eq!(p.display_name(), "张三");
    }
}
