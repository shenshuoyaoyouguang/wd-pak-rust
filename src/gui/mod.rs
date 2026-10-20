//! 图形界面实现（G0 起）。
//!
//! 与平台无关的布局逻辑放在本模块；会话、浏览、最近打开与工作区状态模块
//! 直接挂在 crate 根，窗口与系统 API 留在 GUI 可执行目标。

pub mod layout;
pub mod persist;         // G0-06 配置与会话快照读写
pub mod error_text;      // G0-阶段1 错误文案映射
pub mod text;            // G0-阶段1 文本投影
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
pub mod export;          // G3 导出与预览纯逻辑
pub mod preview;         // G3 预览别名
pub mod replace;         // G4 替换纯逻辑
pub mod pack;            // G4 打包纯逻辑
pub mod unpack;          // G4 解包纯逻辑
pub mod check;           // G5 校验纯逻辑
pub mod validation;      // G5 校验别名
pub mod task_log;        // G5-06 任务日志纯逻辑
pub mod log;             // G5-06 日志别名
pub mod menu;            // G6 菜单/命令栏纯逻辑
pub mod shortcuts;       // G6 快捷键纯逻辑
pub mod dragdrop;        // G6 拖放纯逻辑
pub mod dpi;             // G7 DPI 纯逻辑
pub mod settings;        // G7 设置纯逻辑
pub mod account;         // G7 账户/关于纯逻辑
pub mod help;            // G7 帮助纯逻辑
