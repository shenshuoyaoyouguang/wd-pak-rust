# wd-pak GUI 落地实施计划（差距驱动）

> **来源**：`bg_3a6d9f7a`（计划 vs 实现差距分析）+ `bg_37608775`（Win32 外壳拆分方案）
> **基线**：`main @ 35ce007` + 工作区 WIP（3 个已修改文件 + 17 个未跟踪文件）
> **性质**：本文档只做规划，不含实现。所有 commit 需你确认后执行（未经请求不提交）。
> **验证状态**：标注 ✅ 的结论已由编排者独立复核；标注 🔎 的来自子代理分析，实施时需再确认。

---

## 0. 一句话目标

把 GUI 从 **「单文件 spike（1410 行）+ 死代码（742 行）+ 未接线纯逻辑（3105 行）」** 推进到 **「按 G0–G7 分层、可测试、可维护」**。

---

## 1. 基线事实

### 1.1 已交叉验证 ✅

| 事实 | 证据 |
| :--- | :--- |
| 可运行 GUI 只有一个文件 | `src/bin/wd-pak-gui.rs`（1410 行），`Cargo.toml:25-27` 唯一指定 |
| `src/bin/gui/` 6 文件 742 行**从未编译** | 全仓无 `mod gui;`；`src/bin/` 下无 `main.rs`；`wd-pak-gui.rs:8` 只有 `mod win {` |
| `src/bin/gui/mod.rs:21` 是裸 `EOF` | 文件末尾字节为 `…}\nEOF`，一旦接线即语法错误 |
| `src/gui/` 27 模块中**只有 `layout` 被 GUI 消费** | 外部引用仅 `layout`（bin+tests）与 `states`（仅 tests），其余 0 |
| shell 只 import 4 个库模块 | `wd-pak-gui.rs:14-19`：`browse` / `error` / `gui::layout` / `session` |
| G3/G4/G5 界面为零 | `wd-pak-gui.rs` 全文无导出/替换/打包/解包/校验界面 |

### 1.2 来自分析（实施时复核）🔎

| 项 | 值 |
| :--- | :--- |
| `src/gui/` 纯逻辑规模 | 3105 行 / 27 模块（24 真实 + 3 别名空壳） |
| 子任务完成度 | 65 个子任务，**0 个通过验收**；有界面切片 ≈8 个（≈12%） |
| 规格冲突 | 13 处（D1–D13） |
| 空转枚举变体 | 6 个（`ManifestMtime`/`NotWritable`/`CaseConflict`/`TargetExists`/`DuplicateName`/`Cycle`/`BadName`） |
| 测试覆盖 | shell **0 测试**；纯逻辑 72 内联 + 21 集成 |

---

## 2. 先决策（阻塞项，必须最先做）

### D0.1 — `src/bin/gui/` 的处置

| 选项 | 做法 | 优点 | 缺点 |
| :--- | :--- | :--- | :--- |
| **A（推荐）** | **删除** 6 个文件 | 消除"看起来完成了一半"的误导；无编译负担 | 丢弃 `dialogs.rs` 里更完整的 Esc/Tab 交互（可另存参考） |
| B | 保留并接线 | 复用 742 行 | 需先修 `EOF`；与外壳有 3 处行为漂移；`state.rs` 字段对不上 `Ui` |

> 推荐 A。理由：它是**平行未完成实现**，不是外壳的复制（见拆分方案 §2）。若选 B，必须先补 §5 的 3 处漂移。

### D0.2 — 是否纳入 Momus 评审

本计划是对既有 `docs/windows-ui-impl-plan.md` 的**差距驱动细化**，可选择：
- 直接执行（推荐，因已有两份独立分析交叉验证）
- 或先交 Momus 评审一轮

---

## 3. 阶段总览

| 阶段 | 目标 | 提交数 | 依赖 |
| :--- | :--- | :--- | :--- |
| **0** | 基线固化 | 1 | — |
| **1** | 库层纯逻辑外提（方案 C） | 4–5 | 0 |
| **2** | bin 内模块拆分（方案 A 阶段2） | 12–13 | 0, 1 |
| **3** | 功能接入 G3→G4→G5 | 6 | 2 |
| **4** | 规格冲突修复 D1–D13 | 4 | 1（可并行） |
| **5** | G6/G7 收尾 | 2–3 | 3 |

**推荐路径**：0 → 1 → 2 → 3 → 4 → 5
**最小路径**：0 → 1 →（暂停验证）→ 再决定 2

