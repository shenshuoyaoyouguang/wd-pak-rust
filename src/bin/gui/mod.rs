//! GUI 子模块：按文件所有权重分，供并行实施。
//!
//! 控件契约：
//! - 每个面板模块只暴露 `create(parent, instance)` / `layout(rect, dpi)` / `refresh(&AppState, …)`。
//! - 不允许在内部直接改写 `AppState`。

pub mod detail;
pub mod dialogs;
pub mod open;
pub mod state;
pub mod util;

pub use state::AppState;

/// 启动 GUI 主循环。
pub fn run() {
    // 暂时委托给旧 spike 入口，待 shell.rs 迁移完成。
    // TODO: 迁移 src/bin/wd-pak-gui.rs 的 mod win 逻辑到 shell.rs
    eprintln!("wd-pak-gui: shell migration pending");
}
EOF