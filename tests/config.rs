use wd_pak::config::{Config, LoadOutcome, Theme, UiSettings};

#[test]
fn default_config_is_valid() {
    let c = Config::default();
    assert_eq!(c.version, 1);
    assert_eq!(c.ui.recent_limit, 10);
    assert!(c.recent.is_empty());
    assert_eq!(c.window.width, 1280);
}

#[test]
fn from_json_roundtrip() {
    let mut c = Config::default();
    c.ui.theme = Theme::Dark;
    c.ui.nav_collapsed = true;
    c.touch_recent("a.pak", 42);
    let json = c.to_json();
    let (parsed, outcome) = Config::from_json(&json);
    assert_eq!(outcome, LoadOutcome::Ok);
    assert_eq!(parsed.ui.theme, Theme::Dark);
    assert!(parsed.ui.nav_collapsed);
    assert_eq!(parsed.recent.len(), 1);
    assert_eq!(parsed.recent[0].path, "a.pak");
}

#[test]
fn from_json_empty_falls_back() {
    let (c, outcome) = Config::from_json("");
    assert_eq!(c, Config::default());
    assert!(matches!(outcome, LoadOutcome::FallbackDefault(_)));
}

#[test]
fn from_json_invalid_falls_back() {
    let (c, outcome) = Config::from_json("{not json");
    assert_eq!(c, Config::default());
    assert!(matches!(outcome, LoadOutcome::FallbackDefault(_)));
}

#[test]
fn from_json_wrong_version_falls_back() {
    let json = r#"{"version":999,"window":{"x":0,"y":0,"width":1280,"height":800,"maximized":false,"monitor":""},"ui":{"theme":"system","density":"comfortable","nav_collapsed":false,"restore_session":true,"recent_limit":10},"column_widths":{},"detail_width":320,"task_panel_height":120,"recent":[]}"#;
    let (c, outcome) = Config::from_json(json);
    assert_eq!(c, Config::default());
    assert!(matches!(outcome, LoadOutcome::FallbackDefault(_)));
}

#[test]
fn normalize_clamps_recent_limit_and_dimensions() {
    let mut c = Config::default();
    c.ui.recent_limit = 50;
    c.window.width = 100;
    c.window.height = 100;
    c.detail_width = 10;
    c.task_panel_height = 1;
    let changed = c.normalize();
    assert!(changed);
    assert_eq!(c.ui.recent_limit, 20);
    assert_eq!(c.window.width, 960);
    assert_eq!(c.window.height, 640);
    assert_eq!(c.detail_width, 160);
    assert_eq!(c.task_panel_height, 40);
}

#[test]
fn touch_recent_dedupes_and_limits() {
    let mut c = Config::default();
    c.ui.recent_limit = 2;
    c.touch_recent("a.pak", 1);
    c.touch_recent("b.pak", 2);
    c.touch_recent("a.pak", 3); // update + reorder
    assert_eq!(c.recent.len(), 2);
    assert_eq!(c.recent[0].path, "a.pak");
    assert_eq!(c.recent[0].last_opened, 3);
    c.touch_recent("c.pak", 4);
    assert_eq!(c.recent.len(), 2);
    assert!(!c.recent.iter().any(|e| e.path == "b.pak"));
}

#[test]
fn recent_limit_zero_disables_recording() {
    let mut c = Config::default();
    c.ui.recent_limit = 0;
    assert!(!c.touch_recent("a.pak", 1));
    assert!(c.recent.is_empty());
}

#[test]
fn remove_and_clear_recent() {
    let mut c = Config::default();
    c.touch_recent("a.pak", 1);
    c.touch_recent("b.pak", 2);
    c.remove_recent("a.pak");
    assert_eq!(c.recent.len(), 1);
    assert_eq!(c.recent[0].path, "b.pak");
    c.clear_recent();
    assert!(c.recent.is_empty());
}

#[test]
fn ui_settings_normalize_clamps() {
    let mut s = UiSettings {
        recent_limit: 100,
        ..UiSettings::default()
    };
    assert!(s.normalize());
    assert_eq!(s.recent_limit, 20);
}

#[test]
fn set_recent_pinned_works() {
    use wd_pak::config::set_recent_pinned;
    let mut c = Config::default();
    c.touch_recent("a.pak", 1);
    assert!(set_recent_pinned(&mut c, "a.pak", true));
    assert!(c.recent[0].pinned);
    assert!(!set_recent_pinned(&mut c, "nope", true));
}

#[test]
fn pinned_recent_survives_limit_when_others_touched() {
    use wd_pak::config::set_recent_pinned;
    let mut c = Config::default();
    c.ui.recent_limit = 2;
    c.touch_recent("a.pak", 1);
    assert!(set_recent_pinned(&mut c, "a.pak", true));

    c.touch_recent("b.pak", 2);
    c.touch_recent("c.pak", 3);

    assert_eq!(c.recent.len(), 2);
    assert!(
        c.recent.iter().any(|e| e.path == "a.pak" && e.pinned),
        "固定项不应被较新的未固定项淘汰: {:?}",
        c.recent
    );
}
