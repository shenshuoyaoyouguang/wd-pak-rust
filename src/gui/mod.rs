//! 图形界面实现（G0 起）。
//!
//! 与平台无关的布局逻辑放在本模块；会话、浏览、最近打开与工作区状态模块
//! 直接挂在 crate 根，窗口与系统 API 留在 GUI 可执行目标。

pub mod layout;
pub mod states;          // G2.5 状态机
pub mod focus;           // G0-04 焦点与键盘基线
pub mod task_panel;      // G0-09 任务面板骨架
pub mod status_bar;      // G2-07 状态栏
pub mod theme;           // G0-03 主题与视觉基线
pub mod workbench;       // G1-01 工作台与启动页
pub mod search;          // G2-05 搜索与筛选
pub mod self_check;      // G0-07 环境自检
pub mod locate;          // G2-06 深层定位
pub mod nav;             // G0-02 导航模型
