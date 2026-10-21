//! 写路径 CLI 集成测试 —— pack/replace 与 Python 基准（wd_pak_tool.py）逐字节比对。
//!
//! 比对策略：
//! - 错误路径（两实现均不落盘）：同参数直调，stderr/退出码逐字节一致
//! - 成功路径：两实现写"等长不同"的目标路径（仅中段 py/rs 标记不同），
//!   stderr 将各自标记归一化为占位符后逐字节比对，产物 pak/.bak 逐 sha256 比对
//! - golden：layout（etc_mini 双模式）与 replace 三用例直接断言 sha256
//!   （golden 文件由 Python 产物生成，相等即逐字节一致）
//! - replace --backup 的 .bak 额外校验 mtime 继承语义（对齐 shutil.copy2 的 copystat）
//!
//! 复刻的关键 Python 行为（对拍覆盖）：
//! - 纯空目录树：相邻空目录共享 offset → 自产 pak 读回触发 TreeCycle
//! - 文件名校验三段序：长度 → 非法字符(反斜杠) → latin1 域（café 合法、中文报错）
//! - manifest：order 排序 / mtime 恢复（非法值报错）/ 坏 JSON 宽松忽略 /
//!   `_pak_manifest.json` 目录形态（preflight 递归校验其内容，collect 排除）
//! - 压缩进度 `# 压缩进度 {done}/{total}`（jobs=1 单线程与 jobs=4 进程池行序一致）
//! - 写出失败：`写出 pak 失败: [Errno 21] Is a directory: '<tmp>' -> '<out>'`，.tmp 清理
//!
//! etc.pak 的 extract→pack roundtrip 为 #[ignore] 长测：cargo test -- --ignored。
//! 文本输出在 Windows 上归一化 CRLF 与 Win32/Errno 前缀；PAK 产物仍逐字节严格。
//! 临时产物位于 `WDPAK_TEST_TMP` 或系统临时目录，重复运行覆盖。
//! 缺少 `release/*.pak` 时相关用例显式 skip；`WDPAK_REQUIRE_FIXTURES=1` 可强制失败。

use std::fs::{self, FileTimes};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde_json::Value;
use sha2::{Digest, Sha256};

mod common;

// ---------------------------------------------------------------------------
// 基础辅助（与 read_cli.rs 同范式）
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

fn golden(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(rel)
        .to_str()
        .unwrap()
        .to_string()
}

/// 一次 CLI 调用的结果：退出码 + 两个输出流（字节原样，不做编码转换）
type RunResult = (Option<i32>, Vec<u8>, Vec<u8>);

fn run_rust(args: &[&str]) -> RunResult {
    let out = Command::new(env!("CARGO_BIN_EXE_wd-pak"))
        .args(args)
        .output()
        .expect("启动 Rust CLI 失败");
    (out.status.code(), out.stdout, out.stderr)
}

