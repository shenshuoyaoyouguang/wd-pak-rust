# wd-pak 使用与部署手册

> 本手册是 `wd-pak`（问道服务端 PAK 工具 Rust 版）的完整使用文档，覆盖构建、多平台编译、全部命令用法、运维场景与从 Python 版迁移的完整方案。
>
> 快速入口：[README.md](README.md)（简介速览）｜[BENCHMARK.md](BENCHMARK.md)（性能实测）｜本文（详细手册）。
>
> 手册数据均来自 2026-10-18 本机实测（Ubuntu 24.04 / x86_64 / 4 核 / 7.4GiB 内存）。

---

## 目录

1. [项目概述](#1-项目概述)
2. [构建指南](#2-构建指南)
3. [版本选择](#3-版本选择)
4. [部署与安装](#4-部署与安装)
5. [七个子命令详解](#5-七个子命令详解)
6. [交互模式](#6-交互模式)
7. [运维场景实战](#7-运维场景实战)
8. [从 Python 版迁移](#8-从-python-版迁移)
9. [性能参考](#9-性能参考)
10. [测试与验证](#10-测试与验证)
11. [常见问题 FAQ](#11-常见问题-faq)
12. [已知边界与注意事项](#12-已知边界与注意事项)
13. [工程目录结构](#13-工程目录结构)

---

## 1. 项目概述

`wd-pak` 是问道（WUDAO）服务端 PAK 资源包的命令行工具，提供打包、解包、替换单文件、校验、搜索、导出等能力。本目录为 Python 版 `wd_pak_tool.py`（810 行）的全量 Rust 重写。

### 1.1 重写动机

| 痛点 | Python 版 | Rust 版 |
| :--- | :--- | :--- |
| LZSS 压缩吞吐（病态重复文本语料） | 0.14–0.28 MB/s | 11.1 MB/s（**40–79×**，同机对拍 85×） |
| LZSS 解压吞吐 | 4.45 MB/s | 315.5 MB/s（**71×**） |
| 218 MB 双 PAK 全量 pack | 8–26 分钟 | 约 21 秒（4 核实测） |
| 218 MB 双 PAK 全量 extract | 分钟级 | 1.79 秒 |
| 目标服务器环境 | 需安装 Python 运行时 | **单二进制，无任何运行时依赖** |

### 1.2 兼容性承诺（核心）

与 Python 版**产物逐字节兼容**，可直接替换使用：

- PAK 二进制布局：Header 16B（magic=0x11223344 LE）、目录表 64B/条（`<5I44s`）、DIR_BIT=1、RAW_BIT=0x80000000、深度优先排列；
- LZSS 压缩流：逐字节一致（86 组黄金向量校验）；
- `_pak_manifest.json`：indent=1、`ensure_ascii=False`、键序=遍历序、pack 按原 order 保序；
- CLI 输出：七个子命令的 stdout/stderr 文本、统计行、错误消息格式全部对齐；
- 退出码：`0` 成功 / `1` check 发现问题 / `2` PakError（参数、IO、格式错误）。

以上由 112 项测试（含 10744 文件全量 SHA256 清单对拍、etc.pak roundtrip 逐字节 cmp）持续保障。

---

## 2. 构建指南

### 2.1 环境要求

- Rust 工具链 ≥ 1.7x（本项目实测 1.99.1，标准 rustup 安装即可）
- 六个依赖（clap / serde / serde_json / rayon / thiserror / sha2）**全部纯 Rust、零 C 代码**，无系统库前置要求

### 2.2 各平台编译命令

```bash
cd wd-pak-rust

# ① 本机 glibc 版（动态链接，Ubuntu 22.04+ 同代系统）
cargo build --release
# 产物：target/release/wd-pak

# ② x86_64 musl 全静态版（推荐部署版，任何 x86_64 Linux 通用）
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
# 产物：target/x86_64-unknown-linux-musl/release/wd-pak

# ③ aarch64 musl 全静态版（ARM64 服务器，如鲲鹏）
rustup target add aarch64-unknown-linux-musl
RUSTFLAGS="-C linker=rust-lld" cargo build --release --target aarch64-unknown-linux-musl
# 产物：target/aarch64-unknown-linux-musl/release/wd-pak
```

### 2.3 实测产物一览

| 目标平台 | 大小 | 链接方式 | 实测状态 |
| :--- | :--- | :--- | :--- |
| `x86_64-unknown-linux-gnu` | 1.13 MB | 动态链接 glibc 2.39 | ✅ 112 项测试全绿 |
| `x86_64-unknown-linux-musl` | 1.23 MB | 全静态（static-pie） | ✅ 冒烟通过（version/check/交互），`ldd` 确认零依赖 |
| `aarch64-unknown-linux-musl` | 1.04 MB | 全静态 | ✅ 交叉编译成功（本机为 x86_64，无法运行验证，架构内逻辑相同） |

> **提示**：aarch64 交叉编译必须用 `RUSTFLAGS="-C linker=rust-lld"`——默认 `cc` 是 x86_64 链接器会报 `linking with cc failed`；rust-lld 是 rustup 自带的跨平台链接器，无需安装交叉 gcc。
>
> **Windows**：源码已预留 `#[cfg(not(unix))]` 分支（见 12.4），`x86_64-pc-windows-gnu` 目标可从 Linux 交叉编译（需安装 mingw-w64）；msvc 目标建议在 Windows 上用 VS Build Tools 构建。注意 Windows 控制台默认 GBK 编码，中文输出需 `chcp 65001`，详见 12.4。
>
> **2026-10-18 清理说明**：`target/` 已在安全清理中删除，随时按 2.2 命令重建（已实测从零离线重建 33s，产物与清理前逐字节一致）；三个已验证二进制保留在项目根 `dist/` 目录：`wd-pak-x86_64-gnu` / `wd-pak-x86_64-musl` / `wd-pak-aarch64-musl`。

---

## 3. 版本选择

### 3.1 gnu 与 musl 性能对比（218MB 全量 pack，各 2 轮交替实测）

| 版本 | 第 1 轮 | 第 2 轮 | 结论 |
| :--- | :--- | :--- | :--- |
| gnu（glibc 动态） | 21.02s | 21.29s | 快约 1.7%，噪声级差异 |
| musl（全静态） | 21.44s | 21.58s | 差异在轮间波动（±0.3s）范围内 |

两版 pack 产物**互相一致，且与黄金 etc.pak 逐字节一致**（`cmp` 验证）。本工具瓶颈在 LZSS 计算而非内存分配器，musl 的 malloc 劣势体现不出来；list/check 等微操作同为毫秒级。

### 3.2 选择建议

- **只在构建机同代系统（Ubuntu 22.04+/glibc ≥ 2.39）上用** → gnu 版：略快、略小。
- **要拷去目标服务器（本项目的主要场景）** → **musl 版**：私服环境常见 CentOS 6/7（glibc 2.12/2.17），gnu 版会报 `GLIBC_2.xx not found` 直接无法启动；musl 版零依赖，单文件拷过去即用。
- **ARM64 服务器** → aarch64-musl 版。

**一句话结论：统一用 musl 版**。用不到 2% 的性能差异，换取零 glibc 兼容性风险，一份二进制走天下。

---

## 4. 部署与安装

```bash
# 构建机侧
cargo build --release --target x86_64-unknown-linux-musl
scp target/x86_64-unknown-linux-musl/release/wd-pak user@server:/opt/wudao/bin/

# 服务器侧
chmod +x /opt/wudao/bin/wd-pak
sudo ln -sf /opt/wudao/bin/wd-pak /usr/local/bin/wd-pak   # 可选：加入 PATH

# 部署验证（30 秒完成）
wd-pak --version                 # → 1.0
wd-pak check /path/to/etc.pak    # → # 校验通过（exit 0）
echo 0 | wd-pak                  # → 交互模式可进可退（"已退出"）
```

无任何运行时依赖：不需要 Python、不需要安装库、不需要配置环境变量。

---

## 5. 七个子命令详解

统一语法：`wd-pak <子命令> [参数]`。全局选项：`--version`（显示版本号）、`-h/--help`。

### 5.1 list —— 列出包内文件

```bash
wd-pak list <pak> [-l|--long]
```

- 默认：每行一个条目路径（目录以 `DIR` 前缀标识）
- `-l`：显示 offset/size/attrib/mtime 详情

```text
$ wd-pak list etc.pak | head -3
100heis.list
110heis.list
120heis.list

# 统计行（stderr）：# 2980 files, 0 dirs, rootDirSize=190720
```

### 5.2 find —— 按文件名搜索

```bash
wd-pak find <pak> <关键字>
```

子串匹配文件名，统计行走 stderr：

```text
$ wd-pak find etc.pak heis
100heis.list
110heis.list
...
# 匹配 15 项
```

### 5.3 extract —— 解包到目录

```bash
wd-pak extract <pak> <输出目录>
```

- 输出目录中含 `_pak_manifest.json`（记录条目顺序与 mtime），**回包时依赖它保持顺序，勿删**
- 目标目录已存在时直接覆盖写入；路径被文件占用时报 `错误: [Errno 17] File exists`（exit 2）
- 统计行（stderr）：`# 解包完成: 2980 个文件 -> /path/out`

```bash
wd-pak extract etc.pak etc_out        # 2980 个文件 → etc_out/（含 manifest）
```

### 5.4 pack —— 从目录打包

```bash
wd-pak pack <输入目录> <输出pak> [--no-compress] [--jobs N]
```

- 默认**压缩模式**（LZSS，产物与 Python 版逐字节一致）；`--no-compress` 出 RAW 包（更快、体积大）
- `--jobs N`：并行压缩线程数，默认 `min(8, CPU 核数)`；**产物与 jobs 数值无关**
- 原子写：先写 `.tmp` 再 rename，中断不会产生半截 pak
- 统计行（stderr）：`# 压缩进度 N/M`（>500 文件时仅显示最终行）、`# 打包完成: N 个文件 -> out.pak`

```bash
wd-pak pack etc_out etc_new.pak                # 压缩模式
wd-pak pack etc_out etc_new.pak --no-compress  # RAW 模式
```

### 5.5 replace —— 替换包内单个文件

```bash
wd-pak replace <pak> <包内路径> <新文件> [--no-compress] [--backup]
```

- **其余未修改条目字节原样流式拷贝**（不整包载入内存，42MB 的 pak 秒级完成）
- `--backup`：先备份原 pak 为 `<pak>.bak`（含 mtime 保留，对齐 Python `shutil.copy2`）
- 统计行（stderr）：`# 已备份: x.bak`、`# 已替换 100heis.list: 19 字节 (存储 22 字节)`

```text
$ wd-pak replace etc.pak 100heis.list new.list --backup
# 已备份: etc.pak.bak
# 已替换 100heis.list: 19 字节 (存储 22 字节)
```

### 5.6 cat —— 导出单个文件

```bash
wd-pak cat <pak> <包内路径> [-o|--out <文件>]
```

- 默认输出到 stdout（可接管道）；`-o` 落盘
- 统计行（stderr）：`# 已导出 7291 字节 -> /abs/path`

```bash
wd-pak cat etc.pak 100heis.list | head -1     # → #FORM2
wd-pak cat etc.pak 100heis.list -o /tmp/x.list
```

### 5.7 check —— 校验 pak

```bash
wd-pak check <pak> [--deep]
```

- 浅校验（默认）：目录表结构、attrib 分布、越界检查，毫秒级（2980 条目约 1.5ms）
- `--deep`：全量解压校验每个条目（慢，用于发布前终检）
- 报告走 stdout（`# /path/pak`、`# files=... dirs=...`、`# attrib 分布: ...`、`# 校验通过`）
- **发现问题 → 退出码 1**；pak 无法解析 → 退出码 2

```text
$ wd-pak check etc.pak
# /path/etc.pak
# files=2980 dirs=0 rootDirSize=190720 fileSize=42285854
# attrib 分布: 0x00000000x2980
# 校验通过
```

### 5.8 退出码语义（脚本集成用）

| 退出码 | 含义 | 典型场景 |
| :--- | :--- | :--- |
| `0` | 成功 | 正常完成 |
| `1` | check 发现问题 | 条目越界等（报告仍在 stdout） |
| `2` | PakError | 参数错误、文件不存在（`错误: [Errno 2] ...`）、格式损坏 |

---

## 6. 交互模式

不带参数直接运行进入中文菜单模式（面向不熟悉命令行参数的运维人员）：

```text
$ wd-pak
问道 PAK 工具 v1.0 —— 交互模式

 1) 查看包内文件列表        2) 解包到目录
 3) 从目录打包              4) 替换包内单个文件
 5) 导出单个文件            6) 校验 pak
 7) 搜索文件名              0) 退出
请选择:
```

- 退出方式：`0` / `q` / `exit` / `quit` / 空行 / EOF（Ctrl+D）
- 输入路径可带引号（自动剥离，先双后单）；y/N 提问回车默认否
- 菜单 5（导出）回车默认保存为包内路径的同名文件到当前目录
- 错误提示区分：`错误: ...`（工具域错误）与 `出错: ...`（OS 错误），均打印到 stdout 且**不退出**，回到菜单
- 注意：交互模式中执行校验（选 6）若发现问题，进程直接以退出码 1 结束（复刻 Python `sys.exit(1)` 行为）

---

## 7. 运维场景实战

### 7.1 解包 → 批量改文件 → 回包

```bash
wd-pak extract etc.pak etc_out          # 2980 个文件 → etc_out/
vi etc_out/100heis.list                 # ...任意编辑...
wd-pak pack etc_out etc_new.pak         # 回包（依赖 manifest 保序）
wd-pak check etc_new.pak --deep         # 发布前终检
```

> 实测：`extract → pack` roundtrip 产物与原 etc.pak **逐字节一致**（cmp 验证）。

### 7.2 只替换单个文件（不动整包，推荐）

```bash
echo "patched-content-v1" > new_100heis.list
wd-pak replace etc.pak 100heis.list new_100heis.list --backup
wd-pak cat etc.pak 100heis.list          # 验证：输出 patched-content-v1
```

`--backup` 生成 `etc.pak.bak`，回滚只需 `mv etc.pak.bak etc.pak`。

### 7.3 发布前校验流水线（脚本示例）

```bash
#!/bin/bash
set -e
wd-pak check release/etc.pak --deep
wd-pak check release/lib_gs32.pak --deep
echo "全部通过，可以发布"
# check 有问题 → 退出码 1 → set -e 中止脚本
```

---

## 8. 从 Python 版迁移

### 8.1 迁移前确认（服务器上执行）

```bash
# 新旧版本对拍：产物必须完全一致
python3 wd_pak_tool.py extract etc.pak /tmp/py_out
wd-pak extract etc.pak /tmp/rs_out
diff -r /tmp/py_out /tmp/rs_out && echo "一致，可切换"
```

### 8.2 场景一：无脚本调用，直接替换

```bash
cp wd_pak_tool.py wd_pak_tool.py.bak-$(date +%F)   # 备份（回滚保险）
cp wd-pak /opt/wudao/bin/ && chmod +x /opt/wudao/bin/wd-pak
ln -sf /opt/wudao/bin/wd-pak /usr/local/bin/wd-pak
```

### 8.3 场景二：有存量脚本调 `python3 wd_pak_tool.py`（兼容垫片）

用以下内容**替换** `wd_pak_tool.py`（同目录放 `wd-pak` 二进制），存量脚本零改动：

```python
#!/usr/bin/env python3
"""兼容垫片：转发到 Rust 版 wd-pak（argv 语义完全一致），供存量脚本平滑过渡。"""
import os, sys
WD_PAK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "wd-pak")
os.execv(WD_PAK, [WD_PAK] + sys.argv[1:])
```

已实测：`python3 wd_pak_tool.py --version` → `1.0`，`find`/`check` 正常，退出码正确传递（`os.execv` 进程替换，无额外开销）。

观察运行一两周无异常后，逐步把脚本调用改为 `wd-pak` 直调，最后删除垫片——此后 Python 运行时即可从服务器下线。

### 8.4 回滚

```bash
mv wd_pak_tool.py.bak-<日期> wd_pak_tool.py    # 恢复原 Python 版即可
```

---

## 9. 性能参考

详细测量方法与全部数据见 [BENCHMARK.md](BENCHMARK.md)。摘要（本机 4 核实测，2026-10-18）：

| 指标 | Python 基线 | Rust 实测 | 加速比 |
| :--- | :--- | :--- | :--- |
| LZSS 压缩（病态语料 4MB） | 0.14–0.28 MB/s | 11.1 MB/s | 40–79×（同机对拍 85×） |
| LZSS 解压 | 4.45 MB/s | 315.5 MB/s | 71× |
| 218MB 双 PAK 全量 pack | 8–26 分钟 | 21.0s（≤30s 达标，4 核） | ~50× |
| 218MB 全量 extract | 分钟级 | 1.79s（≤30s 达标） | — |
| 2980 条目 list/find/check | ~84ms（list+check） | 5.18ms | ~16× |

性能要点：

- **压缩为多线程**（rayon，默认 `min(8, CPU)`），218MB pack 在 4 核上 21s，8 核机器更快
- **解压为单线程即 315 MB/s**，extract 场景 IO 占比更高
- 病态语料（高重复文本，56% 编码项打满 128 候选上限）是压缩最坏情况；常规文件（字体等）压缩吞吐 22.6 MB/s，随机不可压缩数据 33.0 MB/s
- 全量 extract 产物已与黄金 SHA256 清单（10744 文件）逐一对拍一致

---

## 10. 测试与验证

```bash
cd wd-pak-rust

cargo test                              # 全量回归（默认 106 项，约 35s）
cargo test --release -- --ignored       # 长测 6 项（etc/lib_gs32 全量对拍，约 3 分钟）
cargo build --release --examples        # 基准程序 lzss_bench
cargo run --release --example lzss_bench  # LZSS 吞吐基准（三语料）
```

测试体系（共 112 项，全部通过）：

| 类别 | 数量 | 覆盖内容 |
| :--- | :--- | :--- |
| 单元测试（src 内） | 53 | LZSS 边界、格式编解码、树解析安全、交互辅助、错误格式 |
| format_basic | 9 | Header/CatalogEntry/latin1/validate_name |
| lzss_golden | 3 | 86 组压缩/解压黄金向量逐字节校验 |
| read_cli | 10+1 | 五个读命令与 Python 对拍、全量 SHA256 清单（10744 行） |
| write_cli | 14+1 | pack/replace 布局向量、roundtrip、jobs 无关性 |
| golden（集成） | 17+2 | 交互模式 15 场景对拍、端到端闭环（AC-03/04/05）、向量库完整性 |

对拍方式：同一输入分别跑 Python 版与 Rust 版，比对 stdout/stderr/退出码/落盘产物逐字节一致性。

---

## 11. 常见问题 FAQ

**Q1：`GLIBC_2.xx not found` 启动失败？**
用了 gnu 版（动态链接）。换 musl 静态版（见 2.2 ②），单文件零依赖。

**Q2：pack 回去的 pak 和原 pak 不一致？**
确认用的是当初 extract 出来的目录（`_pak_manifest.json` 必须在）。删掉 manifest 后按文件名排序打包，顺序会变。正常 roundtrip（extract → pack 压缩模式）产物与原 pak 逐字节一致。

**Q3：替换一个大文件内存占用偏高？**
高熵（不可压缩）大文件走默认压缩模式时，内存约为文件大小的两倍多（LZSS 状态 + 输出缓冲，Python 版同样如此）。运维建议：高熵大文件加 `--no-compress`，实测峰值内存 67.2MB（64MB 文件）达标。

**Q4：`错误: manifest 中 ...` 报错？**
手改 `_pak_manifest.json` 时把 `mtime` 写成了 `true`。Python 版会把布尔当 1 接受，Rust 版拒绝（良性收紧），改成整数 `1` 即可。工具自己写出的 manifest 恒为整数。

**Q5：check 退出码是 1 还是 2？**
`1` = pak 能解析但条目有问题（报告在 stdout）；`2` = 无法解析（magic 错、目录表损坏等，错误消息在 stderr）。与 Python 版逐场景对拍一致。

**Q6：交互模式中文菜单乱码？**
Linux 终端需 UTF-8 locale（`locale` 命令确认）。Windows 控制台默认 GBK，需 `chcp 65001` 或使用 Windows Terminal。

**Q7：能出 Windows 版吗？**
源码已有 Windows 分支、依赖纯 Rust，可编译；但集成对拍测试体系是 Linux 专属（`/tmp` 路径 + `python3` 命令），Windows 版需单独做一轮真机对拍验证。详见 12.4。

---

## 12. 已知边界与注意事项

### 12.1 高熵大文件替换的内存口径

NFR-04 压缩模式字面超出「newfile + 10MB」：超因是逐字节兼容语义强制的 LZSS head 直索引表（32MB 惰性零页）。Python 等价实现同场景 RSS 相同（33.55 vs 33.76MB），属 design 2.11 R9 预论证的已知边界，已在 BENCHMARK.md §6 记录。主旨（未修改条目流式拷贝、禁止整包载入）实测成立；`--no-compress` 路径实测达标。

### 12.2 相邻空目录共享 offset 的自产 pak

Python 版怪癖：相邻空目录共享同一 offset。Rust 版对其读回报 TreeCycle——这是复刻怪癖的必然结果，Python 版自产 pak 同样如此，修复即违约。

### 12.3 深度与安全边界

目录树深度 ≤ 64、目录表循环引用检测、offset+size 全程 u64 算术防溢出、路径穿越检查（`..` 等）、解压禁按 org 预分配——均与 Python 版行为对齐且有专项测试。

### 12.4 Windows 平台边界

- 文件名原始字节：Unix 用 `OsStrExt` 取原始字节（保证非 UTF-8 文件名排序与 Python `listdir` 一致）；Windows 退化为 lossy 转换（design 2.11 R5）
- 控制台编码：需 `chcp 65001`，否则中文输出乱码（Python 3 用 WriteConsoleW 无此问题——这是与 Python 版唯一的可感知差异）
- 覆盖被占用文件：Linux 允许 rename 覆盖已打开文件，Windows 会失败
- 交互输入 CRLF：已兼容（`trim()` 处理 `\r`）

### 12.5 临时构建与测试产物

`/tmp/wdpak_*` 等 /tmp 下的测试、基准、对拍产物均为运行时生成，重跑 `cargo test` 或基准命令即自动重建，不属于源代码。2026-10-18 已执行两轮安全清理：

- **第一轮**：删除 `target/`（899M）、`__pycache__`（104K）与 /tmp 项目产物（3.2G），合计释放约 **4.1 GB**；清理后实测从零离线重建 release-musl 成功（33s）、产物与清理前逐字节一致。三个已验证二进制保留在项目根 `dist/` 目录（3.4M）。
- **第二轮**：/tmp 剩余散落文件按归属分类处置——A 类（本项目开发期对拍产物 280M：`pyext_lib/`、`pyext_etc/`、`trunc.pak`、`cargo_test/` 及 `py_*.out`、`e1–e5` 等，11489 项）经确认后删除；B 类（用户「悟道」逆向期历史数据 696M：`srv_*/rel_*/gs_old` 解包目录、`dwarf.txt/drv.asm` 反汇编输出等，29935 项）先打包归档为 `/root/wen1.60/wudao_reverse_archive_20261018.tar.gz`（165M，条目数与哈希抽验通过）后删除原件。清理后 /tmp 仅剩系统与工具运行时文件。

---

## 13. 工程目录结构

```text
wd-pak-rust/
├── Cargo.toml              # 6 个纯 Rust 依赖；release: lto + codegen-units=1 + strip
├── Cargo.lock              # 依赖版本锁定（构建可复现，勿删）
├── .gitignore              # 忽略 /target、/dist
├── README.md               # 项目简介速览
├── BENCHMARK.md            # 性能基准实测报告
├── MANUAL.md               # 本手册
├── dist/                   # 已验证二进制（x86_64-gnu / x86_64-musl / aarch64-musl，2026-10-18 清理时保留）
├── src/
│   ├── main.rs             # CLI 分发（clap）+ 交互模式入口
│   ├── interactive.rs      # 交互模式（逐字节复刻 Python 菜单/提示/引号剥离）
│   ├── error.rs            # PakError 层级（含 Os 变体 = Python OSError 域）
│   ├── format.rs           # Header/CatalogEntry 编解码、latin1、validate_name
│   ├── lzss.rs             # LZSS 压缩/解压（与 Python 逐字节一致）
│   ├── tree.rs             # 目录树解析（深度64/循环检测/u64 越界）
│   ├── pak.rs              # PakReader（walk/find/read_file）
│   ├── manifest.rs         # manifest 保序写出/宽松读取
│   ├── commands_read.rs    # list/find/cat/check/extract
│   └── commands_write.rs   # pack/replace（rayon 并行、.tmp 原子写）
├── examples/
│   └── lzss_bench.rs       # LZSS 吞吐基准（病态/常规/随机三语料）
├── scripts/
│   └── gen_golden.py       # 黄金向量生成（幂等）
└── tests/
    ├── format_basic.rs     # 9 项
    ├── lzss_golden.rs      # 3 项
    ├── read_cli.rs         # 10+1 项（含 lib_gs32 长测 #[ignore]）
    ├── write_cli.rs        # 14+1 项（含 roundtrip 长测 #[ignore]）
    ├── golden.rs           # 17+2 项（交互对拍 + 端到端闭环）
    └── golden/             # 黄金向量库（166 文件）
```

---

## 附录：相关文档索引

| 文档 | 内容 |
| :--- | :--- |
| `README.md` | 项目简介、构建速查、CLI 速查 |
| `BENCHMARK.md` | 测量环境、方法、NFR-01~05 逐条实测与判定 |
| `../.codeartsdoer/specs/wd-pak-rust/spec.md` | 需求规格（87 条，含 NFR 验收标准） |
| `../.codeartsdoer/specs/wd-pak-rust/design.md` | 技术设计（12 章，含兼容边界 2.10、风险 R1–R12） |
| `../wd_pak_tool.py` | Python 参考实现（行为基准，810 行） |