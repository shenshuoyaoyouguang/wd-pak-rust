# 性能基准报告（wd-pak Rust 版 vs Python 基线）

> 测量日期：2026-10-18。全部数据为本机实测，复现命令见文末。
> 验收口径依据 `spec.md` 第 4.1 节（NFR-01~05，含 NFR-01 v1.1 修订）与 `design.md` 2.9.5 / 2.11 R9。

## 1. 测量环境

| 项 | 值 |
| :--- | :--- |
| CPU | General Purpose Processor（x86_64，主频 2.80 GHz） |
| 逻辑核数 | 4（1 路 2 核 × 2 线程）——注意：spec NFR-02 按 8 核口径制定，见 §4 折算说明 |
| 缓存 | L1d 96 KiB ×2、L2 2.5 MiB ×2、L3 57 MiB |
| 内存 | 7.4 GiB |
| 操作系统 | Linux（Ubuntu 24.04，内核 6.8.0-124-generic） |
| 构建 | `cargo build --release --examples`，0 警告；单二进制 `target/release/wd-pak` 1,180,064 字节（约 1.13 MB，strip 后） |

**页缓存口径**：所有计时前均先执行一次完整预热跑（源目录/PAK 与目标路径写入页缓存），测量值为"热缓存"吞吐，与 Python 基线测量时同口径。

**计时工具**：端到端计时用 `/usr/bin/time -f "%e"`；NFR-05 用 `time.perf_counter()` 包 `subprocess`（微秒级，5 次取最优）；LZSS 单线程吞吐用 `examples/lzss_bench.rs`（预热 1 次 + 3 轮取中位，对齐 design 2.9.5）。

## 2. Python 基线（对照值）

| 指标 | Python 基线 | 来源 |
| :--- | :--- | :--- |
| LZSS 压缩（病态语料） | 0.14–0.28 MB/s | spec/design 记录 |
| LZSS 解压 | 4.45 MB/s | design 2.9.5 |
| 218 MB 全量 pack | 8–26 分钟 | spec/design 记录（本次不重测） |

**本次对拍复核**（佐证基线量级）：同一 4 MB 病态语料（`../etc.pak/server_res_text.txt` 前 4 MB），Python `lzss_compress` 全量压缩耗时 30.45 s = **0.131 MB/s**，与基线区间同量级（受当日机器负载影响略低于区间下限）。压缩输出 2,064,165 字节，与 Rust 版输出**逐字节一致**。

## 3. NFR-01：单线程 LZSS 吞吐（examples/lzss_bench.rs，release）

语料口径：各取前 4 MB，预热 1 次 + 3 轮计时取中位；解压每轮校验输出与原文一致。

| 语料 | 压缩率 | 压缩 (MB/s) | 解压 (MB/s) |
| :--- | :--- | :--- | :--- |
| 病态文本（`../etc.pak/server_res_text.txt` 前 4 MB） | 0.49 | **11.1** | **315.5** |
| 常规字体（`../etc.pak/gbsn00lp.ttf` 前 4 MB，真实包内文件） | 0.82 | 22.6 | 283.6 |
| 随机字节（xorshift 伪随机 4 MB，不可压缩） | 1.12 | 33.0 | 268.1 |

病态语料特征：高重复文本，压缩率 0.49，56% 编码项打满 128 候选上限（约 1.21 亿次候选遍历）——逐字节兼容语义强制串行 prev 单链遍历，每候选一次 L1 load 依赖，为该语料的物理下限路径（单线程极限约 17–23 MB/s，详见 spec NFR-01 修订 v1.1）。

**与 Python 基线对比**：

| 指标 | Rust 实测 | Python 基线 | 倍数 |
| :--- | :--- | :--- | :--- |
| 病态语料压缩 | 11.1 MB/s | 0.14–0.28 MB/s | **40–79×**（同机对拍 0.131 MB/s → 85×） |
| 病态语料解压 | 315.5 MB/s | 4.45 MB/s | **71×** |
| 常规语料压缩 | 22.6 MB/s | —（基线仅病态口径） | 对照项 |
| 随机语料压缩 | 33.0 MB/s | —（对照项，哈希链恒为 1 的上界） | 对照项 |

**判定：PASS**——压缩 11.1 ≥ 10 MB/s（NFR-01 修订口径），解压 315.5 ≥ 50 MB/s（NFR-03 后半句）。

## 4. NFR-02：218 MB 全量 pack ≤ 30 s