---

## 4. 阶段详述

### 阶段 0 — 基线固化

| # | Commit | 内容 | 验证 |
| :--- | :--- | :--- | :--- |
| C0.1 | `chore(gui): 固化 GUI 工作区基线` | `git add` 3 个已修改 + 17 个未跟踪文件（含 `AGENTS.md`、`src/gui/*.rs`、`src/bin/wd-pak-gui.rs`）；提交 | `git status` 干净；`cargo check --all-targets` 通过 |

**为什么先做**：后续拆分要用 `git diff -M` 证明"纯搬移"，没有干净基线就无法判定。

---

### 阶段 1 — 库层纯逻辑外提（方案 C，最高性价比）

> 原则：**只移动 + 调整可见性，不改逻辑**。时间格式化（`mtime_local`）因依赖 Win32 留在 bin。

| # | Commit | 内容 | 源行 | 验证 |
| :--- | :--- | :--- | :--- | :--- |
| C1.1 | `refactor(gui): 提取错误文案映射到 gui::error_text` | 新增 `src/gui/error_text.rs`（`error_title`/`error_reason`/`error_suggestion`）；shell 改为 `use wd_pak::gui::error_text::*` 薄封装 | 530–576 | 编译通过；`error_text` 无 `windows_sys` 依赖；人工开坏包 |
| C1.2 | `refactor(gui): 提取文本投影到 gui::text` | 新增 `src/gui/text.rs`（`entry_text`/`attrib_text`/`detail_text`/`short_middle`/`status_text`）；shell 对应函数转调 | 930–961, 1003–1020, 1036–1087, 1089–1101, 1103–1124 | 编译通过；人工开包确认详情/状态栏**逐字不变** |
| C1.3 | `fix(gui): 对齐 status_bar::full_text 与外壳文案` | `src/gui/status_bar.rs` 的 `full_text` 与外壳格式统一（`（存储比 …）`） | 1103–1124 | 现有 `status_bar` 测试仍过 |
| C1.4 | `test(gui): 新增 tests/gui_text.rs 钉死文案` | 覆盖 `error_text` 各 `PakError` 分支、`entry_text` 9 列分支、`detail_text` 三态、`status_text` 边界 | — | `cargo test --test gui_text` 全绿 |
| C1.5 | `chore(ci): 将 gui_text 加入测试清单` | 同步 `.github/workflows/ci.yml` 的测试枚举 | — | CI 执行到该测试 |

**阶段验证**：`cargo test --lib --test gui_layout --test gui_states --test gui_text` 全绿；Linux `cargo build --release` 通过。

---

### 阶段 2 — bin 内模块拆分（方案 A 阶段2）

> 前置：D0.1 已决。若选 A（删除），C2.0 变为"删除 + 接线新结构"；若选 B，C2.0 需先修 `EOF` 与 3 处漂移。
> 原则：**每步只搬一个模块，保持构建绿**。

| # | Commit | 目标文件 | 源行 | 验证 |
| :--- | :--- | :--- | :--- | :--- |
| C2.0 | `refactor(gui): 接线 src/bin/gui 模块结构` | `wd-pak-gui.rs` 加 `#[cfg(windows)] mod gui;`；处理 D0.1 | — | 编译通过（含 Linux） |
| C2.1 | `refactor(gui): 拆出 gui/util` | `util.rs`（`wide`/`dpi_of`/`place`/`static_color`/`fatal`/`mtime_local`/`mtime_text`） | 118–129, 328–333, 896–906, 963–1001, 1292–1303 | 冒烟 |
| C2.2 | `refactor(gui): 拆出 gui/clipboard` | `clipboard.rs`（`copy_text`） | 908–928 | 冒烟：右键复制路径 |
| C2.3 | `refactor(gui): 拆出 gui/dialogs` | `dialogs.rs`（错误对话框全套）**以外壳为准**，不采用旧文件的 Esc/Tab | 528–928 | 冒烟：坏包 → 展开/复制/关闭 |
| C2.4 | `refactor(gui): 拆出 gui/open` | `open.rs`（文件对话框 + `open_path`） | 459–526 | 冒烟：打开/取消 |
| C2.5 | `refactor(gui): 拆出 gui/layout` | `layout.rs`（`shell_layout`/`layout_controls`/`paint`/`zone`） | 372–445, 1126–1154 | 冒烟：缩放、DPI 切换 |
| C2.6 | `refactor(gui): 拆出 gui/listview` | `listview.rs`（列初始化/`LVN_GETDISPINFOW`/`selected_entry`） | 1251–1275, 217–255, 1022–1034 | 冒烟：列表滚动/选中 |
| C2.7 | `refactor(gui): 拆出 gui/command_bar` | `command_bar.rs`（打开按钮/搜索框/IME/计时器） | 1158–1185, 335–370, 391–411, 207–214, 283–289 | 冒烟：中文组词、F5、Esc |
| C2.8 | `refactor(gui): 拆出 gui/menu` | `menu.rs`（菜单栏/右键菜单/命令分发） | 78–102, 1356–1376, 263–282, 180–206 | 冒烟：菜单、右键 |
| C2.9 | `refactor(gui): 拆出 gui/detail_panel` | `detail_panel.rs` | 1200–1213, 424–433, 447–457 | 冒烟：选中/取消 |
| C2.10 | `refactor(gui): 拆出 gui/status_panel` | `status_panel.rs` | 1214–1227, 435–444, 455–456 | 冒烟 |
| C2.11 | `refactor(gui): 拆出 gui/app 与 gui/wndproc` | `app.rs`（`Ui`+`create_ui`+USERDATA 辅助）、`wndproc.rs`（主 wndproc 分发） | 104–116, 1156–1290, 131–326 | 全量冒烟 |
| C2.12 | `refactor(gui): wd-pak-gui.rs 收敛为薄入口` | 删除 `mod win {}` 包裹层，收敛到 ~10 行 | 1305–1396 | 编译 + 全量冒烟 |

