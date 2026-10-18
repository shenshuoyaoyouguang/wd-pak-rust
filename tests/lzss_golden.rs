//! LZSS golden 向量逐字节校验 —— 向量由 scripts/gen_golden.py 用 Python 版生成。
//!
//! 覆盖三组向量：
//! - lzss_compress_cases.json：20 个合成边界输入，compress 输出逐字节比对
//! - lzss_compress_files.json：etc.pak 抽样 40 文件（≤256KB 落盘逐字节比对，
//!   大文件只记哈希则按哈希+长度比对，输入从 input_source 指向的解包目录读取）
//! - lzss_decompress_cases.json 的 small 部分：真实/合成 LZSS 流解压逐字节比对
//!   （RAW 条目不走 LZSS 解压，跳过；large 部分需 pak 读取器，属集成回归范围）
//!
//! 向量目录缺失时跳过（生成流程未跑时保持测试可独立通过）。

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden")
}

fn root_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let d = h.finalize();
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// 十六进制解码（向量内嵌 hex，仅小写偶数长度输入）
fn hex_decode(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "hex 长度必须为偶数");
    s.as_bytes()
        .chunks(2)
        .map(|p| {
            let hi = (p[0] as char).to_digit(16).expect("非法 hex");
            let lo = (p[1] as char).to_digit(16).expect("非法 hex");
            ((hi << 4) | lo) as u8
        })
        .collect()
}

fn read_golden(rel: &str) -> Option<Vec<u8>> {
    let p = golden_dir().join(rel);
    if !p.exists() {
        return None;
    }
    fs::read(&p).ok()
}

#[derive(Deserialize)]
struct CompressCases {
    cases: Vec<CompressCase>,
}

#[derive(Deserialize)]
struct CompressCase {
    id: u32,
    desc: String,
    input_len: u64,
    input_sha256: String,
    expect_output_len: u64,
    expect_output_sha256: String,
    #[serde(default)]
    input_hex: Option<String>,
    #[serde(default)]
    input_file: Option<String>,
    #[serde(default)]
    expect_output_hex: Option<String>,
    #[serde(default)]
    expect_output_file: Option<String>,
}

impl CompressCase {
    /// 取输入字节：hex 内嵌优先，其次落盘文件
    fn input(&self) -> Option<Vec<u8>> {
        if let Some(h) = &self.input_hex {
            return Some(hex_decode(h));
        }
        if let Some(f) = &self.input_file {
            return read_golden(f);
        }
        None
    }

    fn expect_output(&self) -> Option<Vec<u8>> {
        if let Some(h) = &self.expect_output_hex {
            return Some(hex_decode(h));
        }
        if let Some(f) = &self.expect_output_file {
            return read_golden(f);
        }
        None
    }
}

#[test]
fn golden_compress_cases_byte_exact() {
    let path = golden_dir().join("lzss_compress_cases.json");
    if !path.exists() {
        eprintln!("跳过：未找到 {}", path.display());
        return;
    }
    let cases: CompressCases = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(!cases.cases.is_empty());
    for c in &cases.cases {
        let input = c.input().unwrap_or_else(|| panic!("case {} 无输入", c.id));
        assert_eq!(input.len() as u64, c.input_len, "case {} 输入长度不符", c.id);
        assert_eq!(sha256_hex(&input), c.input_sha256, "case {} 输入哈希不符", c.id);
        let got = wd_pak::lzss::compress(&input);
        assert_eq!(
            got.len() as u64,
            c.expect_output_len,
            "case {} ({}) 输出长度不符",
            c.id,
            c.desc
        );
        assert_eq!(
            sha256_hex(&got),
            c.expect_output_sha256,
            "case {} ({}) 输出哈希不符",
            c.id,
            c.desc
        );
        if let Some(expect) = c.expect_output() {
            assert_eq!(got, expect, "case {} ({}) 输出非逐字节一致", c.id, c.desc);
        }
        // 顺带往返自校验
        let back = wd_pak::lzss::decompress_to_vec(&got, input.len() as u64).unwrap();
        assert_eq!(back, input, "case {} 往返失败", c.id);
    }
}

