# AGENTS.md

问道服务端 PAK 打包/解包工具（Rust 版）。对 `../wd_pak_tool.py`（Python 参考实现）的逐字节兼容重写：PAK 二进制、LZSS 压缩流、`_pak_manifest.json`、CLI 消息文本、退出码均不得偏离。


## 架构

- 库层 `src/lib.rs` 集中声明所有模块，`src/main.rs`（CLI）只做参数解析/分发；集成测试只链接库 target，新增公共模块必须从 `lib.rs` 暴露。
- 格式层：`format.rs`（Header 16B / Catalog 64B / 常量）、`tree.rs`（目录树，深度 64、循环检测）、`pak.rs`（读取器）、`lzss.rs`（Okumura 变体，环形窗口 4096、匹配长 3..=18）、`manifest.rs`。`pak.rs`/`lzss.rs` 头部注释是逐条对齐 Python 的语义清单，改动前先读。
- 命令层：`commands_read.rs`（list/find/extract/cat/check）、`commands_write.rs`（pack/replace）、`interactive.rs`（无子命令时的 REPL）、`error.rs`（`PakError` 消息与 Python 逐字对齐，含 CLI/交互模式 `错误:`/`出错:` 分支差异）。
- GUI 二进制 `src/bin/wd-pak-gui.rs` + `src/bin/gui/`：Win32 原生控件 + `windows-sys`（无第三方 UI 框架）。逻辑与渲染分离，可测逻辑放库层（`browse/`，拆分自 `browse.rs`；`gui/`、`session.rs`、`workspace.rs`、`recent.rs`）与 `tests/gui_layout.rs` 等纯逻辑测试。
- OS 相关 I/O 一律走 `platform/`（`io_message`/`normpath`/`stdout`/`read_line` 等），不要在其他模块加 `cfg(windows)`；Windows 错误文案是 `[Win32 n] 固定文案`，与 Unix 的 `[Errno n] strerror` 不同，属预期。
- `--jobs` 实际并行上限 8（`min(CPU 核数, 8)` 的双实现）；`replace --backup` 固定生成 `<pak>.bak`（覆盖同名），写入统一经 `.tmp` 改名。

## 不可偏离的格式红线

- 名字：Latin-1 原始字节，最长 **43 字节**，禁 `/\NUL`，拒 `.`/`..`；比较用字节，不 decode UTF-8。
- attrib：bit0=目录，bit31=RAW 未压缩；**没有独立的 compressed 字段**。RAW 数据是 `stored[:org]` 截断，不是解压。
- `_pak_manifest.json`：extract 生成、pack 跳过并回放（order 排序 + mtime 还原）；pack 的 files 键序依赖 `serde_json/preserve_order`（Cargo.toml 注释说明，勿关闭）。
- Python 原版怪癖（如相邻空目录共享 offset 导致 TreeCycle 误报）按"逐字节一致优先"复刻，不修——判定标准是金样测试（`tests/golden/`，10744 条 SHA256），不是"逻辑合理性"。
- 单文件 size/org 或整包超 4 GB 拒绝；同层大小写冲突 pack 时阻断（extract 不检）。

## 文档

- `README.md` 用法/兼容性承诺；`MANUAL.md` 部署、多平台编译与文档索引；`BENCHMARK.md` NFR-01~05 实测口径。
- README/MANUAL 引用的 `design.md`（含 2.10 怪癖、2.11 资源边界）与 `spec.md` 位于 `../.codeartsdoer/specs/wd-pak-rust/`，本机不存在；本地 `docs/` 只覆盖 Windows 平台（M0–M4）与 GUI（G0–G7）规划。
