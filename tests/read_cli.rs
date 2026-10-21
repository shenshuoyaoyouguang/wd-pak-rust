//! 读路径 CLI 集成测试 —— Rust 实现与 Python 基准（wd_pak_tool.py）逐字节比对。
//!
//! 比对策略：
//! - list/find/cat/check：同参数分别调用两个实现，stdout/stderr/退出码逐字节一致
//! - cat -o：stderr 含 abspath（同 cwd 同参数 → 一致），文件内容经 golden 清单验证
//! - extract：全量解包后逐文件 sha256 与 tests/golden/extract_files.sha256 比对；
//!   清单由 Python extract 产物生成且含 manifest 行，故 sha256 相等即证明
//!   Rust 生成的 _pak_manifest.json 与 Python 生成物逐字节一致
//! - check：完好 pak exit 0；截断 pak（保留 header+根目录表+少量数据）exit 1，
//!   --deep 同时覆盖成功解压与"解压失败"两类路径
//!
//! lib_gs32.pak（7762 文件 + 519 目录）为 #[ignore] 长测：cargo test -- --ignored。
//! 文本输出在 Windows 上归一化 CRLF 与 Win32/Errno 前缀；`cat` 的二进制 stdout
//! 仍逐字节严格。临时产物位于 `WDPAK_TEST_TMP` 或系统临时目录，重复运行覆盖。
//! 缺少 `release/*.pak` 时相关用例显式 skip；`WDPAK_REQUIRE_FIXTURES=1` 可强制失败。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

mod common;

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

fn golden_sha_list() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/extract_files.sha256")
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

/// 同参数调用两个实现，断言文本 stdout/stderr/退出码一致。
fn assert_same_as_python(args: &[&str]) {
    assert_same_with(args, false);
}

/// 二进制 stdout（cat 无 -o）：stdout 逐字节严格，stderr 仍按文本归一化。
fn assert_same_as_python_bytes(args: &[&str]) {
    assert_same_with(args, true);
}

fn assert_same_with(args: &[&str], binary_stdout: bool) {
    let py = run_py(args);
    let rs = run_rust(args);
    if binary_stdout {
        assert_eq!(
            rs.1,
            py.1,
            "stdout(二进制) 不一致 {args:?}: {}",
            first_diff(&rs.1, &py.1)
        );
    } else {
        let rs_out = common::normalize_text(&rs.1);
        let py_out = common::normalize_text(&py.1);
        assert_eq!(
            rs_out,
            py_out,
            "stdout 不一致 {args:?}: {}",
            first_diff(rs_out.as_bytes(), py_out.as_bytes())
        );
    }
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

fn skip_without_etc() -> bool {
    common::skip_without_fixture(Path::new(&root("release/etc.pak")), "release/etc.pak")
}

fn skip_without_lib() -> bool {
    common::skip_without_fixture(
        Path::new(&root("release/lib_gs32.pak")),
        "release/lib_gs32.pak",
    )
}

fn sha256_hex(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_sha256(path: &Path) -> String {
    let data = fs::read(path).unwrap_or_else(|e| panic!("读取 {path:?} 失败: {e}"));
    sha256_hex(&data)
}

/// 解析 golden 清单 `<sha256>␣␣<pakname>:<rel>` → (key, sha)
fn golden_entries() -> Vec<(String, String)> {
    let text = fs::read_to_string(golden_sha_list()).expect("读取 golden 清单失败");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (sha, key) = l.split_once("  ").expect("golden 行格式错误");
            (key.to_string(), sha.to_string())
        })
        .collect()
}

fn golden_sha(pakname: &str, rel: &str) -> String {
    let key = format!("{pakname}:{rel}");
    golden_entries()
        .into_iter()
        .find(|(k, _)| k == &key)
        .unwrap_or_else(|| panic!("golden 清单无 {key}"))
        .1
}

/// Rust extract 全量解包后逐文件 sha256 与 golden（Python 产物）比对。
/// manifest 行亦在清单内 → 其 sha256 相等即与 Python 生成物逐字节一致。
fn assert_extract_matches_golden(pak: &str, pakname: &str, outdir: &str) {
    let _ = fs::remove_dir_all(outdir);
    let (code, _, stderr) = run_rust(&["extract", pak, outdir]);
    assert_eq!(
        code,
        Some(0),
        "extract 退出码非 0: {}",
        String::from_utf8_lossy(&stderr)
    );
    let prefix = format!("{pakname}:");
    let mut checked = 0usize;
    for (key, want) in golden_entries() {
        let Some(rel) = key.strip_prefix(&prefix) else {
            continue;
        };
        let got = file_sha256(&Path::new(outdir).join(rel));
        assert_eq!(got, want, "sha256 不符: {outdir}/{rel}");
        checked += 1;
    }
    assert!(checked > 0, "golden 清单无 {pakname} 条目");
}

/// 截断 etc.pak：保留 header(16B) + 根目录表 + 50KB 数据 →
/// 前部文件可正常读取、其余越界，check 报告问题并 exit 1
fn make_truncated_pak() -> String {
    let dst = common::temp_str("wdpak_test_trunc.pak");
    let cut = {
        let mut f = fs::File::open(root("release/etc.pak")).unwrap();
        let mut hdr = [0u8; 16];
        f.read_exact(&mut hdr).unwrap();
        let root_size = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as u64;
        16 + root_size + 50_000
    };
    let src = fs::File::open(root("release/etc.pak")).unwrap();
    let mut w = fs::File::create(&dst).unwrap();
    std::io::copy(&mut src.take(cut), &mut w).unwrap();
    dst
}

