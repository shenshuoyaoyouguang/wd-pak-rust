//! 本地持久化配置模型（G0-06）。
//!
//! 本模块只定义结构、默认值、校验与迁移，不触碰磁盘 I/O；调用方负责
//! 读取/写入 JSON 并在损坏或不可写时按本模块提供的回退策略处理。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 配置 schema 版本；迁移逻辑以该字段为依据。
pub const CONFIG_VERSION: u32 = 1;

/// 主题三态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

/// 界面密度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

/// 窗口状态（DIP + 显示器标识，规格 §5.2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    /// 显示器标识（空串表示未记录，回退主屏）。
    pub monitor: String,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: 100,
            y: 100,
            width: 1280,
            height: 800,
            maximized: false,
            monitor: String::new(),
        }
    }
}

/// 设置页六组中的界面相关部分（G7-07）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSettings {
    pub theme: Theme,
    pub density: Density,
    pub nav_collapsed: bool,
    pub restore_session: bool,
    pub recent_limit: u8,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            density: Density::Comfortable,
            nav_collapsed: false,
            restore_session: true,
            recent_limit: 10,
        }
    }
}

impl UiSettings {
    /// 将越界字段夹回合法范围；返回是否发生修正。
    pub fn normalize(&mut self) -> bool {
        let mut changed = false;
        if self.recent_limit > 20 {
            self.recent_limit = 20;
            changed = true;
        }
        changed
    }
}

/// 最近打开条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: String,
    pub pinned: bool,
    /// Unix 秒；0 表示未记录。
    pub last_opened: u64,
}

/// 完整配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub window: WindowState,
    pub ui: UiSettings,
    /// 列宽：列名 -> DIP 宽度。
    pub column_widths: BTreeMap<String, i32>,
    /// 隐藏列键；None 表示使用默认列集。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_columns: Option<Vec<String>>,
    /// 上次导出目录；None 表示尚未导出过。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_export_dir: Option<String>,
    /// 上次打包源目录（G4-03）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pack_dir: Option<String>,
    /// 上次解包输出目录（G4-06）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_unpack_dir: Option<String>,
    /// 详情面板宽度（DIP）。
    pub detail_width: i32,
    /// 任务面板高度（DIP）。
    pub task_panel_height: i32,
    pub recent: Vec<RecentEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            window: WindowState::default(),
            ui: UiSettings::default(),
            column_widths: BTreeMap::new(),
            hidden_columns: None,
            last_export_dir: None,
            last_pack_dir: None,
            last_unpack_dir: None,
            detail_width: 320,
            task_panel_height: 120,
            recent: Vec::new(),
        }
    }
}

/// 配置加载结果：成功或回退默认并附带原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    Ok,
    /// JSON 解析失败或版本不兼容，已回退默认。
    FallbackDefault(String),
    /// 读取阶段不可用（如文件不可读），进入只读模式。
    ReadOnly(String),
}

impl Config {
    /// 从 JSON 文本解析；失败时返回默认配置 + 原因。
    pub fn from_json(text: &str) -> (Self, LoadOutcome) {
        if text.trim().is_empty() {
            return (Self::default(), LoadOutcome::FallbackDefault("空配置".into()));
        }
        match serde_json::from_str::<Config>(text) {
            Ok(mut config) => {
                if config.version != CONFIG_VERSION {
                    // 未知版本：不冒险迁移，回退默认但保留最近打开由调用方决定。
                    return (
                        Self::default(),
                        LoadOutcome::FallbackDefault(format!(
                            "配置版本 {} 与当前 {} 不兼容",
                            config.version, CONFIG_VERSION
                        )),
                    );
                }
                let _ = config.normalize();
                (config, LoadOutcome::Ok)
            }
            Err(err) => (
                Self::default(),
                LoadOutcome::FallbackDefault(format!("JSON 解析失败: {err}")),
            ),
        }
    }

    /// 序列化为 JSON 文本（pretty，便于人工编辑）。
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// 归一化所有字段；返回是否发生修正。
    pub fn normalize(&mut self) -> bool {
        let mut changed = self.ui.normalize();
        if self.detail_width < 160 {
            self.detail_width = 160;
            changed = true;
        }
        if self.task_panel_height < 40 {
            self.task_panel_height = 40;
            changed = true;
        }
        if self.window.width < 960 {
            self.window.width = 960;
            changed = true;
        }
        if self.window.height < 640 {
            self.window.height = 640;
            changed = true;
        }
        // 去重最近打开：同一路径只保留一条，按 last_opened 降序。
        let mut seen = std::collections::BTreeSet::new();
        let mut deduped = Vec::new();
        for entry in self.recent.drain(..) {
            if seen.insert(crate::platform::path_key(std::path::Path::new(&entry.path))) {
                deduped.push(entry);
            }
        }
        deduped.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
        if deduped.len() > self.ui.recent_limit as usize && self.ui.recent_limit > 0 {
            deduped.truncate(self.ui.recent_limit as usize);
        }
        if self.recent != deduped {
            self.recent = deduped;
            changed = true;
        }
        if self.version != CONFIG_VERSION {
            self.version = CONFIG_VERSION;
            changed = true;
        }
        changed
    }

    /// 添加一条最近打开记录；返回是否新增（重复路径会更新时间并置顶）。
    pub fn touch_recent(&mut self, path: &str, now: u64) -> bool {
        if self.ui.recent_limit == 0 {
            return false;
        }
        if let Some(existing) = self.recent.iter_mut().find(|e| crate::platform::path_key(std::path::Path::new(&e.path)) == crate::platform::path_key(std::path::Path::new(path))) {
            existing.last_opened = now;
            // 置顶：移到末尾再排序由 normalize 处理；此处直接重排。
            let mut moved = existing.clone();
            self.recent.retain(|e| crate::platform::path_key(std::path::Path::new(&e.path)) != crate::platform::path_key(std::path::Path::new(path)));
            moved.last_opened = now;
            self.recent.push(moved);
            let _ = self.normalize();
            return false;
        }
        self.recent.push(RecentEntry {
            path: path.to_string(),
            pinned: false,
            last_opened: now,
        });
        let _ = self.normalize();
        true
    }

    /// 移除一条最近打开记录。
    pub fn remove_recent(&mut self, path: &str) {
        self.recent.retain(|e| crate::platform::path_key(std::path::Path::new(&e.path)) != crate::platform::path_key(std::path::Path::new(path)));
    }

    /// 清空最近打开。
    pub fn clear_recent(&mut self) {
        self.recent.clear();
    }
}

/// 固定/取消固定最近打开条目。
pub fn set_recent_pinned(config: &mut Config, path: &str, pinned: bool) -> bool {
    if let Some(entry) = config.recent.iter_mut().find(|e| crate::platform::path_key(std::path::Path::new(&e.path)) == crate::platform::path_key(std::path::Path::new(path))) {
        entry.pinned = pinned;
        true
    } else {
        false
    }
}
