//! wd-pak 库层：供 bin 入口（src/main.rs）与集成测试（tests/）共用。
//! 注意：模块声明集中在本文件而非 main.rs，因为 tests/ 下的集成测试
//! 只能链接库 target，后续 lzss/tree/pak 的 golden 测试同样依赖此处。
//!
//! 模块规划（后续任务逐步补充，均为库内公开模块）：
//!   mod lzss;           // LZSS 压缩/解压（Okumura 变体，窗口 4096/最大 18 字节匹配）
//!   mod tree;           // 目录树解析（深度 64 / 循环检测 / u64 越界检查）
//!   mod pak;            // PAK 读取器（walk / find / read_file / read_stored）
//!   mod manifest;       // _pak_manifest.json 读写
//!   mod commands_read;  // list / find / extract / cat / check
//!   mod commands_write; // pack / replace
//!   mod interactive;    // 交互模式

pub mod error;
pub mod format;
pub mod platform;
pub mod interactive; // 任务 7
pub mod lzss;
pub mod pak;
pub mod tree;pub mod manifest;        // 任务 5
pub mod commands_read;   // 任务 5
pub mod commands_write;  // 任务 6