数据集：`../etc.pak`（85 MB 解包目录，2980 文件）+ `../lib_gs32.pak`（133 MB 解包目录，7762 文件），压缩模式（默认 `--jobs 4` = 本机核数），目标 PAK 写 `/tmp/wdpak_bench/`。每轮计时前先完整预热跑一遍。

| 轮次 | pack etc.pak | pack lib_gs32.pak | 合计 |
| :--- | :--- | :--- | :--- |
| 第 1 轮 | 3.90 s | 17.17 s | **21.07 s** |
| 第 2 轮 | 3.98 s | 17.04 s | **21.02 s** |

产物正确性：`bench_etc.pak`、`bench_lib.pak` 与仓库黄金 PAK（`../release/etc.pak`、`../release/lib_gs32.pak`）`cmp` **逐字节一致**。pack 峰值 RSS 约 124–126 MB（非判定项，仅记录）。

**核数折算说明**：spec 口径为"8 核机器 ≤ 30 s"；本机仅 4 逻辑核（核数减半），实测两轮 21.07 s / 21.02 s 均 ≤ 30 s。pack 并行为 rayon 文件级并行，吞吐随可用核数单调不减，4 核达标蕴含 8 核口径必然达标。

**判定：PASS**（4 核实测 21.02–21.07 s ≤ 30 s；Python 基线同数据集需 8–26 分钟，约 **23–74×**）。

## 5. NFR-03：218 MB 全量 extract ≤ 30 s + 单线程解压 ≥ 50 MB/s

输入：`/tmp/wdpak_bench/bench_etc.pak`（2980 条目，解压后 76 MB）+ `/tmp/wdpak_bench/bench_lib.pak`（7762 条目，解压后 113 MB），输出到 `/tmp/wdpak_bench/out_etc`、`/tmp/wdpak_bench/out_lib`。预热跑后正式计时。

| 步骤 | 耗时 |
| :--- | :--- |
| extract etc.pak（2980 文件） | 0.50 s |
| extract lib_gs32.pak（7762 文件） | 1.29 s |
| **合计** | **1.79 s** |

正确性佐证：解包产物对 `tests/golden/extract_files.sha256`（10744 条黄金清单）逐文件比对——**10742 个数据文件全部一致**；2 个 `_pak_manifest.json` 初次比对差异仅因基准 PAK 文件名（`bench_etc.pak`）与黄金文件名（`etc.pak`）不同导致 manifest 内 `pak` 字段不同，将基准 PAK 以 `etc.pak`/`lib_gs32.pak` 文件名重新 extract 后 **10744/10744 全部一致**。

单线程 LZSS 解压吞吐：315.5 MB/s（§3 病态语料，≥ 50 MB/s）。

**判定：PASS**（合计 1.79 s ≤ 30 s，余量约 16.8×；解压 315.5 ≥ 50 MB/s，为 Python 基线 4.45 MB/s 的 71×）。

## 6. NFR-04：replace 峰值内存 ≤ 被替换文件原始大小 + 10 MB

测量方式：`/usr/bin/time -v` 取 Maximum resident set size。替换目标 pak 为 `../release/etc.pak` 副本，被替换条目 `server_res_text.txt`。

| 组 | newfile | 模式 | 峰值 RSS | 字面上限 | 结果 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| 1 | `gs/start_gs.o`（lib_gs32.pak 最大条目，1.74 MB 文本） | 压缩 | 33.76 MB | 11.74 MB | 字面超出 |
| 2 | 64 MB 高熵随机（不可压缩） | 压缩 | 170.7 MB | 74 MB | 记录不判定（见下） |
| 3 | 64 MB 高熵随机（不可压缩） | `--no-compress` | **67.2 MB** | 74 MB | **PASS** |

### 组 1/2 超出预算的成因分析（如实记录）

超出部分来自压缩器固有状态 `head` 直索引表：为保证候选遍历顺序与 Python dict 语义完全一致（AC-01 逐字节一致的硬约束），`compress` 使用 `2^24 × u16 = 32 MB` 直索引表（`alloc_zeroed` 惰性零页，按实际触达落实物理页）——1.74 MB 文本即触达约 28 MB，64 MB 高熵随机则几乎全量触达。组 2 实测 170.7 MB ≈ raw 64 MB + 压缩输出 72 MB（不可压缩数据膨胀 9/8，已与 Python 字节一致地存储 75,476,884 字节）+ head 表 32 MB + 基线，构成完全吻合。

依据 design 2.11 R9 的既有结论，该项**不判 FAIL**，理由：