#[derive(Deserialize)]
struct CompressFiles {
    files: Vec<CompressFile>,
}

#[derive(Deserialize)]
struct CompressFile {
    id: u32,
    path: String,
    input_len: u64,
    input_sha256: String,
    expect_output_len: u64,
    expect_output_sha256: String,
    input_source: String,
    #[serde(default)]
    in_file: Option<String>,
    #[serde(default)]
    out_file: Option<String>,
}

#[test]
fn golden_compress_files_byte_exact() {
    let path = golden_dir().join("lzss_compress_files.json");
    if !path.exists() {
        eprintln!("跳过：未找到 {}", path.display());
        return;
    }
    let files: CompressFiles = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(!files.files.is_empty());
    let mut checked = 0;
    for f in &files.files {
        // 输入：落盘文件，或从 input_source（如 "etc.pak/<name>"）指向的解包目录读取
        let input = if let Some(rel) = &f.in_file {
            read_golden(rel)
        } else {
            let name = f.input_source.strip_prefix("etc.pak/");
            name.and_then(|n| fs::read(root_dir().join("etc.pak").join(n)).ok())
        };
        let Some(input) = input else {
            eprintln!("跳过 file {}（输入不可得）: {}", f.id, f.path);
            continue;
        };
        assert_eq!(input.len() as u64, f.input_len, "file {} 输入长度不符", f.id);
        assert_eq!(sha256_hex(&input), f.input_sha256, "file {} 输入哈希不符", f.id);
        let got = wd_pak::lzss::compress(&input);
        assert_eq!(
            got.len() as u64,
            f.expect_output_len,
            "file {} ({}) 输出长度不符",
            f.id,
            f.path
        );
        assert_eq!(
            sha256_hex(&got),
            f.expect_output_sha256,
            "file {} ({}) 输出哈希不符",
            f.id,
            f.path
        );
        if let Some(rel) = &f.out_file {
            let expect = read_golden(rel).expect("out 文件缺失");
            assert_eq!(got, expect, "file {} ({}) 输出非逐字节一致", f.id, f.path);
        }
        checked += 1;
    }
    assert!(checked > 0, "未校验任何文件向量");
    eprintln!("抽样压缩向量校验通过 {}/{}", checked, files.files.len());
}

#[derive(Deserialize)]
struct DecompressCases {
    small: Vec<DecompressSmall>,
}

#[derive(Deserialize)]
struct DecompressSmall {
    id: u32,
    pak: String,
    inner: String,
    org: u64,
    stored_file: String,
    out_file: String,
    stored_sha256: String,
    out_sha256: String,
}

#[test]
fn golden_decompress_small_byte_exact() {
    let path = golden_dir().join("lzss_decompress_cases.json");
    if !path.exists() {
        eprintln!("跳过：未找到 {}", path.display());
        return;
    }
    let cases: DecompressCases = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(!cases.small.is_empty());
    let mut checked = 0;
    for c in &cases.small {
        // RAW 条目（no-compress pak）不走 LZSS 解压，属 pak 读取器回归范围
        if c.pak == "etc_mini_nocompress.pak" {
            continue;
        }
        let stored = read_golden(&c.stored_file).expect("stored 文件缺失");
        let expect = read_golden(&c.out_file).expect("out 文件缺失");
        assert_eq!(sha256_hex(&stored), c.stored_sha256, "case {} stored 哈希不符", c.id);
        assert_eq!(sha256_hex(&expect), c.out_sha256, "case {} out 哈希不符", c.id);
        let got = wd_pak::lzss::decompress_to_vec(&stored, c.org).unwrap();
        assert_eq!(
            got, expect,
            "case {} ({}/{}) 解压非逐字节一致",
            c.id, c.pak, c.inner
        );
        checked += 1;
    }
    assert!(checked > 0, "未校验任何解压向量");
    eprintln!("解压向量校验通过 {}/{}", checked, cases.small.len());
}