fn run_py(args: &[&str]) -> RunResult {
    let out = common::python_command()
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

/// 同参数调用两实现，断言文本 stdout/stderr/退出码一致。
/// 仅用于两实现都不落盘的场景（写路径 = 全部错误路径）。
fn assert_same_as_python(args: &[&str]) {
    let py = run_py(args);
    let rs = run_rust(args);
    let rs_out = common::normalize_text(&rs.1);
    let py_out = common::normalize_text(&py.1);
    assert_eq!(
        rs_out,
        py_out,
        "stdout 不一致 {args:?}: {}",
        first_diff(rs_out.as_bytes(), py_out.as_bytes())
    );
    let rs_err = common::normalize_text(&rs.2);
    let py_err = common::normalize_text(&py.2);
    assert_eq!(
        rs_err,
        py_err,
        "stderr 不一致 {args:?}: {}",
        first_diff(rs_err.as_bytes(), py_err.as_bytes())
    );
    assert_eq!(
        rs.0, py.0,
        "退出码不一致 {args:?}: rust={:?} python={:?}",
        rs.0, py.0
    );
}

// ---------------------------------------------------------------------------
// 写路径专用辅助
// ---------------------------------------------------------------------------

/// 等长目标路径对：py 与 rs 的目标路径仅中段标记不同，
/// stderr 中出现的目标路径（含 .tmp / .bak 派生路径）归一化后可逐字节比对。
struct Pair {
    py: String,
    rs: String,
}

const PY_MARK: &str = "wdpak_wtest_py_";
const RS_MARK: &str = "wdpak_wtest_rs_";

fn pair(name: &str) -> Pair {
    Pair {
        py: common::temp_str(format!("{PY_MARK}{name}")),
        rs: common::temp_str(format!("{RS_MARK}{name}")),
    }
}

/// stderr 归一化：平台文本差异 + 目标路径标记替换为占位符
fn norm(b: &[u8], mark: &str) -> String {
    common::normalize_text(b).replace(mark, "<OUT>")
}

fn skip_without_etc() -> bool {
    common::skip_without_fixture(Path::new(&etc_pak()), "release/etc.pak")
}

fn skip_without_lib() -> bool {
    common::skip_without_fixture(Path::new(&lib_pak()), "release/lib_gs32.pak")
}

/// 断言退出码一致 + stderr（各自归一化后）逐字节一致
fn assert_run_match(py: &RunResult, rs: &RunResult) {
    let py_err = norm(&py.2, PY_MARK);
    let rs_err = norm(&rs.2, RS_MARK);
    assert_eq!(
        rs.0, py.0,
        "退出码不一致: rust={:?} python={:?}\nrust stderr:\n{rs_err}",
        rs.0, py.0
    );
    assert_eq!(
        rs_err,
        py_err,
        "stderr 不一致:\n-- rust --\n{rs_err}\n-- python --\n{py_err}\n{}",
        first_diff(rs_err.as_bytes(), py_err.as_bytes())
    );
}

/// 两实现 pack 同一 indir 到等长不同路径，断言 stderr/退出码/产物 sha256 一致。
/// 返回路径对（供读回验证）；错误场景（py 退出码非 0）不比对产物。
fn assert_pack_same(indir: &str, extra: &[&str], tag: &str) -> Pair {
    let p = pair(tag);
    let mut a = vec!["pack", indir, p.py.as_str()];
    a.extend(extra.iter().copied());
    let py = run_py(&a);
    let mut b = vec!["pack", indir, p.rs.as_str()];
    b.extend(extra.iter().copied());
    let rs = run_rust(&b);
    assert_run_match(&py, &rs);
    if py.0 == Some(0) {
        assert_eq!(
            file_sha256(Path::new(&p.rs)),
            file_sha256(Path::new(&p.py)),
            "pack 产物 sha256 不一致: indir={indir}"
        );
    }
    p
}

/// 两实现 replace 同一源 pak 的等长副本，断言 stderr/退出码/产物 pak sha256 一致；
/// --backup 时额外断言 .bak sha256 一致且等于替换前的源 pak。
fn assert_replace_same(src: &str, inner: &str, newfile: &str, extra: &[&str], tag: &str) -> Pair {
    let p = pair(tag);
    fs::copy(src, &p.py).unwrap();
    fs::copy(src, &p.rs).unwrap();
    let mut a = vec!["replace", p.py.as_str(), inner, newfile];
    a.extend(extra.iter().copied());
    let py = run_py(&a);
    let mut b = vec!["replace", p.rs.as_str(), inner, newfile];
    b.extend(extra.iter().copied());
    let rs = run_rust(&b);
    assert_run_match(&py, &rs);
    if py.0 == Some(0) {
        assert_eq!(
            file_sha256(Path::new(&p.rs)),
            file_sha256(Path::new(&p.py)),
            "replace 产物 sha256 不一致: inner={inner}"
        );
        if extra.contains(&"--backup") {
            let (bpy, brs) = (format!("{}.bak", p.py), format!("{}.bak", p.rs));
            assert_eq!(
                file_sha256(Path::new(&brs)),
                file_sha256(Path::new(&bpy)),
                ".bak sha256 不一致"
            );
            assert_eq!(
                file_sha256(Path::new(&bpy)),
                file_sha256(Path::new(src)),
                ".bak 应等于替换前的源 pak（shutil.copy2 语义）"
            );
        }
    }
    p
}

// ---------------------------------------------------------------------------
// fixture 构造
// ---------------------------------------------------------------------------

fn write_file(path: &str, data: &[u8]) {
    if let Some(d) = Path::new(path).parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(path, data).unwrap();
}

/// 固定种子伪随机字节（测试数据可复现）
fn lcg(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as u8
        })
        .collect()
}