**阶段验证**：`git diff -M --stat` 显示 rename；全量人工冒烟（见 §6.3）；Linux `cargo build --release` 通过。

---

### 阶段 3 — 功能接入（G3 → G4 → G5）

> 每接一个功能 = 1 个 commit + 1 条 `tests/` 级验收。顺序按风险从低到高。

| # | Commit | 功能 | 复用纯逻辑 | 验证 |
| :--- | :--- | :--- | :--- | :--- |
| C3.1 | `feat(gui): G3 导出与预览` | 导出对话框 + 预览窗口 + 拖出 | `gui/export.rs` | 新增 `tests/gui_export_flow.rs` |
| C3.2 | `feat(gui): G4 替换` | 替换向导 + 备份 + 写后刷新 | `gui/replace.rs` | 新增 `tests/gui_replace_flow.rs` |
| C3.3 | `feat(gui): G4 打包` | 打包向导 + 并行度 + 预检浮层 | `gui/pack.rs` | 新增 `tests/gui_pack_flow.rs` |
| C3.4 | `feat(gui): G4 解包` | 解包向导 + 冲突提示 | `gui/unpack.rs` | 新增 `tests/gui_unpack_flow.rs` |
| C3.5 | `feat(gui): G5 校验` | 校验页 + 问题表 + 结论生命周期 | `gui/check.rs` | 新增 `tests/gui_check_flow.rs` |
| C3.6 | `feat(gui): G5 任务面板与通知` | 任务中心 + 通知中心 | `gui/task_panel.rs`/`task_log.rs`/`states.rs` | 新增 `tests/gui_task_flow.rs` |

> **注意**：G4 是 CLI 七项能力中风险最高、规格最细的部分（`plan.md:305-423`），建议单独排期。

---

### 阶段 4 — 规格冲突修复（D1–D13）

> 与阶段 1–3 可并行。每个 commit 修一组冲突 + 补测试。

| # | Commit | 修复项 | 位置 |
| :--- | :--- | :--- | :--- |
| C4.1 | `fix(gui): 修正默认值以对齐规格` | D1 备份默认开、D2 jobs 默认 `min(CPU,8)`、D3 settings jobs、D4 托盘默认关、D5 字号 100–150 | `replace.rs`/`pack.rs`/`settings.rs` |
| C4.2 | `fix(gui): 修正版本/账户/校验结论` | D6 版本显示 `1.0`、D7 账户无登录态、D8 校验"已取消" | `diagnostics.rs`/`account.rs`/`status_bar.rs` |
| C4.3 | `fix(gui): 补齐打包非 Latin-1 预检` | D9 新增 `NonAscii` 变体并阻断 | `pack.rs`/`format.rs` |
| C4.4 | `fix(gui): 补齐断点/导航/菜单/对话框` | D10 三档断点、D11 导航项、D12 菜单默认隐藏、D13 错误框重试/打开文件夹 | `layout.rs`/`nav.rs`/`menu.rs`/`dialogs.rs` |

