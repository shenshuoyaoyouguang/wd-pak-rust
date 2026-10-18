# wd-pak —— 问道服务端 PAK 打包/解包工具（Rust 版）

`wd-pak` 是问道（WUDAO）服务端 PAK 资源包的命令行工具，提供打包、解包、替换、校验等能力。本目录为 Python 版 `wd_pak_tool.py` 的全量 Rust 重写，**与 Python 版产物逐字节兼容**（PAK 二进制、LZSS 压缩流、manifest 均完全一致），可直接替换使用。

## 重写动机

| 痛点 | Python 版 | Rust 版 |
| :--- | :--- | :--- |
| LZSS 压缩吞吐（病态重复文本语料） | 0.14–0.28 MB/s | 11.1 MB/s（**40–79×**） |
| LZSS 解压吞吐 | 4.45 MB/s | 315.5 MB/s（**71×**） |
| 218 MB 双 PAK 全量 pack | 8–26 分钟 | 约 21 秒（4 核实测） |
| 218 MB 双 PAK 全量 extract | 分钟级 | 1.79 秒 |
| 目标服务器环境 | 需要安装 Python 运行时 | **单二进制，无运行时依赖**（约 1.13 MB，静态链接常规 glibc） |

## 构建

```bash
cargo build --release
# 产物：target/release/wd-pak（单二进制，0 警告）
```

基准程序（可选）：`cargo build --release --examples`，产物 `target/release/examples/lzss_bench`。

## CLI 用法

七个子命令与 Python 版参数完全一致（`--long/-l`、`--no-compress`、`--jobs N`、`--backup`、`--out/-o`、`--deep`）：

```bash
# 列出包内文件（-l 显示 offset/size/attrib/mtime 详情）
wd-pak list <pak> [-l]
wd-pak list release/etc.pak -l

# 按关键字搜索包内文件名
wd-pak find <pak> <keyword>
wd-pak find release/etc.pak server

# 解包到目录（生成 _pak_manifest.json 记录条目顺序）
wd-pak extract <pak> <outdir>
wd-pak extract release/etc.pak /tmp/out_etc

# 从目录打包（默认压缩；--jobs 并行数默认=CPU 核数，1=单线程）
wd-pak pack <indir> <pak> [--no-compress] [--jobs N]
wd-pak pack /tmp/out_etc /tmp/new.pak --jobs 4

# 替换包内单个文件（其余条目字节原样流式保留；--backup 先生成 .bak）
wd-pak replace <pak> <inner> <newfile> [--no-compress] [--backup]
wd-pak replace release/etc.pak server_res_text.txt new.txt --backup

# 导出单个文件（默认写 stdout，-o 保存到文件）
wd-pak cat <pak> <inner> [-o <file>]
wd-pak cat release/etc.pak server_res_text.txt -o res.txt

# 校验 pak（默认浅校验：目录树 + 条目元数据；--deep 全量解压验证）
wd-pak check <pak> [--deep]
wd-pak check release/etc.pak --deep
```

**交互模式**：无参数启动进入菜单（对齐 Python 版），提供列表/解包/打包/替换/导出/校验/搜索七项功能，输入 `0`/`q`/`exit`/`quit` 或空行退出：

```bash
wd-pak            # 进入交互模式
echo 0 | wd-pak   # 非交互环境下直接退出（"已退出"）
```

**退出码语义**（与 Python 版一致）：`0` 成功；`1` check 发现问题；`2` 任何 PakError（错误消息 `错误: {消息}` 输出到 stderr，交互模式内为 `错误:`/`出错:` 输出到 stdout 不退出）。

**版本**：`wd-pak --version` 输出 `1.0`。

## 与 Python 版的兼容性

- **逐字节一致**（NFR-12）：LZSS 压缩流、解压结果、`--no-compress` pack 布局、replace 产物、extract 文件清单（含 SHA256 与 `_pak_manifest.json`）均与 Python 参考实现逐字节相同——由黄金向量测试套件（`tests/golden/`，10744 条 SHA256 清单）锁定。
- **CLI 兼容**（NFR-13）：子命令、参数名、错误与提示消息文本（对齐 Python argparse 与附录 A 消息清单）、退出码语义完全一致。
- **格式怪癖原样复刻**：如相邻空目录共享 offset 导致读回 TreeCycle 误报等 Python 原版行为，均按"逐字节一致优先"原则复刻而非修复（详见 design 2.10）。

## 测试

```bash
# L1 快速门禁（日常必跑）：单元 + 黄金向量小/中用例 + 损坏输入 + CLI
cargo test

# L2 全量长测（发布前）：双 PAK（etc.pak 85MB + lib_gs32.pak 133MB）extract/pack/replace
# 全量差分、256KB 随机压缩向量、--deep check
cargo test --release -- --ignored
```

L1 共 112 项（106 passed + 6 ignored——ignored 均为 L2 长测项，需按上述命令显式运行）。

## 性能

详细数据、测量方法与 NFR-01~05 逐条判定见 [BENCHMARK.md](BENCHMARK.md)；构建、部署、多平台编译、迁移与常见问题详见 [MANUAL.md](MANUAL.md)。摘要：

| 指标 | Rust（本机 4 核实测） | Python 基线 |
| :--- | :--- | :--- |
| LZSS 压缩（病态语料） | 11.1 MB/s | 0.14–0.28 MB/s |
| LZSS 解压 | 315.5 MB/s | 4.45 MB/s |
| 218 MB pack（压缩） | 21.0 s | 8–26 分钟 |
| 218 MB extract | 1.79 s | — |
| 2980 条目 list/find/check | 5.18 ms | — |

已知边界：替换高熵（不可压缩）大文件且用压缩模式时，峰值内存受语义强制的 head 直索引表（32 MB 惰性分配）影响超出"newfile + 10 MB"预算（Python 版等价实现同样超出，详见 BENCHMARK.md §6 与 design 2.11 R9）；此时建议 `--no-compress`（RAW 路径实测 64 MB newfile 峰值 67.2 MB，达标）。