/// 设置文件 mtime（构造 shutil.copy2 语义验证的固定旧时间戳）
fn set_mtime(path: &str, t: SystemTime) {
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_times(FileTimes::new().set_modified(t)).unwrap();
}

fn etc_pak() -> String {
    root("release/etc.pak")
}

fn lib_pak() -> String {
    root("release/lib_gs32.pak")
}

/// 测试用 newfile（固定内容，避免依赖外部文件）
fn newfile() -> String {
    let p = common::temp_str("wdpak_wtest_newfile.bin");
    write_file(&p, &lcg(0x5eed, 4500));
    p
}

// ---------------------------------------------------------------------------
// pack：基本场景
// ---------------------------------------------------------------------------

#[test]
fn pack_basic_matches_python() {
    // 嵌套目录 + 空目录 + 空文件 + 大文件 + latin1 名（café）+ 超长名；
    // 三种场景：默认压缩 / --no-compress。
    // Windows 受 MAX_PATH 限制，超长名取 150 字节（已远超 PAK 的 43 字节上限）。
    let indir = common::temp_str("wdpak_wtest_basic");
    write_file(&format!("{indir}/a.txt"), b"hello pak\n");
    write_file(&format!("{indir}/big.bin"), &lcg(1, 100_000));
    write_file(&format!("{indir}/empty.txt"), b"");
    write_file(&format!("{indir}/{}", "caf\u{e9}.list"), b"latin1 name\n"); // café.list
    let long_name_len = if cfg!(windows) { 150 } else { 255 };
    write_file(&format!("{indir}/{}", "l".repeat(long_name_len)), b"255");
    write_file(&format!("{indir}/sub/nested.txt"), b"nested\n");
    write_file(&format!("{indir}/sub/deep/d.bin"), &lcg(2, 300));
    fs::create_dir_all(format!("{indir}/emptydir/inner")).unwrap();

    assert_pack_same(&indir, &[], "basic_c");
    assert_pack_same(&indir, &["--no-compress"], "basic_nc");
}

#[test]
fn pack_empty_dirs_only_tree_cycle() {
    // 纯空目录树：相邻空目录共享 offset（Python 布局怪癖）→
    // 自产 pak 读回触发 TreeCycle；两实现产物的 list 输出一致即证明怪癖复刻
    let indir = common::temp_str("wdpak_wtest_empty");
    fs::create_dir_all(format!("{indir}/d1")).unwrap();
    fs::create_dir_all(format!("{indir}/d2/d2sub")).unwrap();
    fs::create_dir_all(format!("{indir}/d3")).unwrap();

    let out = assert_pack_same(&indir, &[], "empty");
    // 读回：py list 自产 pak vs rs list 自产 pak（TreeCycle 路径的输出/退出码一致）
    let py = run_py(&["list", &out.py]);
    let rs = run_rust(&["list", &out.rs]);
    let py_out = common::normalize_text(&py.1);
    let rs_out = common::normalize_text(&rs.1);
    assert_eq!(
        rs_out,
        py_out,
        "自产 pak 的 list stdout 不一致: {}",
        first_diff(rs_out.as_bytes(), py_out.as_bytes())
    );
    let py_err = common::normalize_text(&py.2);
    let rs_err = common::normalize_text(&rs.2);
    assert_eq!(
        rs_err,
        py_err,
        "自产 pak 的 list stderr 不一致: {}",
        first_diff(rs_err.as_bytes(), py_err.as_bytes())
    );
    assert_eq!(rs.0, py.0);
}

