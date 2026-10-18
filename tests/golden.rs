//! golden 集成回归 —— 薄整合层，聚焦三块既有测试未覆盖的面：
//!
//! 1. **交互模式**（无子命令 REPL，src/interactive.rs）：同 stdin 脚本分别喂
//!    Python 与 Rust，stdout / stderr / 退出码逐字节对拍（stderr 中由 cwd 派生
//!    的 abspath 归一化为 `{CWD}` 后比对），落盘产物逐字节 / 目录树比对。
//!    覆盖实测确认的 Python 交互层特有行为：错误走 stdout、OSError → `出错:`、
//!    PakError → `错误:`、check 问题 sys.exit(1) 穿透、_ask 引号剥离序、
//!    cat 回车默认 basename 落盘（与 CLI 的 out=None 走 stdout 不同）。
//! 2. **端到端验收闭环**（AC-03/04/05 整合）：单测试串完整链路
//!    extract → pack → replace → cat 读回，各步产物与 Python 整链产物
//!    sha256 一致（区别于 write_cli.rs 的单命令 roundtrip / 向量断言）。
//! 3. **golden 向量库完整性**：golden_index.json 声明的向量文件全部存在、
//!    数量与清单行数相符（防向量漂移 / 误删）。
//!
//! 不重复既有断言：LZSS 向量（lzss_golden.rs）、CLI 对拍与全量 extract 清单
//! （read_cli.rs）、layout/replace 向量与 jobs 一致性（write_cli.rs）。
//! 长场景（etc.pak 交互 extract / etc 整链）标 #[ignore]：
//! `cargo test --test golden -- --ignored`

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// 基础辅助（与 read_cli.rs / write_cli.rs 同范式）
// ---------------------------------------------------------------------------

/// 仓库根（CARGO_MANIFEST_DIR 的上级：wd_pak_tool.py 与 release/ 所在处）下的路径
fn root(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(rel)
        .to_str()
        .unwrap()
        .to_string()
}

fn golden(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(rel)
}

/// 一次调用的结果：退出码 + 两个输出流（字节原样，不做编码转换）
type RunResult = (Option<i32>, Vec<u8>, Vec<u8>);

fn run_rust(args: &[&str]) -> RunResult {
    let out = Command::new(env!("CARGO_BIN_EXE_wd-pak"))
        .args(args)
        .output()
        .expect("启动 Rust CLI 失败");
    (out.status.code(), out.stdout, out.stderr)
}

fn run_py(args: &[&str]) -> RunResult {
    let out = Command::new("python3")
        .arg(root("wd_pak_tool.py"))
        .args(args)
        .output()
        .expect("启动 Python 基准失败");
    (out.status.code(), out.stdout, out.stderr)
}

/// 首个差异位置与上下文（大输出比对失败时的最小诊断信息）
fn first_diff(a: &[u8], b: &[u8]) -> String {
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            let ctx = |v: &[u8]| {
                let lo = i.saturating_sub(30);
                String::from_utf8_lossy(&v[lo..(i + 30).min(v.len())]).into_owned()
            };
            return format!("首个差异@{i}: rust={:?} python={:?}", ctx(a), ctx(b));
        }
    }
    if a.len() != b.len() {
        return format!("前缀一致但长度不同: rust={} python={}", a.len(), b.len());
    }
    "无差异".to_string()
}

fn sha256_hex(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_sha256(path: &Path) -> String {
    let data = fs::read(path).unwrap_or_else(|e| panic!("读取 {path:?} 失败: {e}"));
    sha256_hex(&data)
}

// ---------------------------------------------------------------------------
// 交互模式对拍辅助
// ---------------------------------------------------------------------------

/// py / rs 双实现的对拍工作目录（各自独立 cwd，产物互不干扰）
struct Pair {
    py: PathBuf,
    rs: PathBuf,
}

/// 建立干净的双 cwd 工作目录（/tmp/wdpak_golden_it/{tag}/py|rs）
fn setup_pair(tag: &str) -> Pair {
    let base = Path::new("/tmp/wdpak_golden_it").join(tag);
    for side in ["py", "rs"] {
        let d = base.join(side);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
    }
    Pair { py: base.join("py"), rs: base.join("rs") }
}

/// 向 mini pak 场景的双 cwd 放置输入文件（etc_mini pak + replace 向量 newfile）
fn place_mini_inputs(p: &Pair) {
    for side in [&p.py, &p.rs] {
        fs::copy(golden("layout/etc_mini_nocompress.pak"), side.join("mini.pak")).unwrap();
        fs::copy(golden("replace/newfile.bin"), side.join("newfile.bin")).unwrap();
    }
}

/// 喂 stdin 脚本运行一个实现（交互模式 = 无子命令启动），cwd 决定产物落点
fn run_repl(py: bool, script: &[u8], cwd: &Path) -> RunResult {
    let mut cmd = if py {
        let mut c = Command::new("python3");
        c.arg(root("wd_pak_tool.py"));
        c
    } else {
        Command::new(env!("CARGO_BIN_EXE_wd-pak"))
    };
    cmd.current_dir(cwd);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动交互进程失败");
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(script); // 脚本远小于管道缓冲，写入后随句柄 drop 关闭
    }
    let out = child.wait_with_output().expect("等待交互进程失败");
    (out.status.code(), out.stdout, out.stderr)
}