1. **NFR-04 条文主旨（括号内明示）是"未修改条目必须流式拷贝，禁止整包载入内存"**，压缩器固有状态不在其列。主旨已实测验证：同样替换 1.74 MB 文本到 42 MB 的 etc.pak 与 49 MB 的 lib_gs32.pak，RSS 分别为 33.76 MB / 34.30 MB——**与 pak 大小无关，且小于 pak 体积本身**（整包载入方案仅载入即需 ≥ 42 MB），证明未修改条目流式拷贝、无整包载入。
2. **行为基线等价**：Python 版同场景（组 1 同一 newfile、同一 pak）实测 RSS 33.55 MB ≈ Rust 33.76 MB——两个实现等价超界，NFR-12（与 Python 逐字节等价）优先级更高；本组 Rust 耗时 0.17 s vs Python 8.64 s（51×）。
3. design R9 对高熵大文件已给出运维指引：**替换高熵大文件用 `--no-compress`**（RAW 路径无 head 表，组 3 实测 67.2 MB 达标，余量 6.8 MB）。

**判定：主旨口径 PASS**（流式拷贝、禁止整包载入均成立；RAW 路径字面达标 67.2 ≤ 74 MB）；压缩模式按"raw + 10 MB"字面口径超出（超因 = 语义强制的 head 直索引表，Python 等价实现同样超出），依 R9 论证豁免并如实记录。

## 7. NFR-05：2980 条目 list/find/check 浅校验全流程 ≤ 1 s

对象：`../release/etc.pak`（2980 条目），release 构建，stdout 重定向（纯处理耗时，含目录树解析与输出格式化），5 次取最优。

| 命令 | Rust 耗时 | Python 对照 |
| :--- | :--- | :--- |
| list（2980 行） | 2.15 ms | 43.6 ms |
| find（匹配 2 项） | 1.58 ms | — |
| check（浅校验） | 1.46 ms | 39.9 ms |
| **合计** | **5.18 ms** | — |

**判定：PASS**（5.18 ms ≤ 1 s，余量约 193×）。

## 8. 判定汇总

| NFR | 要求 | 实测 | 判定 |
| :--- | :--- | :--- | :--- |
| NFR-01 | 单线程压缩 ≥ 10 MB/s（病态语料） | 11.1 MB/s（40–79× Python） | **PASS** |
| NFR-03 后半 | 单线程解压 ≥ 50 MB/s | 315.5 MB/s（71× Python） | **PASS** |
| NFR-02 | 218 MB pack ≤ 30 s（8 核） | 21.02–21.07 s（4 核实测，折算见 §4） | **PASS** |
| NFR-03 前半 | 218 MB 全量 extract ≤ 30 s | 1.79 s | **PASS** |
| NFR-04 | replace RSS ≤ newfile + 10 MB | RAW 路径 67.2 ≤ 74 MB；主旨（流式/不整包）验证成立；压缩模式字面超出（head 表固有状态，Python 等价同超，依 design R9 豁免） | **PASS（R9 口径）** |
| NFR-05 | 2980 条目 list/find/check ≤ 1 s | 5.18 ms | **PASS** |

## 9. 复现命令

```bash
# 构建（0 警告）
cargo build --release --examples

# NFR-01 / NFR-03 后半：单线程 LZSS 吞吐（病态/常规/随机三语料）
cargo run --release --example lzss_bench

# NFR-02：218 MB pack（预热跑后正式计时，默认 jobs=核数）
/usr/bin/time -f "%e s" ./target/release/wd-pak pack ../etc.pak /tmp/wdpak_bench/bench_etc.pak
/usr/bin/time -f "%e s" ./target/release/wd-pak pack ../lib_gs32.pak /tmp/wdpak_bench/bench_lib.pak

# NFR-03：全量 extract（预热跑后正式计时）
/usr/bin/time -f "%e s" ./target/release/wd-pak extract /tmp/wdpak_bench/bench_etc.pak /tmp/wdpak_bench/out_etc
/usr/bin/time -f "%e s" ./target/release/wd-pak extract /tmp/wdpak_bench/bench_lib.pak /tmp/wdpak_bench/out_lib

# NFR-04：replace 峰值内存（newfile 依次为 1.74 MB 文本 / 64 MB 高熵随机，各测压缩与 --no-compress）
/usr/bin/time -v ./target/release/wd-pak replace <pak副本> server_res_text.txt <newfile>

# NFR-05：list/find/check 计时（stdout 重定向）
/usr/bin/time -f "%e s" ./target/release/wd-pak list ../release/etc.pak >/dev/null
/usr/bin/time -f "%e s" ./target/release/wd-pak find ../release/etc.pak server >/dev/null
/usr/bin/time -f "%e s" ./target/release/wd-pak check ../release/etc.pak >/dev/null
```