// ---------------------------------------------------------------------------
// pack：文件名校验（错误路径，同参数对拍）
// ---------------------------------------------------------------------------

#[test]
fn pack_invalid_names_match_python() {
    // 文件名含非 ASCII（超出 latin1 域，如中文）——所有平台
    let d1 = common::temp_str("wdpak_wtest_cn");
    write_file(&format!("{d1}/\u{4e2d}\u{6587}.txt"), b"x"); // 中文.txt
    let err1 = common::temp_str("wdpak_wtest_err1.pak");
    assert_same_as_python(&["pack", d1.as_str(), err1.as_str()]);

    // 以下用例依赖 Unix 文件名语义：Windows 不允许文件名含反斜杠，
    // 且 NTFS 大小写不敏感，无法构造反斜杠名与大小写冲突。
    #[cfg(unix)]
    {
        // 反斜杠（Linux 文件名合法，pak 内非法）
        let d2 = common::temp_str("wdpak_wtest_bs");
        write_file(&format!("{d2}/back\\slash.txt"), b"x");
        let err2 = common::temp_str("wdpak_wtest_err2.pak");
        assert_same_as_python(&["pack", d2.as_str(), err2.as_str()]);

        // 同目录大小写冲突（两个文件名仅大小写不同）
        let d3 = common::temp_str("wdpak_wtest_case");
        write_file(&format!("{d3}/a.txt"), b"1");
        write_file(&format!("{d3}/A.txt"), b"2");
        let err3 = common::temp_str("wdpak_wtest_err3.pak");
        assert_same_as_python(&["pack", d3.as_str(), err3.as_str()]);

        // 目录与文件大小写冲突（preflight 按层先目录后文件，消息中后到者在前）
        let d4 = common::temp_str("wdpak_wtest_casemix");
        write_file(&format!("{d4}/Sub/inner.txt"), b"1");
        write_file(&format!("{d4}/sub"), b"2"); // 与目录 Sub 的 lower 键冲突
        let err4 = common::temp_str("wdpak_wtest_err4.pak");
        assert_same_as_python(&["pack", d4.as_str(), err4.as_str()]);
    }
}

// ---------------------------------------------------------------------------
// pack：manifest（order / mtime / 宽松读）
// ---------------------------------------------------------------------------

#[test]
fn pack_manifest_order_and_mtime_matches_python() {
    // manifest：order 逆序（验证 build_dir 排序）+ mtime 恢复（0 与上界）
    let indir = common::temp_str("wdpak_wtest_mf");
    write_file(&format!("{indir}/a.txt"), b"aaa");
    write_file(&format!("{indir}/b.txt"), b"bbb");
    write_file(&format!("{indir}/sub/c.txt"), b"ccc");
    let mf = serde_json::json!({
        "pak": "x",
        "order": ["sub/c.txt", "b.txt", "a.txt"],
        "files": {
            "a.txt": {"attrib": 0, "mtime": 0, "stored": 3, "orig": 3},
            "b.txt": {"attrib": 0, "mtime": 4294967295u64, "stored": 3, "orig": 3},
            "sub/c.txt": {"mtime": 1234567890}
        }
    });
    write_file(
        &format!("{indir}/_pak_manifest.json"),
        serde_json::to_string(&mf).unwrap().as_bytes(),
    );
    assert_pack_same(&indir, &[], "mf_c");
    assert_pack_same(&indir, &["--no-compress"], "mf_nc");

    // files 条目缺 mtime 键 → .get('mtime', 0) 取 0
    let indir2 = common::temp_str("wdpak_wtest_mf2");
    write_file(&format!("{indir2}/a.txt"), b"aaa");
    write_file(
        &format!("{indir2}/_pak_manifest.json"),
        br#"{"order": [], "files": {"a.txt": {"attrib": 0}}}"#,
    );
    assert_pack_same(&indir2, &[], "mf2");

    // 坏 JSON → _load_manifest 整份忽略（等价无 manifest）
    let indir3 = common::temp_str("wdpak_wtest_mf3");
    write_file(&format!("{indir3}/a.txt"), b"aaa");
    write_file(&format!("{indir3}/_pak_manifest.json"), b"{not json");
    assert_pack_same(&indir3, &[], "mf3");
}

