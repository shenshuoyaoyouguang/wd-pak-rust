# wd-pak Windows x64 版适配规划

> 状态：**实施中（M0 编译基线、M1 P0 已落地，M2–M4 未开始，见 §0）**｜适用版本：wd-pak 1.0.0｜代码基线：`main` @ `b832f97`
> 本文只描述方案与验证路径，不含实现代码。实施阶段的代码改动以第 2 节的模块清单为边界，逐条落地。
> 与 `MANUAL.md` 12.4「Windows 平台边界」的关系：本文**取代**该节的结论性描述（该节写在仅有 cfg 分支、未做 Windows 适配的阶段）；M4 完成后需回写 `MANUAL.md` 与 `README.md`。

---

## 0. 实施进度

| 里程碑 | 状态 | 说明 |
| :--- | :--- | :--- |
| M0 编译基线 | ✅ 已完成 | 静态 CRT 构建 0 警告；`.github/workflows/ci.yml` 增加 `build-windows` job（构建 + golden 包冒烟 + 产物归档） |
| M1 P0-1 ~ P0-8 | ✅ 已完成 | 见下表 |
| M2 测试门禁 | ⬜ 未开始 | 自包含冒烟用例、三套对拍测试门控、windows test job |
| M3 安装包 / M4 发布 | ⬜ 未开始 | MSI、tag 发布流水线 |

| 项 | 落地情况 |
| :--- | :--- |
| P0-1 平台层 | `src/platform/{mod,unix,windows}.rs`；`unix.rs` 为原实现等比平移 |
| P0-2/P0-3 文案与码表 | `platform::io_message`；Win32 12 条固定文案；`PakError::Io` 统一走该入口 |
| P0-4 盘符 | `platform::normpath` 保留 `C:` / UNC 前缀；`cat -o` 输出路径已带盘符（实测） |
| P0-5 控制台 | **采用方案 B**：`WriteConsoleW` / `ReadConsoleW`，不改代码页 |
| P0-6 交互输入 | 非法 UTF-8 提示后重试；控制台输入不受代码页影响 |
| P0-7 替换加固 | rename 前显式 `drop` 源句柄；被占用时报「目标文件被其他程序占用或不可写」，退出码 2，`.tmp` 不残留（实测） |
| P0-8 单测平台化 | 3 条断言按平台分支；`cargo test --lib` Windows 55 passed / 0 failed |

---

## 1. 现状与 Windows 适配差距分析

### 1.1 目标系统与兼容性要求

| 维度 | 决策 | 说明 |
| :--- | :--- | :--- |
| 操作系统 | Windows 10 1809（17763）及以上、Windows 11、Windows Server 2019 及以上 | 用户确认；低于 17763 不承诺。Win7/2008 R2 不覆盖 |
| 架构 | 仅 x64 | x86 32 位与 ARM64 不覆盖 |
| 目标三元组 | `x86_64-pc-windows-msvc` | 官方 tier-1，避免 mingw 交叉链 |
| C 运行时 | `-C target-feature=+crt-static`（静态 CRT） | 单 exe 零运行时依赖，对齐项目「单二进制」取向 |
| 工具链 | stable（实测 1.93.0）+ VS Build Tools（MSVC v143+、Windows SDK 10.0.17763+） | 本机为 VS Community 2026（MSVC 14.51）实测通过 |
| 文件系统 | NTFS 为基线；FAT32/exFAT 允许 | FAT 的 mtime 精度 2 秒，影响时间戳类断言 |
| 产物字节兼容 | **硬要求**：PAK 二进制、LZSS 压缩流、`_pak_manifest.json` 与 Python 参考实现逐字节一致 | 已实测（见 1.2） |
| CLI 兼容 | 子命令、参数名、退出码语义与 Python 版一致 | 已实测退出码 |
| 输出文案 | Windows 用固定文案，不复刻 Python-on-Windows 的本地化 strerror | 用户确认；Linux 侧文案不变 |
| 分发 | per-machine MSI（WiX）+ 独立 exe | 支持静默安装、静默卸载、组策略分发 |
| CI | `windows-latest` 构建 + 冒烟；tag 触发 Release | 与现有 ubuntu 门禁并列 |

### 1.2 构建与运行实测（2026-10-18，本机 Win x64）

| 验证项 | 命令 | 结果 |
| :--- | :--- | :--- |
| 静态 Release 构建 | `RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target x86_64-pc-windows-msvc` | **成功，32.07s，0 warning** |
| 产物体积 | — | `wd-pak.exe` = **1,331,712 B**（1.27 MiB） |
| DLL 依赖 | `dumpbin /dependents` | KERNEL32.dll、ntdll.dll、api-ms-win-core-synch-l1-2-0.dll、bcryptprimitives.dll；**无 vcruntime140 / msvcp140 / ucrtbase** |
| 版本冒烟 | `wd-pak.exe --version` | `1.0`，退出码 0 |
| 解包冒烟 | `wd-pak.exe extract tests\golden\layout\etc_mini_nocompress.pak <tmp>` | 16 个文件 + `_pak_manifest.json` = 17 个，退出码 0 |
| 打包字节一致性（不压缩） | `wd-pak.exe pack <tmp> repack.pak --no-compress` | SHA256 `0e1affb9…9102b`，与 `tests/golden/layout/etc_mini_nocompress.pak` **完全相同** |
| 打包字节一致性（压缩） | `wd-pak.exe pack <tmp> repack_c.pak` | SHA256 `ad01c944…53afc2`，与 `tests/golden/layout/etc_mini_compress.pak` **完全相同**（并行压缩输出可复现） |
| 替换冒烟 | `wd-pak.exe replace <pak> 100heis.list tests\golden\replace\newfile.bin` | 成功，`# 已替换 100heis.list: 4500 字节 (存储 542 字节)` |
| 退出码 | 缺失 pak / 坏魔数 / `--version` | 2 / 2 / 0 |

**结论**：功能面（格式解析、压缩、打包、解包、替换、退出码）在 Windows 上已可用且字节一致；缺口集中在**文案、路径、控制台、测试门禁、分发**五处，其中前三处属于「能用但不像 Windows 程序」，后两处属于「无法交付与自证」。

### 1.3 差距 A：错误文案与 errno 语义（P0）

| 项 | 内容 |
| :--- | :--- |
| 现状锚点 | `src/commands_read.rs:259-269` `py_errno_msg`：`format!("[Errno {code}] {msg}")`，`msg` 来自 `io::Error::to_string()`；`src/commands_read.rs:274-280` `py_os_error` 追加 `: {repr(path)}` |
| Windows 实际 | 实测目标被占用时输出：`错误: 写出 pak 失败: [Errno 5] 拒绝访问。: 'C:\\...\\x.pak.tmp' -> 'C:\\...\\x.pak'`。`5` 是 Win32 `ERROR_ACCESS_DENIED`；`拒绝访问。` 来自系统 FormatMessage，**随系统语言变化** |
| 与 Python 的偏差 | Python 3 在 Windows 通过 `_dosmaperr` 把 Win32 码映射成 `errno`，同一情形输出 `[Errno 13] Permission denied`；数字与文本都不同 |
| 次级锚点 | `src/error.rs:88-89` `PakError::Io` 直接 Display 透传，`pak.rs` / `collect_files` 路径上的 IO 错误不带任何码前缀 |
| 现有失败单测 | `commands_read.rs:401`（期望 `/a/c`）、`commands_read.rs:409-414`（期望英文 `No such file or directory`）、`commands_write.rs:1001-1008`（以 `from_raw_os_error(21)` 构造"是目录"，Windows 上 21 是 `ERROR_NOT_READY`） |
| 影响面 | 运维脚本、告警规则、文档中所有基于错误文本或 `[Errno n]` 数字的匹配；三个单测在 Windows 上永久失败 |