/// 交互模式对拍：同 stdin 脚本跑双实现，断言 stdout / stderr / 退出码一致，
/// artifacts（相对 cwd 路径）逐字节一致。
/// stderr 中 cwd 绝对路径归一化为 {CWD}（cat 的 `# 已导出 ... -> abspath`）。
fn assert_repl_same(p: &Pair, script: &str, artifacts: &[&str]) {
    let py = run_repl(true, script.as_bytes(), &p.py);
    let rs = run_repl(false, script.as_bytes(), &p.rs);
    assert_eq!(rs.0, py.0, "退出码不一致: rust={:?} python={:?}", rs.0, py.0);
    assert_eq!(rs.1, py.1, "stdout 不一致: {}", first_diff(&rs.1, &py.1));
    let norm = |mut err: Vec<u8>, cwd: &Path| -> Vec<u8> {
        let s = String::from_utf8_lossy(&err).replace(cwd.to_str().unwrap(), "{CWD}");
        err.clear();
        err.extend_from_slice(s.as_bytes());
        err
    };
    let py_err = norm(py.2, &p.py);
    let rs_err = norm(rs.2, &p.rs);
    assert_eq!(rs_err, py_err, "stderr(归一化) 不一致: {}", first_diff(&rs_err, &py_err));
    for f in artifacts {
        let a = fs::read(p.py.join(f)).unwrap_or_else(|e| panic!("py 侧产物缺失 {f}: {e}"));
        let b = fs::read(p.rs.join(f)).unwrap_or_else(|e| panic!("rs 侧产物缺失 {f}: {e}"));
        assert_eq!(a, b, "产物不一致: {f}");
    }
}

/// 目录树递归比对：相对路径清单一致 + 每文件字节一致
fn assert_tree_equal(a: &Path, b: &Path) {
    let collect = |root: &Path, out: &mut Vec<String>| {
        fn rec(dir: &Path, base: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let rel = e.path().strip_prefix(base).unwrap().to_string_lossy().into_owned();
                if e.file_type().unwrap().is_dir() {
                    rec(&e.path(), base, out);
                } else {
                    out.push(rel);
                }
            }
        }
        rec(root, root, out);
        out.sort();
    };
    let (mut la, mut lb) = (Vec::new(), Vec::new());
    collect(a, &mut la);
    collect(b, &mut lb);
    assert_eq!(la, lb, "目录树文件清单不一致: {a:?} vs {b:?}");
    for rel in &la {
        let x = fs::read(a.join(rel)).unwrap();
        let y = fs::read(b.join(rel)).unwrap();
        assert_eq!(x, y, "文件内容不一致: {rel}");
    }
}

/// etc.pak 绝对路径（对拍两侧同路径，stdout 中原样出现亦一致）
fn etc_pak() -> String {
    root("release/etc.pak")
}

// ---------------------------------------------------------------------------
// 交互模式：退出 / 无效选择
// ---------------------------------------------------------------------------

#[test]
fn interactive_eof_exit() {
    // EOF（stdin 立即关闭）：标题 + 菜单 + `请选择: 已退出`，exit 0
    let p = setup_pair("eof");
    assert_repl_same(&p, "", &[]);
}

#[test]
fn interactive_exit_variants() {
    // 0 / q / exit / quit 四种退出词 + 尾行无换行的 '0'（input 读到内容非 EOF）
    let p = setup_pair("exitv");
    for script in ["0\n", "q\n", "exit\n", "quit\n", "0"] {
        assert_repl_same(&p, script, &[]);
    }
}