#[test]
fn pack_manifest_mtime_invalid_matches_python() {
    // mtime 非法值：Python 校验 isinstance(int) 且 0 <= v <= 0xFFFFFFFF，
    // 错误消息 `manifest 中 {rel} 的 mtime 非法: {值!r}`（py repr 格式）
    for (tag, raw) in [
        ("f15", "1.5"),        // float → 1.5
        ("str", "\"x\""),      // str → 'x'
        ("nul", "null"),       // None
        ("neg", "-1"),         // 负数
        ("big", "4294967296"), // 超上界
    ] {
        let indir = common::temp_str(format!("wdpak_wtest_mti_{tag}"));
        write_file(&format!("{indir}/a.txt"), b"aaa");
        let mf = format!(r#"{{"order": [], "files": {{"a.txt": {{"mtime": {raw}}}}}}}"#);
        write_file(&format!("{indir}/_pak_manifest.json"), mf.as_bytes());
        assert_pack_same(&indir, &[], &format!("mti_{tag}"));
    }
}

#[test]
fn pack_manifest_dir_quirk() {
    // `_pak_manifest.json` 为目录（Python 怪癖）：
    // _load_manifest: open 目录抛 IsADirectoryError → 宽松返回 None（无 manifest）
    // _preflight: os.walk 无条件递归进入 → 校验其内容的文件名
    // _collect_files/_build_dir: 跳过 → 内容不进 pak
    let indir = common::temp_str("wdpak_wtest_mdir");
    write_file(&format!("{indir}/a.txt"), b"aaa");
    write_file(&format!("{indir}/_pak_manifest.json/keep.bin"), b"kkk");
    assert_pack_same(&indir, &[], "mdir"); // 成功，keep.bin 不进 pak（sha256 相等隐含）

    // 目录内容含非法名（中文）→ preflight 递归校验报错
    let indir2 = common::temp_str("wdpak_wtest_mdir2");
    write_file(&format!("{indir2}/a.txt"), b"aaa");
    write_file(
        &format!("{indir2}/_pak_manifest.json/\u{4e2d}\u{6587}.txt"),
        b"x",
    );
    let err5 = common::temp_str("wdpak_wtest_err5.pak");
    assert_same_as_python(&["pack", indir2.as_str(), err5.as_str()]);
}

// ---------------------------------------------------------------------------
// pack：进度与 jobs 无关性
// ---------------------------------------------------------------------------

#[test]
fn pack_progress_matches_python() {
    // 501 文件：进度行 done%500==0 或 done==total → 500/501、501/501 两行；
    // jobs=1（单线程路径）与 jobs=4（进程池 ex.map 有序消费）行序一致
    let indir = common::temp_str("wdpak_wtest_501");
    for i in 1..=501u32 {
        write_file(&format!("{indir}/f{i:04}.bin"), &lcg(i as u64, 64));
    }
    assert_pack_same(&indir, &["--jobs", "1"], "p501j1");
    assert_pack_same(&indir, &["--jobs", "4"], "p501j4");

    // --no-compress 无进度行
    assert_pack_same(&indir, &["--no-compress"], "p501nc");

    // Rust 侧 jobs 无关性：jobs=1 与 jobs=4 产物 sha256 相等
    let a = pair("p501r1");
    let b = pair("p501r4");
    run_rust(&["pack", indir.as_str(), a.rs.as_str(), "--jobs", "1"]);
    run_rust(&["pack", indir.as_str(), b.rs.as_str(), "--jobs", "4"]);
    assert_eq!(
        file_sha256(Path::new(&a.rs)),
        file_sha256(Path::new(&b.rs)),
        "jobs=1 与 jobs=4 产物应逐字节一致"
    );
}

// ---------------------------------------------------------------------------
// pack：写出失败
// ---------------------------------------------------------------------------

#[test]
fn pack_rename_failure_matches_python() {
    // 目标路径是已存在目录：写 .tmp 成功但 os.replace 到目录 → EISDIR，
    // 消息 `写出 pak 失败: [Errno 21] Is a directory: '<out>.tmp' -> '<out>'`，
    // 失败后 .tmp 清理
    let indir = common::temp_str("wdpak_wtest_rf");
    write_file(&format!("{indir}/a.txt"), b"aaa");
    let p = pair("rndir");
    fs::create_dir_all(&p.py).unwrap();
    fs::create_dir_all(&p.rs).unwrap();

    let py = run_py(&["pack", indir.as_str(), p.py.as_str()]);
    let rs = run_rust(&["pack", indir.as_str(), p.rs.as_str()]);
    #[cfg(not(windows))]
    {
        assert_run_match(&py, &rs);
        assert_eq!(py.0, Some(2), "应失败退出 2");
    }
    #[cfg(windows)]
    {
        // Windows 上 Rust 按 P0-7 输出固定文案（不含 .tmp 路径），Python 仍是
        // WinError 5 形态；这是设计允许的差异，只断言语义与退出码。
        assert_eq!(
            rs.0, py.0,
            "退出码不一致: rust={:?} python={:?}",
            rs.0, py.0
        );
        assert_eq!(py.0, Some(2), "应失败退出 2");
        let rs_err = norm(&rs.2, RS_MARK);
        let py_err = norm(&py.2, PY_MARK);
        assert!(rs_err.contains("写出 pak 失败"), "rust stderr: {rs_err}");
        assert!(py_err.contains("写出 pak 失败"), "python stderr: {py_err}");
        assert!(
            rs_err.contains("目标文件被其他程序占用或不可写"),
            "rust stderr: {rs_err}"
        );
        assert!(
            rs_err.contains("<OUT>rndir"),
            "rust stderr 应包含输出路径: {rs_err}"
        );
    }
    // .tmp 已清理（两实现一致）
    assert!(
        !Path::new(&format!("{}.tmp", p.py)).exists(),
        "py .tmp 残留"
    );
    assert!(
        !Path::new(&format!("{}.tmp", p.rs)).exists(),
        "rs .tmp 残留"
    );
}

// ---------------------------------------------------------------------------
// golden：layout（etc_mini 双模式）与 replace 三用例
// ---------------------------------------------------------------------------

#[test]
fn golden_layout_pack() {
    // etc_mini 含 _pak_manifest.json（order 排序 + mtime 恢复）、café.list（latin1）、
    // "zuanshi -test.list"（空格名）、空文件 task_score.list —— golden pak 由 Python
    // 产物生成，sha256 相等即逐字节一致
    let indir = golden("layout/etc_mini");
    let out_nc = common::temp_str("wdpak_wtest_gold_nc.pak");
    let (code, _, err) = run_rust(&["pack", indir.as_str(), out_nc.as_str(), "--no-compress"]);
    assert_eq!(
        code,
        Some(0),
        "pack --no-compress 失败: {}",
        String::from_utf8_lossy(&err)
    );
    assert_eq!(
        file_sha256(Path::new(&out_nc)),
        file_sha256(Path::new(&golden("layout/etc_mini_nocompress.pak"))),
        "no-compress 产物与 golden 不符"
    );

    let out_c = common::temp_str("wdpak_wtest_gold_c.pak");
    let (code, _, err) = run_rust(&["pack", indir.as_str(), out_c.as_str()]);
    assert_eq!(code, Some(0), "pack 失败: {}", norm(&err, "x"));
    assert_eq!(
        file_sha256(Path::new(&out_c)),
        file_sha256(Path::new(&golden("layout/etc_mini_compress.pak"))),
        "compress 产物与 golden 不符"
    );
}

#[test]
fn golden_replace() {
    // golden replace.json：etc.pak 100heis.list（压缩/非压缩）+ lib_gs32.pak
    // clone/misc/mixed_agent.o（压缩）；期望值为 Python 产物 sha256
    if skip_without_etc() || skip_without_lib() {
        return;
    }
    let j: Value =
        serde_json::from_str(&fs::read_to_string(golden("replace/replace.json")).unwrap()).unwrap();
    let newfile = golden("replace/newfile.bin");
    for (i, case) in j["cases"].as_array().unwrap().iter().enumerate() {
        let pak = root(&format!("release/{}", case["pak"].as_str().unwrap()));
        let inner = case["inner"].as_str().unwrap();
        let before = case["pak_sha256_before"].as_str().unwrap();
        let after = case["pak_sha256_after"].as_str().unwrap();
        assert_eq!(
            file_sha256(Path::new(&pak)),
            before,
            "golden 源 pak 校验失败（release 文件变更?）: {pak}"
        );
        let out = common::temp_str(format!("wdpak_wtest_goldrep_{i}.pak"));
        fs::copy(&pak, &out).unwrap();
        let extra: Vec<&str> = if case["mode"].as_str().unwrap() == "no-compress" {
            vec!["--no-compress"]
        } else {
            vec![]
        };
        let mut args = vec!["replace", out.as_str(), inner, newfile.as_str()];
        args.extend(extra.iter().copied());
        let (code, _, err) = run_rust(&args);
        assert_eq!(
            code,
            Some(0),
            "replace 失败: {}",
            String::from_utf8_lossy(&err)
        );
        assert_eq!(
            file_sha256(Path::new(&out)),
            after,
            "replace 产物与 golden 不符: {pak} / {inner} / {}",
            case["mode"].as_str().unwrap()
        );
    }
}

// ---------------------------------------------------------------------------
// replace：对拍 + backup mtime 语义 + 错误路径
// ---------------------------------------------------------------------------

#[test]
fn replace_basic_matches_python() {
    if skip_without_etc() {
        return;
    }
    // etc.pak 内压缩条目 100heis.list，替换内容为 4500 字节伪随机数据
    let nf = newfile();
    let etc = etc_pak();
    assert_replace_same(&etc, "100heis.list", &nf, &[], "rep_c");
    assert_replace_same(&etc, "100heis.list", &nf, &["--no-compress"], "rep_nc");
}

#[test]
fn replace_backup_matches_python() {
    if skip_without_etc() {
        return;
    }
    // --backup：stderr 顺序 `# 已备份` 先于 `# 已替换`；
    // .bak = shutil.copy2(pak, bak) —— 字节 + mtime 均继承替换前的 pak
    let nf = newfile();
    let etc = etc_pak();
    assert_replace_same(&etc, "100heis.list", &nf, &["--backup"], "rep_bk");
}

#[test]
fn replace_backup_mtime_semantics() {
    if skip_without_etc() {
        return;
    }
    // mtime 继承专门验证：副本设固定旧时间 T0 → replace --backup →
    // .bak 的 mtime == T0（对齐 shutil.copy2 的 copystat；两实现各自验证）
    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let nf = newfile();
    let p = pair("bmt");
    for side in [&p.py, &p.rs] {
        fs::copy(etc_pak(), side).unwrap();
        set_mtime(side, t0);
    }
    for side in [&p.py, &p.rs] {
        let args = vec![
            "replace",
            side.as_str(),
            "100heis.list",
            nf.as_str(),
            "--backup",
        ];
        let (code, _, err) = run_impl(side == &p.py, &args);
        assert_eq!(
            code,
            Some(0),
            "replace --backup 失败: {}",
            String::from_utf8_lossy(&err)
        );
        let bak = format!("{side}.bak");
        assert_eq!(
            fs::metadata(&bak).unwrap().modified().unwrap(),
            t0,
            ".bak mtime 未继承替换前的 pak mtime: {bak}"
        );
    }
}

/// 按标记选择实现执行（replace_backup_mtime_semantics 用）
fn run_impl(py: bool, args: &[&str]) -> RunResult {
    if py {
        run_py(args)
    } else {
        run_rust(args)
    }
}

#[test]
fn replace_errors_match_python() {
    if skip_without_etc() {
        return;
    }
    // 包内不存在 / 是目录 / newfile 缺失 —— 错误均在写出前，不落盘，同参数对拍
    let nf = newfile();
    let etc = etc_pak();
    assert_same_as_python(&["replace", etc.as_str(), "no_such_entry.list", nf.as_str()]);
    if !skip_without_lib() {
        let lib = lib_pak();
        assert_same_as_python(&["replace", lib.as_str(), "clone", nf.as_str()]);
        // 目录
    }
    let missing = common::temp_str("wdpak_wtest_missing_newfile.bin");
    assert_same_as_python(&["replace", etc.as_str(), "100heis.list", missing.as_str()]);
    // OSError 兜底: 错误: [Errno 2] No such file or directory: '...'
}

// ---------------------------------------------------------------------------
// 长测：etc.pak extract → pack roundtrip（#[ignore]）
// ---------------------------------------------------------------------------

#[test]
#[ignore] // 长测：2981 文件全量解包 + 双实现打包对拍，cargo test -- --ignored
fn roundtrip_etc_pak() {
    if skip_without_etc() {
        return;
    }
    let rt = common::temp_str("wdpak_wtest_rt");
    let etc = etc_pak();

    // 1) Rust extract 到干净目录（产物含 _pak_manifest.json：order + mtime）
    let outdir = format!("{rt}/etc");
    let (code, _, err) = run_rust(&["extract", etc.as_str(), outdir.as_str()]);
    assert_eq!(
        code,
        Some(0),
        "extract 失败: {}",
        String::from_utf8_lossy(&err)
    );

    // 2) 双实现 pack 对拍（默认 jobs；stderr 归一化比对 + 产物 sha256 比对）
    let indir = format!("{rt}/etc");
    let p = pair("rt");
    let py = run_py(&["pack", indir.as_str(), p.py.as_str()]);
    let rs = run_rust(&["pack", indir.as_str(), p.rs.as_str()]);
    assert_run_match(&py, &rs);
    assert_eq!(py.0, Some(0));
    assert_eq!(
        file_sha256(Path::new(&p.rs)),
        file_sha256(Path::new(&p.py)),
        "roundtrip pak sha256 不一致"
    );

    // 3) Rust jobs 无关性 + 产物可校验
    let j1 = format!("{rt}/jobs1.pak");
    let (code, _, err) = run_rust(&["pack", indir.as_str(), j1.as_str(), "--jobs", "1"]);
    assert_eq!(
        code,
        Some(0),
        "jobs=1 pack 失败: {}",
        String::from_utf8_lossy(&err)
    );
    assert_eq!(
        file_sha256(Path::new(&j1)),
        file_sha256(Path::new(&p.rs)),
        "roundtrip jobs=1 与默认 jobs 产物应一致"
    );
    let (code, out, _) = run_rust(&["check", p.rs.as_str()]);
    assert_eq!(
        code,
        Some(0),
        "roundtrip pak check 失败:\n{}",
        String::from_utf8_lossy(&out)
    );
}