判定：这是**文案不可移植 + 错误码语义跨平台不一致**，不是功能缺陷；但按「Windows 原生文案优先」的决策，必须整体重构映射层，而不是逐条改字符串。

### 1.4 差距 B：路径规范化丢失盘符（P0，功能缺陷）

| 项 | 内容 |
| :--- | :--- |
| 现状锚点 | `src/commands_read.rs:330-365` `normpath`：`Component::Prefix(_) => {}` 直接丢弃盘符，且末尾 `out.push("/")` 硬编码 POSIX 根；`abspath`（`:318-327`）基于它 |
| Windows 实际 | 单测实测：`abspath("C:\\...\\wd-pak-rust\\x\\y")` 得到 `/Users\\76578\\...\\x\\y`（前导盘符与第一个分隔符丢失），与期望 `C:\\...\\x/y` 不符 |
| 功能影响 | `cmd_cat` 的 `-o` 成功提示（`:162` 用 `abspath(path).display()`）会打印出**不可用的路径**（丢盘符 → 变成根相对路径），用户直接复制会失败 |
| 文案影响 | `commands_read.rs:133/162/214`、`commands_write.rs:810/905` 用 `display()` 输出路径，Windows 上是 `C:\a\b`；这是 Windows 正常表现，不视为缺陷，但与 Linux 输出不同 |
| 输入侧核对 | pak 内条目名归一化已存在：`pak.rs:87`、`commands_write.rs:871`、`interactive.rs:120` 均执行 `replace('\\', "/")`，用户输入 `a\b\c` 可正确定位条目 |
| 影响面 | `cat -o` 输出错误（功能性）；3 个路径类断言（测试性） |

### 1.5 差距 C：写入与原子替换（P0 的原始判断需修正）

| 项 | 内容 |
| :--- | :--- |
| 现状锚点 | `src/commands_write.rs:732-752`：`File::open(src)` 句柄活到函数结束，之后才 `fs::rename(tmp, out_path)`；`:907` 注释明写「Linux 下 rename 覆盖已打开文件虽可行」 |
| Windows 实测 | 正常 replace **成功**——Rust std 打开文件时带 `FILE_SHARE_DELETE`，`MoveFileEx` 可替换被自身句柄打开的目标 |
| 失败路径实测 | 用不共享删除的句柄（`FileShare.Read`）占用目标后 replace：`错误: 写出 pak 失败: [Errno 5] 拒绝访问。: '…x.pak.tmp' -> '…x.pak'`，退出码 2，`.tmp` 按 FR-38 清理；释放句柄后立即重试成功 |
| 结论 | 不是功能阻断，但有两处必须加固：①当前正确性依赖 std 的隐式共享模式（未写进代码契约，未来换 I/O 实现即失效）；②目标被占用时报「拒绝访问」，无法判断是权限不足还是文件被占用 |
| 备份路径 | `:899-905` `fs::copy` + `copy_mtime`：只复内容与 mtime，不复只读属性/ACL；`.bak` 已存在且为只读时失败 |
| mtime 回写 | `:917-922` `FileTimes` 写回；NTFS 100 ns、FAT 2 s。`tests/write_cli.rs:610` 的 `assert_eq!(modified(), t0)` 属外部资产套件，Windows 侧**尚未实测**（列入 M2 自包含用例） |

### 1.6 差距 D：控制台编码与交互输入（P0）

| 项 | 内容 |
| :--- | :--- |
| 现状锚点 | 全仓无 TTY 检测（无 `is_terminal`），无控制台代码页处理；`src/main.rs:95` `eprintln!("错误: {e}")`、`src/interactive.rs:55-66` 全中文输出 |
| Windows 实际 | 实测 stderr 原始字节 `35 32 232 167 163 …`（`# 解`），即**始终输出 UTF-8 字节**，与当前控制台代码页无关。默认 cp936 控制台下中文乱码，`chcp 65001` 后正常 |
| 与 Python 的偏差 | Python 3 在 Windows 走 `WriteConsoleW`，无乱码——这是 `MANUAL.md:459-461` 已记录的「唯一可感知差异」 |
| 交互输入 | `src/interactive.rs:33` `read_line(&mut line).unwrap_or(0)`：cp936 控制台输入中文 → 非法 UTF-8 → `Err` → 被当作 EOF → 打印「已退出」直接退出程序 |
| 管道场景 | 无颜色、无 `\r` 覆盖进度（进度是 `# 压缩进度 n/N` 独立行，`commands_write.rs:411-416`），重定向输出不受影响 |
| 影响面 | 交互模式在中文 Windows 上基本不可用（菜单可读性 + 中文路径输入）；非交互模式可用但需 `chcp` |

### 1.7 差距 E：文件系统语义差异（P1）