#[test]
fn interactive_invalid_choice_retry() {
    // 无效选择 → `无效选择，请重试` 后菜单重印，不退出
    let p = setup_pair("invalid");
    assert_repl_same(&p, "x\n9\n0\n", &[]);
}

// ---------------------------------------------------------------------------
// 交互模式：list / find / check（stdout 主体 + stderr 统计行分流验证）
// ---------------------------------------------------------------------------

#[test]
fn interactive_list_flows() {
    // etc.pak（2980 文件，普通模式）stdout 全量列表 + stderr 统计行分开捕获比对；
    // mini pak 长格式（y/N 大小写：'y'/'Y' 均接受）
    let p = setup_pair("list");
    place_mini_inputs(&p);
    let etc = etc_pak();
    assert_repl_same(&p, &format!("1\n{etc}\nn\n0\n"), &[]);
    assert_repl_same(&p, "1\nmini.pak\ny\n0\n", &[]);
    assert_repl_same(&p, "1\nmini.pak\nY\n0\n", &[]);
}

#[test]
fn interactive_check_and_find() {
    let p = setup_pair("chkfind");
    place_mini_inputs(&p);
    let etc = etc_pak();
    // check：报告全走 stdout、`# 校验通过`
    assert_repl_same(&p, "6\nmini.pak\nn\n0\n", &[]);
    // find：匹配项走 stdout、`# 匹配 N 项` 走 stderr
    assert_repl_same(&p, &format!("7\n{etc}\nxiangsi\n0\n"), &[]);
}

#[test]
fn interactive_check_problems_exit1() {
    // check 发现问题：Python sys.exit(1) 的 SystemExit 穿透 except Exception，
    // 交互进程直接以 exit 1 终止（无 `已退出`）
    let p = setup_pair("chkbad");
    for side in [&p.py, &p.rs] {
        let data = fs::read(golden("layout/etc_mini_nocompress.pak")).unwrap();
        fs::write(side.join("trunc.pak"), &data[..20_000]).unwrap();
    }
    assert_repl_same(&p, "6\ntrunc.pak\nn\n0\n", &[]);
}

// ---------------------------------------------------------------------------
// 交互模式：错误分流（出错: = OSError 域 / 错误: = PakError 域，均在 stdout）
// ---------------------------------------------------------------------------

#[test]
fn interactive_oserror_prefix_chucuo() {
    // pak 打不开 → Python FileNotFoundError（非 PakError）→ `出错: [Errno 2] ...`
    let p = setup_pair("oserr");
    assert_repl_same(&p, "1\n/nonexist.pak\nn\n0\n", &[]);
    // 输出目录是已存在文件 → makedirs FileExistsError → `出错: [Errno 17]`
    let etc = etc_pak();
    for side in [&p.py, &p.rs] {
        fs::write(side.join("afile"), b"x").unwrap();
    }
    assert_repl_same(&p, &format!("2\n{etc}\nafile\n0\n"), &[]);
}

#[test]
fn interactive_pakerror_prefix_cuowu() {
    // PakError 域：cat 不存在 / pack 非目录 / replace 包内不存在 → `错误: ...`
    let p = setup_pair("pakerr");
    place_mini_inputs(&p);
    assert_repl_same(&p, "5\nmini.pak\nno_such.list\n\n0\n", &[]);
    assert_repl_same(&p, "3\n/nonexist_dir\nx.pak\nn\n0\n", &[]);
    assert_repl_same(&p, "4\nmini.pak\nno_such.list\nnewfile.bin\nn\n0\n", &[]);
}

#[test]
fn interactive_quote_stripping() {
    // _ask 剥引号：双引号 / 单引号包裹的路径照常工作；
    // 含 ' 的路径触发 OSError 消息的 repr 引号规则（双引号包裹）
    let p = setup_pair("quote");
    let etc = etc_pak();
    assert_repl_same(&p, &format!("1\n\"{etc}\"\nn\n0\n"), &[]);
    assert_repl_same(&p, "1\n'/nonexist.pak'\nn\n0\n", &[]);
    assert_repl_same(&p, "1\n\"/non'exist.pak\"\nn\n0\n", &[]);
}

// ---------------------------------------------------------------------------
// 交互模式：cat（回车默认 basename 同名落盘 —— 与 CLI out=None 走 stdout 不同）
// ---------------------------------------------------------------------------