---

### 阶段 5 — G6 / G7 收尾

| # | Commit | 内容 |
| :--- | :--- | :--- |
| C5.1 | `feat(gui): G6 Windows 集成` | 拖放、托盘、命令栏、跳转列表、文件关联 |
| C5.2 | `feat(gui): G7 视觉与无障碍` | 三档断点覆盖式详情、主题、文本缩放、UIA |

---

## 5. 不变量（任何阶段不得违反）

1. **`Ui.text` 指针逃逸**：必须继续挂在长期存活的 `Ui` 上，不得改成函数内局部 `Vec`。
2. **`GWLP_USERDATA` 双重语义**：主窗口 `Box<Ui>` vs 错误框 `Box<ErrorState>`；禁止无类型的全局 `userdata()` 辅助。
3. **`Box<Ui>` 所有权**：先构造完再 `into_raw`；唯一释放在 `WM_NCDESTROY`。
4. **嵌套消息循环**：禁止把状态抽成 `static mut` / 全局单例。
5. **跨平台隔离**：新增 `mod gui;` 必须 `#[cfg(windows)]`，否则 Linux CI 失败。
6. **行为保持**：阶段 1–2 只搬移；3 处已知漂移（Esc/Tab、无包详情文案、存储比格式）不得"顺手统一"。

---

## 6. 验证矩阵

### 6.1 自动

| 层级 | 命令 | 期望 |
| :--- | :--- | :--- |
| 库单元 | `cargo test --lib` | 全绿 |
| GUI 纯逻辑 | `cargo test --test gui_layout --test gui_states --test gui_text` | 全绿 |
| 跨平台 | Linux `cargo build --release` | 成功 |
| 搬移纯度 | `git diff -M --stat` / `git log --follow` | 显示 rename，无逻辑行增删 |

### 6.2 新增测试（钉死文案）

`tests/gui_text.rs`：
- `error_text`：`Truncated`/`BadMagic`/`TreeCycle`/`NameTooLong`/`NameCaseConflict`/`Missing`/`NotExists`/兜底
- `entry_text`：9 列 × {目录, LZSS, RAW, `org=0`, `mtime=0`}
- `detail_text`：无包 / 空选中 / 选中目录 / `attrib=0`
- `status_text`：无包 / 无选中 / `short_middle` 边界

### 6.3 人工冒烟（无自动化 GUI 测试，必须做）

用 `tests/golden/layout/etc_mini_compress.pak`：
1. 启动 → 标题 `wd-pak`，详情 `"未打开包\n\n点击"打开 PAK""`
2. 打开包 → 6 列 16 行，标题 `wd-pak — …`
3. 搜索 → 过滤；中文 IME 组词期间不过滤；F5 刷新；Esc 清空
4. Ctrl+O / Ctrl+F
5. 右键 → 复制路径 → 剪贴板正确
6. 菜单：文件/帮助
7. 打开非 PAK → 错误对话框全流程
8. 缩放 <1100 DIP、<760 高、拖到 125%/150% 屏

---

## 7. 回滚策略

- 每个 commit 独立可回滚（`git revert <sha>`）。
- 阶段 1–2 是纯搬移，`git diff -M` 可证明无逻辑变更，回滚零风险。
- 阶段 3–5 是功能新增，回滚即移除该功能。

---

## 8. 待办与开放问题

| # | 问题 | 影响 |
| :--- | :--- | :--- |
| Q1 | `src/bin/gui/` 删除还是接线？（D0.1） | 决定 C2.0 形态 |
| Q2 | 65 个子任务分母未独立复核（`impl-plan` 用表格编号） | 影响百分比精度，不影响阶段划分 |
| Q3 | 17 个未跟踪文件是否纳入版本库 | 影响 CI 门禁完整性 |
| Q4 | `TrackPopupMenu` 后无 `DestroyMenu`（`wd-pak-gui.rs:274-279`） | 既有资源泄漏，建议单列 commit 修 |

---

## 附：提交粒度汇总

| 阶段 | 提交数 | 累计 |
| :--- | :--- | :--- |
| 0 | 1 | 1 |
| 1 | 4–5 | 5–6 |
| 2 | 12–13 | 17–19 |
| 3 | 6 | 23–25 |
| 4 | 4 | 27–29 |
| 5 | 2–3 | **29–32** |

> 全量约 **30 个 commit**。若走最小路径（0→1），仅 **5–6 个 commit** 即可获得"文案可测"的收益。