| 项 | 内容 |
| :--- | :--- |
| 包内名合法性 | `src/format.rs:141` 只拒绝空名、`.`、`..`、含 `/` `\` `\0`、>43 字节；`: * ? " < > |`、保留设备名（CON/PRN/AUX/NUL/COM1/LPT1）、尾随点/空格**可以入包**（Python 版同样允许） |
| 解包后果 | `src/commands_read.rs:99` `outdir.join(&rel)` 后直接 `File::create`（`:300`）：上述名字在 Windows 上或失败（`[Errno 5]`/`[Errno 123]`）、或写入 ADS（`a:b`）、或落成非法文件名。`write_file`（`:300-304`）报错时带目标路径，能定位条目，但不会说明「该名字在 Windows 上非法」，且解包中途失败会留下半成品目录 |
| 大小写冲突 | pack 侧已有校验（`commands_write.rs:314` `NameCaseConflict`，`error.rs:62` 文案明确提到 Windows）；**extract 侧只警告**（`commands_read.rs:104-111`），NTFS 大小写不敏感 → 后到者覆盖，静默丢数据 |
| 长路径 | 无应用清单 `longPathAware`、无 `\\?\` 前缀；路径 >260 字符时失败（除非系统已启用 `LongPathsEnabled`） |
| 文件名原始字节 | `commands_write.rs:38-47` `cfg(not(unix))` 退化为 lossy；Windows 无「原始字节文件名」概念，排序按 UTF-8 字节序与 Unicode 码点序单调一致，**不构成偏差** |
| 时间戳 | pak 内 mtime 为 u32 原样搬运（`format.rs:95,120`），无时区换算——跨平台一致 |

### 1.8 差距 F：测试体系与门禁（P0）

本地实测（`cargo test --no-fail-fast`，Windows）：

| 测试目标 | passed / failed / ignored | 说明 |
| :--- | :--- | :--- |
| `src/lib.rs` 单元测试 | 50 / 3 / 2 | 3 个失败均为平台语义（见 1.3、1.4） |
| `tests/format_basic.rs` | 9 / 0 / 0 | 自包含，Windows 可用 |
| `tests/lzss_golden.rs` | 3 / 0 / 0 | 自包含，Windows 可用 |
| `tests/golden.rs` | 1 / 16 / 2 | 依赖外部资产 |
| `tests/read_cli.rs` | 0 / 10 / 1 | 依赖外部资产 |
| `tests/write_cli.rs` | 1 / 13 / 1 | 依赖外部资产 |

外部资产与硬编码：

| 类型 | 位置 | 说明 |
| :--- | :--- | :--- |
| Python 参考实现 | `tests/golden.rs:59,126`、`tests/read_cli.rs:49`、`tests/write_cli.rs:66` | `Command::new("python3")` 调 `../wd_pak_tool.py`；Windows 上通常是 `python`/`py -3` |
| 大 PAK | `tests/read_cli.rs:156,162,183,238-239`、`tests/write_cli.rs:257-261`、`tests/golden.rs:200,318` | `../release/etc.pak`（85 MB）、`../release/lib_gs32.pak`（133 MB）；**本机已不存在该目录**，故这三套件在本地也跑不了 |
| POSIX 临时路径 | `tests/read_cli.rs:154,197,217,270,281`；`tests/write_cli.rs:143,144,266,279,297,326,355,418,441,473,498,512,541,638,649`；`tests/golden.rs:106,431` | 硬编码 `/tmp/...`，Windows 解析为 `C:\tmp\...`，普通用户无权创建 |
| 替换用例数据 | `tests/golden/replace/replace.json` | 三条用例全部指向 `etc.pak` / `lib_gs32.pak`，**没有针对 mini 包的用例** |

可用于 Windows 的自包含资产（**已确认存在**）：`tests/golden/layout/etc_mini_compress.pak`、`etc_mini_nocompress.pak`、`layout.json`、`layout/etc_mini/`（16 文件）、`replace/newfile.bin`、`replace/replace.json`、`extract_files.sha256`、`golden_index.json`、`comp/`、`decomp/` 向量。这套资产足以在**不依赖 Python** 的前提下验证打包/解包/替换的字节一致性——M2 的 Windows 冒烟即以此为基础。

### 1.9 已确认「不是问题」的部分（避免重复投入）

- 格式解析、LZSS 压缩、manifest 序列化：无平台分支，字节一致已实测
- rayon 并行、`available_parallelism`、`Mutex<File>` 共享读：冒烟正常
- clap 解析、退出码（0/1/2）语义：实测正常
- 单元测试临时目录：`pak.rs:298`、`manifest.rs:200,223`、`commands_write.rs:1084` 已使用 `std::env::temp_dir()`，无需改动
- 文件名排序：Windows 侧 lossy 与 Python 的 `str` 排序一致（见 1.7 末行）

---

## 2. 模块级适配与优化清单

### 2.1 设计原则

1. **只改 I/O 边界**：格式解析、LZSS、manifest、tree 构建一行不动；所有平台差异收敛到 `src/platform/`。
2. **Linux 行为零回归**：`unix.rs` 是现有逻辑的等比平移，不是重写；判断标准是 ubuntu CI 与现有 112 项测试结果不变（除已平台门控的 3 条）。
3. **不为 Windows 引入新抽象层之上的抽象**：业务代码只调 `platform::*` 五个函数，不出现 `cfg(windows)`。

### 2.2 新增：平台隔离层

```
src/platform/
├── mod.rs        # 统一 re-export + 公共类型（ConsoleGuard 等）
├── unix.rs       # 现有 POSIX 行为原样平移（py_errno_msg / normpath 原实现）
└── windows.rs    # Win32 映射、控制台、输入解码
```

| 接口 | unix 实现 | windows 实现 | 替换的现有锚点 |
| :--- | :--- | :--- | :--- |
| `io_message(&io::Error) -> String` | 现有 `py_errno_msg` 逻辑原样 | 固定 Win32 码表 → `[Win32 n] 中文文案`；未收录码回退 `[Win32 n] {系统文本}` | `commands_read.rs:259-269`、`error.rs:88-89` |
| `normpath(&Path) -> PathBuf` | 现有 `commands_read.rs:330-365` | 保留 `Component::Prefix`（盘符 `C:`）与原分隔符；根为 `C:\` | `commands_read.rs:318-365` |
| `init_console() -> ConsoleGuard` | no-op | `SetConsoleOutputCP/SetConsoleCP(65001)`，`ConsoleGuard::drop` 恢复旧代码页 | 新增（`main.rs` 入口调用） |
| `read_line(prompt: &str) -> Result<String, &'static str>` | 现有 `interactive.rs:28-38` | 同流程，但非法 UTF-8 返回错误而非当作 EOF | `interactive.rs:33` |
| `is_sharing_violation(&io::Error) -> bool` | 恒 `false` | `ERROR_SHARING_VIOLATION(32)` / `ERROR_LOCK_VIOLATION(33)` / `ERROR_ACCESS_DENIED(5)` 且目标存在 | 新增（替换失败诊断） |

**控制台方案的取舍（需在 M1 决策时确认）**：

| 方案 | 做法 | 优点 | 代价 |
| :--- | :--- | :--- | :--- |
| A（推荐） | 启动设 CP 65001，退出时恢复 | 改动最小（约 40 行），`println!` 全部无需改 | 代码页是**控制台级**状态，异常终止时不恢复；子进程继承同一控制台 |
| B | stdout/stderr 收口为自定义 Writer，控制台走 `WriteConsoleW` | 与 Python 一致，无全局副作用 | 需改所有 `&mut impl Write` 调用点，改动面大（P2 再评估） |

**输入方案**：`SetConsoleCP(65001)` 后控制台把键入按 UTF-8 提交，`read_line` 正常工作；仅在输入被重定向且字节非法时返回可读错误（`输入不是合法 UTF-8，请确认终端编码为 UTF-8`），**不引入 GBK 解码表**（避免新依赖与双编码歧义）。

### 2.3 适配：按模块的改动影响面