#[test]
fn interactive_cat_default_name() {
    // 回车 → out = basename(inner.replace('\\','/'))，总是落盘；`# 已导出` 走 stderr
    let p = setup_pair("catdef");
    let etc = etc_pak();
    assert_repl_same(&p, &format!("5\n{etc}\n100heis.list\n\n0\n"), &["100heis.list"]);
    // 多级 inner：basename 取尾段（lib_gs32 的 clone/misc/mixed_agent.o → mixed_agent.o）
    let lib = root("release/lib_gs32.pak");
    assert_repl_same(&p, &format!("5\n{lib}\nclone/misc/mixed_agent.o\n\n0\n"),
                     &["mixed_agent.o"]);
}

#[test]
fn interactive_cat_quoted_out() {
    // 保存路径带引号 → _ask 剥离后使用
    let p = setup_pair("catq");
    let etc = etc_pak();
    assert_repl_same(&p, &format!("5\n{etc}\n100heis.list\n\"out.bin\"\n0\n"), &["out.bin"]);
    assert_repl_same(&p, &format!("5\n{etc}\n100heis.list\n'out3.bin'\n0\n"), &["out3.bin"]);
}

// ---------------------------------------------------------------------------
// 交互模式：extract / pack / replace（产物对拍）
// ---------------------------------------------------------------------------

#[test]
fn interactive_extract_mini() {
    // 解包产物目录树逐字节比对（含 café.list / "zuanshi -test.list" 特殊名）
    let p = setup_pair("extr");
    place_mini_inputs(&p);
    assert_repl_same(&p, "2\nmini.pak\noutdir\n0\n", &[]);
    assert_tree_equal(&p.py.join("outdir"), &p.rs.join("outdir"));
}

#[test]
fn interactive_pack_flows() {
    // 双模式 pack（y=不压缩 / n=压缩）：与 Python 交互产物逐字节一致，
    // 且与 Rust CLI pack 同参数产物逐字节一致（任务验收点）
    let p = setup_pair("pack");
    place_mini_inputs(&p);
    // 交互 extract 出源目录（两 cwd 各自独立生成）
    for py in [true, false] {
        let dir = if py { &p.py } else { &p.rs };
        let (code, _, err) = run_repl(py, b"2\nmini.pak\noutdir\n0\n", dir);
        assert_eq!(code, Some(0), "交互 extract 失败: {}", String::from_utf8_lossy(&err));
    }
    // 源目录树一致（pack 产物对拍的前提）
    assert_tree_equal(&p.py.join("outdir"), &p.rs.join("outdir"));
    // 交互 pack 双模式
    assert_repl_same(&p, "3\noutdir\ni_nc.pak\ny\n0\n", &["i_nc.pak"]);
    assert_repl_same(&p, "3\noutdir\ni_c.pak\nn\n0\n", &["i_c.pak"]);
    // CLI pack 同参数：--no-compress 与压缩默认
    let out = Command::new(env!("CARGO_BIN_EXE_wd-pak"))
        .args(["pack", "outdir", "c_nc.pak", "--no-compress"])
        .current_dir(&p.rs)
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = Command::new(env!("CARGO_BIN_EXE_wd-pak"))
        .args(["pack", "outdir", "c_c.pak"])
        .current_dir(&p.rs)
        .output()
        .unwrap();
    assert!(out.status.success());
    for (i, c) in [("i_nc.pak", "c_nc.pak"), ("i_c.pak", "c_c.pak")] {
        let a = fs::read(p.rs.join(i)).unwrap();
        let b = fs::read(p.rs.join(c)).unwrap();
        assert_eq!(a, b, "交互 pack 产物与 CLI pack 不一致: {i} vs {c}");
    }
}

#[test]
fn interactive_replace_flow() {
    // replace：backup 恒开启（.bak 产物比对）+ 双模式产物比对 + cat 读回 == newfile
    let p = setup_pair("repl");
    place_mini_inputs(&p);
    // y=不压缩 / n=压缩（jobs 由实现自定，产物与并行度无关）
    assert_repl_same(&p, "4\nmini.pak\n100heis.list\nnewfile.bin\ny\n0\n",
                     &["mini.pak", "mini.pak.bak"]);
    // 重置后再测压缩模式
    place_mini_inputs(&p);
    assert_repl_same(&p, "4\nmini.pak\n100heis.list\nnewfile.bin\nn\n0\n",
                     &["mini.pak", "mini.pak.bak"]);
    // 读回验证：CLI cat 二进制透明
    let (code, out, _) = run_rust(&["cat", p.rs.join("mini.pak").to_str().unwrap(), "100heis.list"]);
    assert_eq!(code, Some(0));
    let nf = fs::read(golden("replace/newfile.bin")).unwrap();
    assert_eq!(out, nf, "replace 后 cat 读回内容 != newfile");
}