// ---------------------------------------------------------------------------
// 基本输出与错误路径
// ---------------------------------------------------------------------------

#[test]
fn version_matches_python() {
    // argparse action='version'：纯版本号 `1.0` 到 stdout、exit 0（Rust 手动 flag 对齐）
    let py = run_py(&["--version"]);
    let rs = run_rust(&["--version"]);
    assert_eq!(rs.1, b"1.0\n");
    assert_eq!(rs.0, py.0);
    assert_eq!(common::normalize_text(&rs.1), common::normalize_text(&py.1));
    assert_eq!(common::normalize_text(&rs.2), common::normalize_text(&py.2));
}

#[test]
fn list_matches_python() {
    if skip_without_etc() {
        return;
    }
    let pak = root("release/etc.pak");
    assert_same_as_python(&["list", pak.as_str()]);
    assert_same_as_python(&["list", "-l", pak.as_str()]);
}

#[test]
fn find_matches_python() {
    if skip_without_etc() {
        return;
    }
    let pak = root("release/etc.pak");
    assert_same_as_python(&["find", pak.as_str(), "list"]); // 2910 命中
    assert_same_as_python(&["find", pak.as_str(), "zz_nomatch"]); // 0 命中
}

#[test]
fn missing_pak_error_matches_python() {
    let p = common::temp_str("wdpak_test_no_such.pak");
    assert_same_as_python(&["list", p.as_str()]);
    assert_same_as_python(&["cat", p.as_str(), "x"]);
    assert_same_as_python(&["check", p.as_str()]);
}

// ---------------------------------------------------------------------------
// cat
// ---------------------------------------------------------------------------

#[test]
fn cat_stdout_matches_python() {
    if skip_without_etc() {
        return;
    }
    let pak = root("release/etc.pak");
    // stdout 二进制透明（对齐 sys.stdout.buffer.write）：压缩大文件 + 小文件
    assert_same_as_python_bytes(&["cat", pak.as_str(), "wuwuroom.list"]);
    assert_same_as_python_bytes(&["cat", pak.as_str(), "100heis.list"]);
}

#[test]
fn cat_out_file_matches_python() {
    if skip_without_etc() {
        return;
    }
    let path = common::temp_str("wdpak_test_cat_out.bin");
    let pak = root("release/etc.pak");
    let args = ["cat", pak.as_str(), "wuwuroom.list", "-o", path.as_str()];
    let py = run_py(&args);
    let rs = run_rust(&args);
    // stderr: `# 已导出 N 字节 -> <abspath>`（同 cwd 同参数 → abspath 一致）
    let py_err = common::normalize_text(&py.2);
    let rs_err = common::normalize_text(&rs.2);
    assert_eq!(
        rs_err,
        py_err,
        "cat -o stderr 不一致: {}",
        first_diff(rs_err.as_bytes(), py_err.as_bytes())
    );
    assert_eq!(rs.0, py.0);
    // 此刻文件为 Rust 写入的版本，内容经 golden 验证
    assert_eq!(
        file_sha256(Path::new(&path)),
        golden_sha("etc.pak", "wuwuroom.list")
    );
}

#[test]
fn cat_errors_match_python() {
    if skip_without_etc() {
        return;
    }
    let etc = root("release/etc.pak");
    assert_same_as_python(&["cat", etc.as_str(), "no_such_entry.list"]); // 不存在 → exit 2
    if !skip_without_lib() {
        let lib = root("release/lib_gs32.pak");
        assert_same_as_python(&["cat", lib.as_str(), "clone"]); // 目录 → exit 2
    }
}

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

#[test]
fn check_ok_matches_python() {
    if skip_without_etc() {
        return;
    }
    // 完好 pak：exit 0，报告逐字节一致
    let pak = root("release/etc.pak");
    assert_same_as_python(&["check", pak.as_str()]);
}

#[test]
fn check_truncated_matches_python() {
    if skip_without_etc() {
        return;
    }
    let trunc = make_truncated_pak();
    // 截断 pak：exit 1（"越界"问题列表）；--deep 混合成功解压与"解压失败"路径
    assert_same_as_python(&["check", trunc.as_str()]);
    assert_same_as_python(&["check", "--deep", trunc.as_str()]);
}

// ---------------------------------------------------------------------------
// extract（golden sha256 = 与 Python 产物逐字节一致的证明）
// ---------------------------------------------------------------------------

#[test]
fn extract_etc_matches_golden() {
    if skip_without_etc() {
        return;
    }
    let pak = root("release/etc.pak");
    let outdir = common::temp_str("wdpak_test_etc");
    assert_extract_matches_golden(pak.as_str(), "etc.pak", &outdir);
}

#[test]
#[ignore] // 长测：7762 文件 + 519 目录，cargo test -- --ignored 单独执行
fn lib_gs32_full_read_path_matches() {
    if skip_without_lib() {
        return;
    }
    let lib = root("release/lib_gs32.pak");
    assert_same_as_python(&["list", lib.as_str()]);
    assert_same_as_python(&["list", "-l", lib.as_str()]);
    assert_same_as_python(&["find", lib.as_str(), "misc"]);
    assert_same_as_python(&["check", lib.as_str()]);
    let outdir = common::temp_str("wdpak_test_lib");
    assert_extract_matches_golden(lib.as_str(), "lib_gs32.pak", &outdir);
}