| 模块 | 文件 | 改动性质 | 具体动作 | 优先级 |
| :--- | :--- | :--- | :--- | :--- |
| 入口 | `src/main.rs` | 适配 | 启动调用 `platform::init_console()`；其余分发逻辑不动 | P0 |
| 库根 | `src/lib.rs` | 新增 | 挂载 `pub mod platform;` | P0 |
| 错误 | `src/error.rs` | 适配 | `PakError::Io` 的 Display 走 `platform::io_message`，消除两套文案 | P0 |
| 读命令 | `src/commands_read.rs` | 适配 | `py_errno_msg`/`py_os_error` 改调平台层；`abspath`/`normpath` 下沉；`cmd_extract` 增加名字合法性预检（P1） | P0/P1 |
| 写命令 | `src/commands_write.rs` | 适配 | `write_to_tmp` 在 rename 前显式 drop 源句柄；rename 失败时判定共享冲突并给出「目标被占用」提示；备份属性（P2） | P0/P2 |
| 交互 | `src/interactive.rs` | 适配 | `ask()` 改调 `platform::read_line`，非法字节给出提示并重试/退出，不再静默 EOF | P0 |
| 格式/压缩 | `src/format.rs`、`src/lzss.rs`、`src/tree.rs` | **不改** | — | — |
| 读写核心 | `src/pak.rs` | **不改** | 条目名归一化（`:87`）已覆盖 Windows 输入 | — |
| manifest | `src/manifest.rs` | **不改** | 写入的条目名来自 pak 内部 `/` 分隔的 rel，天然平台无关 | — |
| 测试 | `tests/*` | 适配/新增 | 三套外部资产测试门控；新增 `tests/windows_smoke.rs`；`/tmp` 改 `temp_dir` | P1 |
| 构建 | `Cargo.toml`、`build.rs`（新增）、`wd-pak.exe.manifest`（新增） | 新增 | Windows 目标依赖、静态 CRT、清单嵌入 | P1 |
| 分发 | `installer/wd-pak.wxs`（新增） | 新增 | WiX 源文件 | P1 |
| CI | `.github/workflows/ci.yml`、`release.yml`（新增） | 新增 | windows job + tag 发布 | P1 |

### 2.4 P0 清单（不修则 Windows 版不可交付）

| ID | 动作 | 改动边界 | 验证方式 |
| :--- | :--- | :--- | :--- |
| P0-1 | 建 `src/platform/` 三文件，`unix.rs` 平移现有实现 | 纯新增 + `lib.rs` 一行挂载 | Linux：`cargo test --lib` 结果与基线一致；Windows：编译通过 |
| P0-2 | 统一 IO 消息入口（`io_message`） | `commands_read.rs:257-280`、`error.rs:88-89` | Linux：现有涉及消息的断言（`format_basic`、`lzss_golden`、单元测试）逐条不变；Windows：错误输出固定中文文案 |
| P0-3 | Win32 错误码表（首批 12 条：2/3/5/13/15/32/33/112/123/145/183/206） | `platform/windows.rs` 常量表 + 单元测试 | 单测覆盖「码→文案」映射；未收录码走回退路径 |
| P0-4 | `normpath` 保留盘符 | `platform/{unix,windows}.rs` | `abspath("C:\\a\\b")` → `C:\a\b`；`cat -o` 打印的路径可直接 `Get-Item` 打开（验收时实测） |
| P0-5 | 控制台 UTF-8 初始化 + 代码页恢复 | `main.rs` + `platform/windows.rs` | cp936 窗口内菜单/中文路径/错误消息无乱码；退出后 `chcp` 显示原值 |
| P0-6 | 交互输入降级 | `interactive.rs:28-38` | 中文路径输入可正常定位条目；非法字节给出提示，不再「已退出」 |
| P0-7 | 替换流程句柄释放 + 占用诊断 | `commands_write.rs:732-752`、`:900-908` | 正常 replace 成功且字节正确；目标被占用时报「目标文件被其他程序占用」，退出码 2，`.tmp` 不残留 |
| P0-8 | 3 条单测改为平台分支断言 | `commands_read.rs:401,409-414`、`commands_write.rs:1001-1008` | Windows `cargo test --lib` 全绿；Linux 断言文本不变 |

### 2.5 P1 清单（必做，否则体验/可靠性不达标）