#[test]
fn interactive_crlf_inputs() {
    // CRLF 行尾（trim 吸收 \r）与空行默认值组合
    let p = setup_pair("crlf");
    place_mini_inputs(&p);
    assert_repl_same(&p, "1\r\nmini.pak\r\nn\r\n0\r\n", &[]);
    assert_repl_same(&p, "6\r\nmini.pak\r\n\r\n0\n", &[]);
}

// ---------------------------------------------------------------------------
// 交互模式长测：etc.pak（2980 文件全量解包，双实现各 ~20s）
// ---------------------------------------------------------------------------

#[test]
#[ignore] // 长测：etc.pak 交互 extract 全量产物树比对，cargo test -- --ignored
fn interactive_extract_etc_full() {
    let p = setup_pair("etc_full");
    let etc = etc_pak();
    assert_repl_same(&p, &format!("2\n{etc}\netc_out\n0\n"), &[]);
    assert_tree_equal(&p.py.join("etc_out"), &p.rs.join("etc_out"));
}

// ---------------------------------------------------------------------------
// 端到端验收闭环（AC-03/04/05 整合）：extract → pack → replace → cat 读回
// ---------------------------------------------------------------------------

/// 双实现各跑完整链路并对拍各步产物：
/// extract 目录树 → pack pak（mode）→ replace（--no-compress --backup）→ cat 读回。
/// pak 参数为 golden mini pak（16 文件）或 etc.pak（2980 文件，调用方标 #[ignore]）。
fn assert_e2e_pipeline(tag: &str, pak: &str, inner: &str, pack_no_compress: bool) {
    let base = Path::new("/tmp/wdpak_golden_e2e").join(tag);
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let nf = golden("replace/newfile.bin").to_str().unwrap().to_string();

    let run_chain = |py: bool| -> Vec<String> {
        let side = if py { "py" } else { "rs" };
        let run = if py { run_py } else { run_rust };
        let outdir = base.join(format!("{side}_out"));
        let pak_out = base.join(format!("{side}.pak"));
        // 1) extract
        let (code, _, err) = run(&["extract", pak, outdir.to_str().unwrap()]);
        assert_eq!(code, Some(0), "{side} extract 失败: {}", String::from_utf8_lossy(&err));
        // 2) pack（模式由参数决定）
        let mut pack_args = vec!["pack", outdir.to_str().unwrap(), pak_out.to_str().unwrap()];
        if pack_no_compress {
            pack_args.push("--no-compress");
        }
        let (code, _, err) = run(&pack_args);
        assert_eq!(code, Some(0), "{side} pack 失败: {}", String::from_utf8_lossy(&err));
        // 3) replace --no-compress --backup（备份语义恒定）
        let bak = format!("{}.bak", pak_out.display());
        let (code, _, err) = run(&["replace", pak_out.to_str().unwrap(), inner, &nf,
                                    "--no-compress", "--backup"]);
        assert_eq!(code, Some(0), "{side} replace 失败: {}", String::from_utf8_lossy(&err));
        // 4) cat 读回 == newfile
        let (code, data, _) = run(&["cat", pak_out.to_str().unwrap(), inner]);
        assert_eq!(code, Some(0), "{side} cat 读回失败");
        assert_eq!(data, fs::read(&nf).unwrap(), "{side} cat 读回内容 != newfile");
        vec![file_sha256(&pak_out), file_sha256(Path::new(&bak)), sha256_hex(&data)]
    };
    let py = run_chain(true);
    let rs = run_chain(false);
    // 各步产物与 Python 整链产物 sha256 一致（pack / replace / .bak）
    assert_eq!(rs[0], py[0], "pack 产物 sha256 不一致");
    assert_eq!(rs[1], py[1], "replace 后 .bak sha256 不一致");
    assert_eq!(rs[2], py[2], "cat 读回 sha256 不一致");
    // extract 产物目录树逐字节一致
    assert_tree_equal(&base.join("py_out"), &base.join("rs_out"));
}

#[test]
fn e2e_mini_pipeline() {
    // mini pak（16 文件，压缩 pack）：快链路
    assert_e2e_pipeline("mini", golden("layout/etc_mini_nocompress.pak").to_str().unwrap(),
                        "100heis.list", false);
}