| ID | 动作 | 改动边界 | 验证方式 |
| :--- | :--- | :--- | :--- |
| P1-1 | 解包前校验条目名在 Windows 上的可创建性：`<>:"\|?*`、保留设备名（含 `COM1`-`COM9`/`LPT1`-`LPT9`）、尾随点或空格 | `commands_read.rs:97-116` + 新增错误变体（`error.rs`） | 构造含 `CON.list`、`a:b` 的 pak → 明确报错并指出条目名，不再出现裸 `[Win32 123]` |
| P1-2 | 解包大小写冲突：Windows 上由「警告」改为**默认拒绝**并列出冲突条目（沿用 `error.rs:62` 文案建议的改名方案）；Linux 行为不变 | `commands_read.rs:104-111` | Windows：冲突包解包失败并列出 `A.txt / a.txt`；Linux：仍只警告（现有测试不变） |
| P1-3 | 长路径：①`wd-pak.exe.manifest` 声明 `longPathAware`；②输入路径超过 255 字符时先绝对化再加 `\\?\` 前缀（UNC 用 `\\?\UNC\`） | 新增 `build.rs`（`cargo:rustc-link-arg=/MANIFESTINPUT:...`，无第三方依赖）+ `platform/windows.rs` | 300 字符路径下 `list`/`extract` 成功；未启用 `LongPathsEnabled` 的机器上清单不生效但有明确错误 |
| P1-4 | 新增 `tests/windows_smoke.rs`：不依赖 Python、不依赖 `release/*.pak` | 纯新增 | 见 2.6 用例表 |
| P1-5 | 三套外部资产测试平台门控（`#![cfg(unix)]`） | `tests/golden.rs`、`read_cli.rs`、`write_cli.rs` 首行 | Windows `cargo test` 全绿（不再出现 26 条失败） |
| P1-6 | `/tmp` 硬编码改 `std::env::temp_dir()`；`python3` 改可配置探测（`WD_PAK_PY` 环境变量 → `python3` → `python`） | 同 4 个测试文件 | Linux CI 仍绿；有资产的机器上对拍可跑 |
| P1-7 | CI 增加 `windows-latest` job（构建 + 冒烟 + 上传 exe） | `ci.yml` | job 绿，artifact 可下载 |
| P1-8 | MSI 打包与静默安装验证 | `installer/wd-pak.wxs` + `release.yml` | 见 3.5 |

### 2.6 M2 冒烟用例表（`tests/windows_smoke.rs`，全部自包含）

| 用例 | 输入 | 断言 |
| :--- | :--- | :--- |
| `version` | — | stdout = `1.0`，退出码 0 |
| `list_mini` | `tests/golden/layout/etc_mini_nocompress.pak` | stdout 恰 16 行文件名，顺序等于 `layout.json` 的 `manifest_order`；stderr 统计行 `# 16 files, 0 dirs, rootDirSize=1024` |
| `extract_mini` | 同上 | 输出目录 17 个文件（16 + `_pak_manifest.json`）；逐文件 SHA256 等于 `layout.json` 的 `input_files[].sha256` |
| `pack_roundtrip` | 上一步目录 → `--no-compress` | 产物 SHA256 == `layout.json` 的 `nocompress.sha256`（`0e1affb9…9102b`，已实测） |
| `pack_compress_roundtrip` | 上一步目录 → 默认压缩 | 产物 SHA256 == `layout.json` 的 `compress.sha256`（`ad01c944…53afc2`，已实测） |
| `cat_stdout` | `100heis.list` | stdout 字节 == `layout/etc_mini/100heis.list` |
| `replace_mini` | mini 包 + `replace/newfile.bin` → `100heis.list` | `cat` 回读字节 == `newfile.bin`；`check` 退出码 0；`.bak` 已生成且内容等于替换前整包 |
| `check_ok` / `check_bad` | 正常包 / 篡改魔数 | `校验通过` + 退出码 0 / 退出码 2 |
| `missing_pak` | 不存在路径 | 退出码 2，stderr 含固定文案（平台无关部分） |
| `long_path`（`#[cfg(windows)]`） | 300 字符路径 | 命令成功 |

> `tests/golden/replace/replace.json` 的三条用例全部指向 `etc.pak` / `lib_gs32.pak`（85 MB / 133 MB，不在仓库内），**不能用于 mini 包冒烟**。若要把 replace 也钉成整包哈希，需在 M2 用 Linux 版与 Python 版共同确认后新增一份 `replace_mini.json`（独立于现有 `replace.json`，不改动后者）。

这套用例的价值：**Windows 上的字节一致性第一次可以自证**，且不依赖 Python 与 218 MB 大包。

### 2.7 P2 清单（增强，可后置）

| ID | 动作 | 理由 |
| :--- | :--- | :--- |
| P2-1 | TTY 下进度覆盖刷新（`\r`） | 纯观感；重定向时必须退回逐行，逻辑要保持两支 |
| P2-2 | Ctrl+C 中断时删除 `.tmp` | 目前仅错误路径清理（`commands_write.rs:703-706`），中断会留下残留文件 |
| P2-3 | 备份保留只读属性/ACL | 对齐 `shutil.copy2` 语义；跨平台需 `fs::Permissions` 分支 |
| P2-4 | mtime 写入按文件系统粒度取整 | 避免 FAT 上「写回值 ≠ 读回值」 |
| P2-5 | stdout/stderr 收口 `WriteConsoleW` | 消除代码页副作用（替代方案 A） |
| P2-6 | 进度/统计行在 `--jobs>1` 下的稳定性回归 | 现有按文件序重放已确定，仅需回归 |

### 2.8 明确不改的部分

- `src/format.rs`、`src/lzss.rs`、`src/tree.rs`、`src/pak.rs`、`src/manifest.rs` 的格式与算法
- 子命令集合、参数名、退出码语义（0/1/2）
- Linux 侧任何可观察输出（包括 `[Errno n] 英文 strerror` 形态）
- `py_str_repr` 的转义规则（Windows 下反斜杠转义为 `\\`，与 Python `repr` 语义一致，实测输出 `'C:\\Users\\...'`）

### 2.9 依赖与构建配置变更

| 项 | 内容 | 备注 |
| :--- | :--- | :--- |
| Windows 专属依赖 | `[target.'cfg(windows)'.dependencies] windows-sys = { version = "0.5x", features = ["Win32_System_Console", "Win32_Storage_FileSystem"] }` | 实际版本以 `Cargo.lock` 解析结果为准并回填本文 |
| 静态 CRT | CI 中 `RUSTFLAGS="-C target-feature=+crt-static"`；本地命令写入 `MANUAL.md` | 不用 `.cargo/config.toml`，避免 `cargo test` 也走静态链接 |
| 应用清单 | 新增 `build.rs` + `wd-pak.exe.manifest`（`longPathAware`、`supportedOS`） | 若 `build.rs` 的 `/MANIFESTINPUT` 与 rustc 默认清单冲突，回退到 `embed-manifest` crate（M1 实测后定） |
| Release profile | 不变（lto、codegen-units=1、strip） | 实测已生效，exe 1.27 MiB |

### 2.10 改动规模估算

| 部分 | 新增/修改行数（估） | 说明 |
| :--- | :--- | :--- |
| `src/platform/*` | +180 / -60 | 新层 + 原函数平移 |
| 业务模块适配 | ~+60 / -40 | 调用点替换为主 |
| `tests/windows_smoke.rs` | +220 | 10 条用例 |
| 测试门控与路径修正 | ~+40 / -30 | 4 个文件 |
| `build.rs` + manifest | +40 | 清单嵌入（`longPathAware`、`supportedOS`） |
| WiX 源文件 | +120 | 安装/卸载/PATH 组件 |
| 工作流 | +90 | windows job + release.yml |

---

## 3. 分阶段里程碑与关键风险点

### 3.1 总览

| 里程碑 | 目标 | 依赖 | 运行环境 | 预估 | 退出条件（可判定） |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **M0 编译基线** | Windows 上一次编译通过、可运行、纳入 CI | 无 | 本机 Win x64 + VS Build Tools；GitHub `windows-latest` | 0.5 人日 | windows job 绿；0 warning；exe 无 vcruntime 依赖；`--version` = `1.0` |
| **M1 I/O 适配** | 文案/路径/替换/控制台/输入按 Windows 规范可用 | M0；P0-1…P0-8 | 本机 + Win10 22H2 虚机 | 2–3 人日 | Windows `cargo test --lib` 全绿；Linux CI 全绿；cp936 手工验收表 8/8 |
| **M2 测试门禁** | 字节一致性在 Windows 可自证 | M1；P1-4…P1-7 | 上述两个 OS | 1–2 人日 | 两 OS `cargo test` 全绿；`windows_smoke` 10 条全过 |
| **M3 安装包** | 标准 MSI 可静默安装/卸载 | M2；P1-8 | Win10 22H2、Win11 干净虚机 | 1–2 人日 | 安装/运行/升级/卸载清单全过，无残留 |
| **M4 发布流水线** | tag 一键产出发行件 | M3 | GitHub Actions | 0.5–1 人日 | `v*` tag 产出 exe + MSI + SHA256SUMS，下载后在干净 VM 安装成功 |

依赖关系（串行）：`M0 → M1 → M2 → M3 → M4`。M1 内部 P0 项可并行，但 P0-1（平台层）必须先落地；M3 的 WiX 源文件可在 M2 期间并行起草。

### 3.2 M0：编译基线

| 任务 | 产出 | 验证 |
| :--- | :--- | :--- |
| M0-1 固化构建命令与 flags | `MANUAL.md` 新增 Windows 构建段（草稿） | 命令在干净克隆上可复制执行 |
| M0-2 CI 增加 windows job（仅构建 + 冒烟，不含新增测试） | `ci.yml` 增 `build-windows` | job 绿，artifact 含 `wd-pak.exe` |
| M0-3 记录运行时依赖基线 | 本文 1.2 表 | `dumpbin /dependents` 仅系统 DLL |
| M0-4 版本与 tag 约定 | 约定 tag 形如 `v1.0.0`，与 `Cargo.toml` 一致 | M4 中由脚本校验 |

本机验证命令（已执行通过，作为 M0 的判定基准）：

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release --target x86_64-pc-windows-msvc
dumpbin /dependents target\x86_64-pc-windows-msvc\release\wd-pak.exe
target\x86_64-pc-windows-msvc\release\wd-pak.exe --version   # 期望输出 1.0，退出码 0
```

> M0 **不改任何源码**。当前代码即可通过，目的是先把「可编译、可运行、可下载」这条链路钉死，后续适配出的问题才可归因。

### 3.3 M1：I/O 适配

| 任务 | 依赖 | 产出 | 验证 |
| :--- | :--- | :--- | :--- |
| M1-1 平台层骨架（P0-1） | M0 | `src/platform/{mod,unix,windows}.rs`、`lib.rs` 挂载 | Linux `cargo test --lib` 与基线一致；Windows 编译通过 |
| M1-2 错误文案与码表（P0-2、P0-3） | M1-1 | `io_message` + 12 条码表 + 映射单测 | 单测覆盖码→文案；Linux 输出逐字节不变 |
| M1-3 路径盘符（P0-4） | M1-1 | `normpath` 双实现 | `cat -o` 打印路径可直接 `Get-Item` 打开 |
| M1-4 控制台（P0-5） | M1-1 | `init_console` + `ConsoleGuard` | 见 3.3.1 手工验收表 |
| M1-5 交互输入（P0-6） | M1-4 | `read_line` | 中文路径输入可定位条目 |
| M1-6 替换加固（P0-7） | M1-2 | 句柄先 drop + 占用诊断 | 占用/非占用两场景实测 |
| M1-7 单测平台化（P0-8） | M1-3 | 3 条断言分支 | Windows `cargo test --lib` 全绿 |

#### 3.3.1 M1 手工验收表（在 cp936 默认代码页的 cmd 中执行）

| # | 场景 | 操作 | 期望 |
| :--- | :--- | :--- | :--- |
| 1 | 默认代码页中文输出 | 不执行 `chcp`，直接 `wd-pak.exe list tests\golden\layout\etc_mini_nocompress.pak` | 中文无乱码 |
| 2 | 交互菜单 | 无参启动，选择 `1` | 菜单与提示正常显示 |
| 3 | 交互中文路径 | 输入含中文目录的 pak 路径 | 正确定位并列出 |
| 4 | 错误文案 | `wd-pak.exe list 不存在.pak` | 固定中文文案，退出码 2，不含 `[Errno 5]` 之类跨平台混淆数字 |
| 5 | `cat -o` 路径 | `wd-pak.exe cat <pak> 100heis.list -o out.bin` | 打印的绝对路径含盘符，可直接打开 |
| 6 | 替换被占用 | 用 Excel/记事本占用目标 pak 后 replace | 「目标被占用」提示，退出码 2，`.tmp` 不残留 |
| 7 | 控制台恢复 | 退出程序后执行 `chcp` | 显示执行前的代码页（未被永久改为 65001） |
| 8 | 重定向输出 | `wd-pak.exe list <pak> > list.txt` | 文件为 UTF-8、行尾 LF（`println!` 不随平台变 CRLF）；`cat` 重定向出的二进制与包内条目逐字节相同 |

### 3.4 M2：测试门禁

| 任务 | 依赖 | 产出 | 验证 |
| :--- | :--- | :--- | :--- |
| M2-1 `tests/windows_smoke.rs`（P1-4） | M1 | 10 条自包含用例 | 两 OS 全过；Windows 上首次给出字节一致性结论 |
| M2-2 外部资产套件门控（P1-5） | M1 | 三个文件 `#![cfg(unix)]` | Windows `cargo test` 0 失败；Linux 结果不变 |
| M2-3 路径与解释器可配置（P1-6） | M2-2 | `/tmp`→`temp_dir`、`WD_PAK_PY` 探测 | 有资产机器上对拍仍可跑（人工确认一次） |
| M2-4 CI 扩展（P1-7） | M2-1 | `ci.yml` 增 windows test job | 两 OS 门禁同屏可见 |

**门禁定义**（两条必须同时满足）：

- Windows：`cargo test --locked` 在 `windows-latest` 与本机均 0 failed
- Linux：`cargo test --locked` 在 `ubuntu-latest` 保持 0 failed（现有 53 + 9 + 3 不得减少；`windows_smoke` 在非 Windows 上以 `cfg(windows)` 跳过）

### 3.5 M3：安装包

| 任务 | 依赖 | 产出 | 验证 |
| :--- | :--- | :--- | :--- |
| M3-1 产品信息与目录结构定稿 | M2 | 安装目录 `%ProgramFiles%\wd-pak\`、开始菜单项、卸载图标 | 与 3.7 表一致 |
| M3-2 WiX 源文件 | M3-1 | `installer/wd-pak.wxs` | `wix build` 成功，ICE 校验无 error |
| M3-3 静默安装/卸载脚本 | M3-2 | `installer/verify-msi.ps1` | 干净 VM 上 `/qn` 安装、`/x` 卸载 |
| M3-4 升级与降级路径 | M3-2 | MajorUpgrade 配置 | 1.0.0 → 1.0.1 覆盖安装后仅保留一条 ARP 记录 |
| M3-5 干净虚机验证 | M3-3 | 验证记录 | 3.7.5 清单全过 |

M3 的退出条件里明确包含「安装后不依赖 VC++ Redist」：在未安装任何 Visual C++ 运行库的干净 Win10 22H2 上运行 `wd-pak.exe --version` 必须成功。

### 3.6 M4：发布流水线

| 任务 | 依赖 | 产出 | 验证 |
| :--- | :--- | :--- | :--- |
| M4-1 `release.yml`（tag `v*`） | M3 | 发布工作流 | 推送 `v1.0.0` 后自动跑通 |
| M4-2 版本一致性校验 | M4-1 | 脚本：`Cargo.toml` 版本 == tag == MSI 版本 | 版本不匹配时 job 失败 |
| M4-3 发行件与校验和 | M4-1 | `wd-pak-x86_64-windows.exe`、`wd-pak-1.0.0-x64.msi`、`SHA256SUMS` | 下载后 `Get-FileHash` 与清单一致 |
| M4-4 安装说明与文档回写 | M4-3 | `README.md`/`MANUAL.md` Windows 段定稿 | 文档中的命令逐条可执行 |

### 3.7 标准安装程序（WiX MSI）交付细则

#### 3.7.1 工具与运行环境

| 项 | 决策 | 说明 |
| :--- | :--- | :--- |
| 打包工具 | WiX Toolset v5（`dotnet tool install --global wix`） | 需要 .NET SDK 6+；`windows-latest` 与本机（已装 .NET）均可 |
| 构建命令 | `wix build installer\wd-pak.wxs -o wd-pak-1.0.0-x64.msi` | 具体扩展参数（UI 扩展等）**待实机验证**并回填 |
| 安装范围 | per-machine（`InstallScope="perMachine"`、`ALLUSERS=1`） | 组策略分发、静默部署要求 per-machine；安装需管理员 |
| 平台 | `x64`（Package `Platform="x64"`） | 与目标架构一致 |
| 版本 | 与 `Cargo.toml` 一致（1.0.0），三处（tag / exe / MSI）由 M4-2 脚本校验 | 不一致即发布失败 |

#### 3.7.2 安装内容与目录布局

| 项 | 路径/名称 | 组件类型 | 说明 |
| :--- | :--- | :--- | :--- |
| 主程序 | `%ProgramFiles%\wd-pak\wd-pak.exe` | 必装，KeyPath | 唯一功能文件；单 exe 无依赖 |
| 系统 PATH | 追加 `%ProgramFiles%\wd-pak` | **可选 Feature**（默认勾选） | 用 MSI `Environment` 表写系统级 PATH；卸载时由组件移除自动清理，但不保证恢复原顺序 |
| 开始菜单 | `wd-pak 命令行` → 打开 cmd 于安装目录 | 可选 Feature | 便于不熟命令行的运维；不建桌面快捷方式 |
| ARP 记录 | 显示名 `wd-pak 1.0.0`、Publisher、`ARPNOMODIFY=1`、`ARPNOREPAIR=1` | — | 不允许「修改」流程，避免误触发 PATH 重写 |
| 许可协议 | 无（内部工具） | — | 若后续要对外分发再补 RTF 与 `WixUILicenseRtf` |

#### 3.7.3 升级与卸载策略

| 场景 | 期望行为 | 验证 |
| :--- | :--- | :--- |
| 同版本重装 | 允许，走文件替换 | `msiexec /i ... /qn` 连续两次成功，ARP 仍只有一条 |
| 升级（1.0.0 → 1.0.1） | `MajorUpgrade` 自动卸载旧版并保留 PATH 条目 | ARP 只有新版本；`wd-pak --version` 为新版本 |
| 降级（1.0.1 → 1.0.0） | 阻止（`AllowDowngrades="no"`），需先卸载 | 安装失败并提示已存在更高版本（具体错误码与提示文案**待实机验证**） |
| 卸载 | 移除 exe、目录、PATH 条目、快捷方式、ARP 记录 | 见 3.7.5 残留检查 |
| 静默卸载 | `msiexec /x wd-pak-1.0.0-x64.msi /qn` 与 `msiexec /x {ProductCode} /qn` 均可用 | 两条命令独立验证 |

#### 3.7.4 静默安装参数（运维对接用）

| 用途 | 命令 | 备注 |
| :--- | :--- | :--- |
| 静默安装（含日志） | `msiexec /i wd-pak-1.0.0-x64.msi /qn /norestart /l*v "%TEMP%\wd-pak-install.log"` | 组策略/PDQ/SCCM 直接复用 |
| 不勾选 PATH（自定义） | `msiexec /i ... /qn ADDLOCAL=MainFeature` | Feature 名以 3.7.2 定稿为准 |
| 静默卸载 | `msiexec /x wd-pak-1.0.0-x64.msi /qn /norestart` | 卸载后新开 cmd 验证 PATH 已清理 |
| 管理安装（提取文件） | `msiexec /a wd-pak-1.0.0-x64.msi /qn TARGETDIR=C:\temp\wdpak-admin` | 企业无管理员场景的兜底 |

#### 3.7.5 安装/卸载验证清单（干净虚机）

环境：Win10 22H2 x64（未装任何 VC++ Redist）、Win11 23H2+ 各一台，均无旧版 wd-pak。

| # | 步骤 | 期望结果 |
| :--- | :--- | :--- |
| 1 | 双击 MSI，UI 安装 | 安装向导可完成；无需额外运行库 |
| 2 | 新开 cmd：`wd-pak --version` | `1.0`，退出码 0 |
| 3 | `where wd-pak` | 指向 `%ProgramFiles%\wd-pak\wd-pak.exe` |
| 4 | 从 `C:\` 以外的路径执行 `wd-pak list <golden mini pak>` | 正常输出，中文无乱码 |
| 5 | `msiexec /x ... /qn` 卸载 | 退出码 0 |
| 6 | `%ProgramFiles%\wd-pak` | 目录已删除（无残留 exe） |
| 7 | 新开 cmd：`where wd-pak` | 无输出；`Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment' Path` 不含该目录 |
| 8 | 控制面板 → 程序和功能 | 无 wd-pak 记录 |
| 9 | 升级场景（3.7.3） | 仅一条 ARP 记录，PATH 不重复追加 |
| 10 | 安装日志 | 无 error 行；`msiexec` 返回 0 |

> 卸载残留检查必须以**新开进程**为准：PATH 变更对已存在的 cmd/Explorer 会话不生效。

### 3.8 验证矩阵（编译 → 冒烟 → 安装）

| 层级 | 检查内容 | 命令/入口 | 通过标准 | 环境 |
| :--- | :--- | :--- | :--- | :--- |
| L0 编译 | 静态 Release 构建 | `RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target x86_64-pc-windows-msvc` | 0 error / 0 warning | 本机 + windows-latest |
| L0 依赖 | 无 VC 运行库依赖 | `dumpbin /dependents` | 仅系统 DLL | 本机 |
| L1 单元 | `cargo test --locked --lib` | — | 全绿（含 P0-8 平台化断言） | 两 OS |
| L1 自包含集成 | `--test format_basic --test lzss_golden --test windows_smoke` | — | 全绿 | 两 OS（smoke 仅 Windows 实际执行用例） |
| L2 CLI 冒烟 | 2.6 用例表 10 条 | `cargo test --test windows_smoke` | 含 2 条整包 SHA256、1 组逐文件 SHA256、1 条回读断言 | 两 OS / windows-latest |
| L2 手工冒烟 | 3.3.1 表 8 条 | 手工 | 8/8 | Win10 22H2 虚机 |
| L3 安装 | 3.7.5 表 10 条 | `installer\verify-msi.ps1` | 10/10 | 干净虚机 |
| L4 兼容 | 无 VC++ Redist 机器运行 | `wd-pak --version` | 成功 | 干净 Win10 22H2 |
| L5 卸载 | 残留检查（文件/PATH/ARP） | PowerShell | 三项均无残留 | 干净虚机 |
| L6 发布 | tag 触发的 Release | GitHub Release 页面 | 6 个资产齐全，哈希一致 | GitHub |

### 3.9 发布流水线（`release.yml`）

| 阶段 | 作业 | 运行环境 | 产出 |
| :--- | :--- | :--- | :--- |
| 门禁 | `test`（复用现有 ubuntu 与 windows 测试） | ubuntu-latest、windows-latest | 两 OS 测试报告 |
| 版本校验 | `check-version`：`Cargo.toml` 版本 == tag（去 `v` 前缀） | ubuntu-latest | 不匹配则整条流水线失败 |
| Linux 构建 | `build-linux`（现有三目标矩阵，保持 `dist/` 命名） | ubuntu-latest | `wd-pak-x86_64-gnu` / `-musl` / `aarch64-musl` |
| Windows 构建 | `build-windows`：静态 CRT 构建 + `--version` 冒烟 | windows-latest | `wd-pak-x86_64-windows.exe` |
| MSI 构建 | `build-msi`：`dotnet tool install --global wix` → `wix build` | windows-latest | `wd-pak-1.0.0-x64.msi` |
| 发布 | `publish`：汇总产物 → 生成 `SHA256SUMS` → `gh release create` | ubuntu-latest | Release（草稿→人工确认后发布） |

发布产物的固定命名（与现有 `dist/` 体系并列）：

```
wd-pak-x86_64-windows.exe
wd-pak-1.0.0-x64.msi
wd-pak-x86_64-gnu
wd-pak-x86_64-musl
wd-pak-aarch64-musl
SHA256SUMS
```

规则：`publish` 只允许在 `test` 与版本校验全绿后运行；Release 默认以 **draft** 创建，人工检查资产后再发布，避免「CI 成功即对外」的不可逆动作（当前仓库无签名证书，误发需撤回）。

### 3.10 关键风险与缓解

| ID | 风险 | 概率 | 影响 | 缓解手段 | 验证手段 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| R1 | 文案重构误伤 POSIX 分支，Linux 输出漂移 | 中 | 高（破坏既有兼容承诺） | `unix.rs` 只做平移不改逻辑；`io_message` 的 unix 分支保持 `[Errno n] {strerror}` 原样 | ubuntu CI 全绿 + 对 `list`/`check` 的 stdout 做一次人工 diff |
| R2 | 目标 pak 被服务端进程占用，replace/pack 失败 | 高 | 中（运维中断） | 失败时给出「目标被占用」明确文案 + 退出码 2 + `.tmp` 清理；文档标注「先停服」 | 3.3.1 第 6 条（句柄占用实测） |
| R3 | 改控制台代码页产生全局副作用（退出后 cmd 仍是 65001） | 中 | 中 | `ConsoleGuard` 在正常/错误退出路径恢复旧代码页；panic 路径在文档中说明；必要时切 P2-5 方案 | 3.3.1 第 7 条 |
| R4 | 保留名/大小写/尾随空格行为与 Python-on-Windows 不一致 | 中 | 中 | Windows 上改为**显式报错**（不再静默覆盖或写 ADS），并在文档声明「有意偏离」 | P1-1/P1-2 的构造用例 |
| R5 | 无代码签名，SmartScreen 首次运行拦截、杀软误报 | 高 | 中 | Release 附 `SHA256SUMS` 与安装说明；企业内网加白；提示「更多信息 → 仍要运行」 | 干净 VM 首次运行观察 |
| R6 | 长路径仍受限（未启用 `LongPathsEnabled`） | 中 | 中 | 清单声明 `longPathAware` + 输入侧 `\\?\` 前缀；超限给明确错误 | 300 字符路径用例（2.6 / 3.8 L2） |
| R7 | 逐字节一致性无法全面自证（218 MB 大包不在仓库） | 高 | 高 | M2 用 golden mini 包覆盖 pack（压缩/不压缩）、extract 逐文件、replace 回读三条链路；大包整包对拍保留为「有资产环境的手工 L2」 | 2.6 用例表和 `layout.json` 的 SHA256 字段 |
| R8 | WiX / .NET / MSVC 版本漂移导致打包或构建失效 | 中 | 低 | 锁定 WiX 工具版本（`dotnet tool install --version`），CI 不取 latest；SDK 版本记录在 MANUAL | 连续两次 CI 运行结果一致 |
| R9 | Win32 码表覆盖不全，未知错误回退为系统文本（本地化） | 中 | 低 | 首批 12 条常见码 + 回退路径；新增码需带单测 | 码表单测 + Code Review |
| R10 | 写系统 PATH 影响宿主环境；卸载不恢复原顺序 | 中 | 中 | PATH 作为可选 Feature（默认勾选）；卸载仅移除本目录条目；文档写明 | 3.7.5 第 7 条 |
| R11 | Windows 开发机未装 VS Build Tools，`link.exe` 缺失 | 中 | 低 | MANUAL 增加环境要求与检查命令（`dumpbin`/`link.exe` 是否可用） | M0 构建命令在干净机器复现 |
| R12 | windows job 拉长 CI 时长 | 低 | 低 | 先不引入缓存；若超 5 分钟再加 `Swatinem/rust-cache` | Actions 时长记录 |

### 3.11 完成定义（DoD）

Windows 版可交付需同时满足：

1. `windows-latest` 与 `ubuntu-latest` 的 `cargo test --locked` 均 0 failed
2. 静态 Release 构建 0 warning，exe 不依赖任何 VC 运行库
3. `tests/windows_smoke.rs` 的字节断言通过（pack 两分支整包 SHA256、extract 逐文件 SHA256、replace 回读）
4. 3.3.1 手工冒烟表 8/8 通过（含 cp936 控制台与代码页恢复）
5. 3.7.5 安装验证表 10/10 通过（Win10 22H2 与 Win11 各一轮）
6. `v*` tag 能自动产出发行件并由人工确认发布
7. `MANUAL.md` / `README.md` 的 Windows 段与实际命令一致，且 `MANUAL.md:459-461` 的旧结论已更新
8. 明确记录已知限制：无签名、FAT mtime 精度、`LongPathsEnabled` 依赖、大包对拍需外部资产

### 3.12 待实机验证清单（本文中标注为「待验证」的结论）

| 项 | 验证方式 | 影响章节 |
| :--- | :--- | :--- |
| WiX v5 具体构建命令与扩展参数 | 本机执行 `wix build` | 3.7.1 |
| `wix msi validate` 的 ICE 结果是否全过 | 本机 + CI | M3-2 |
| `build.rs` 注入 `/MANIFESTINPUT` 是否与 rustc 默认清单冲突 | 构建后 `mt.exe`/`dumpbin` 检查清单 | 2.9 |
| NTFS 下 mtime 写回后读回是否严格相等 | 新增自包含用例 | 1.5、P2-4 |
| 干净 VM 上无 VC++ Redist 运行 | 3.7.5 | M3 |
| `windows-latest` 实际 MSVC/SDK 版本 | CI 日志 | 1.1 |

---

## 附录 A：术语与既有约定

| 术语 | 含义 |
| :--- | :--- |
| 逐字节一致 | PAK 二进制、LZSS 流、manifest JSON 与 Python 参考实现完全相同；由 `tests/golden/` 的 SHA256 清单锁定 |
| L1 / L2 | README 口径：L1 为日常快速门禁（`cargo test`），L2 为发布前全量长测（`cargo test --release -- --ignored`） |
| 自包含资产 | 仓库内 `tests/golden/` 下的 mini pak 与向量，无需 Python 与 218 MB 大包即可断言字节一致 |
| 外部资产 | `../wd_pak_tool.py` 与 `../release/*.pak`；位于本仓库之外，CI 与干净克隆不可得 |

## 附录 B：文档回写清单（M4 执行）

| 文件 | 需要更新的内容 |
| :--- | :--- |
| `MANUAL.md` 2.2 | 增加 Windows 构建命令（静态 CRT + MSVC） |
| `MANUAL.md` 12.4 | 用本文第 1 节的实测结论替换旧描述；保留「与 Python 唯一可感知差异」的说明并改为「已消除：控制台 UTF-8 由程序自行处理」 |
| `MANUAL.md` 8.3 | 说明 Python 垫片（`os.execv`）在 Windows 不适用，给出 PowerShell 等价方案或声明不支持 |
| `README.md` 构建/测试章节 | 增加 Windows 一键构建、`cargo test` 的平台差异说明、MSI 下载与静默安装命令 |
| `BENCHMARK.md` | 追加一节 Windows 实测（至少记录 pack/extract 单机耗时与 exe 体积） |
| `docs/windows-plan.md`（本文） | M0-M4 完成后把「待实机验证」逐条改为实测结论，并标注完成日期 |