#[test]
#[ignore] // 长测：etc.pak 2980 文件全链（双实现各一次全量解压 + 打包），-- --ignored
fn e2e_etc_full_pipeline() {
    // etc.pak：no-compress pack（Python 压缩 80MB 过慢；压缩产物一致性已由
    // write_cli roundtrip 与 golden layout 向量覆盖）
    assert_e2e_pipeline("etc", &etc_pak(), "100heis.list", true);
}

// ---------------------------------------------------------------------------
// golden 向量库完整性（防向量漂移 / 误删）
// ---------------------------------------------------------------------------

#[test]
fn golden_index_files_complete() {
    let idx: Value = serde_json::from_str(
        &fs::read_to_string(golden("golden_index.json")).unwrap(),
    )
    .unwrap();
    let v = idx.get("vectors").unwrap();

    // 1) 各 section 的 index / manifest / data_dir 引用存在
    for section in ["extract", "decompress", "compress_cases", "compress_files", "layout", "replace"] {
        let s = v.get(section).unwrap_or_else(|| panic!("缺 section {section}"));
        for key in ["manifest", "index"] {
            if let Some(f) = s.get(key).and_then(Value::as_str) {
                assert!(golden(f).is_file(), "golden_index 声明的文件缺失: {f}");
            }
        }
        if let Some(d) = s.get("data_dir").and_then(Value::as_str) {
            assert!(golden(d).is_dir(), "golden_index 声明的目录缺失: {d}");
        }
    }

    // 2) extract 清单行数与声明一致
    let lines = fs::read_to_string(golden(v["extract"]["manifest"].as_str().unwrap()))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    assert_eq!(lines as u64, v["extract"]["lines"].as_u64().unwrap(), "extract 清单行数漂移");

    // 3) decompress：small 项的 stored_file / out_file 外部向量存在
    //    （large 项由 pak offset/size 现场抽取，无外部文件），数量 = small + large
    let d: Value = serde_json::from_str(
        &fs::read_to_string(golden("lzss_decompress_cases.json")).unwrap(),
    )
    .unwrap();
    let mut n = 0u64;
    for group in ["small", "large"] {
        for c in d[group].as_array().unwrap() {
            for key in ["stored_file", "out_file"] {
                if let Some(f) = c.get(key).and_then(Value::as_str) {
                    assert!(golden(f).is_file(), "解压向量文件缺失: {f}");
                }
            }
            n += 1;
        }
    }
    assert_eq!(n, v["decompress"]["small"].as_u64().unwrap() + v["decompress"]["large"].as_u64().unwrap());

    // 4) compress_files：每项的 in_file / out_file（内联项无此字段）存在，数量相符
    let cf: Value = serde_json::from_str(
        &fs::read_to_string(golden("lzss_compress_files.json")).unwrap(),
    )
    .unwrap();
    let files = cf["files"].as_array().unwrap();
    for f in files {
        for key in ["in_file", "out_file"] {
            if let Some(x) = f.get(key).and_then(Value::as_str) {
                assert!(golden(x).is_file(), "压缩向量文件缺失: {x}");
            }
        }
    }
    assert_eq!(files.len() as u64, v["compress_files"]["count"].as_u64().unwrap());

    // 5) layout：input_dir 与 paks 存在（paks 相对 layout/ 目录）
    let lay: Value = serde_json::from_str(
        &fs::read_to_string(golden("layout/layout.json")).unwrap(),
    )
    .unwrap();
    let input_dir = lay["input_dir"].as_str().unwrap();
    assert!(golden(input_dir).is_dir(), "layout 源目录缺失: {input_dir}");
    for p in v["layout"]["paks"].as_array().unwrap() {
        let f = p.as_str().unwrap();
        assert!(golden("layout").join(f).is_file(), "layout pak 向量缺失: layout/{f}");
    }

    // 6) replace：每 case 的 newfile 存在，数量相符
    let rp: Value = serde_json::from_str(
        &fs::read_to_string(golden("replace/replace.json")).unwrap(),
    )
    .unwrap();
    let cases = rp["cases"].as_array().unwrap();
    for c in cases {
        let f = c["newfile"].as_str().unwrap();
        assert!(golden(f).is_file(), "replace 向量文件缺失: {f}");
    }
    assert_eq!(cases.len() as u64, v["replace"]["cases"].as_u64().unwrap());
}