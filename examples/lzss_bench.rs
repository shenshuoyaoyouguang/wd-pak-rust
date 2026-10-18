//! LZSS 吞吐基准：对多份 4MB 语料测单线程压缩/解压吞吐（release 模式运行）。
//!
//! cargo run --release --example lzss_bench
//!
//! 三份语料（各取前 4MB，口径统一）：
//! - 病态语料：../etc.pak/server_res_text.txt（高重复文本，全文件压缩率≈0.49，
//!   56% 编码项打满 128 候选上限——NFR-01 指定语料；缺失时退化为合成文本）
//! - 常规语料：../etc.pak/gbsn00lp.ttf（真实字体文件，压缩率≈0.81，包内常规对照）
//! - 随机语料：xorshift 伪随机字节（不可压缩，哈希链恒为 1，吞吐上界对照）
//!
//! 测量口径（对齐 design 2.9.5）：每语料预热 1 次（不计入）+ 3 轮计时取中位；
//! 解压每轮校验输出与原文一致，防止基准测到错误实现。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// 单份语料的测量结果（吞吐单位 MB/s，1MB=1048576 字节）
struct Bench {
    name: &'static str,
    comp: f64,
    decomp: f64,
    ratio: f64,
}

/// 合成文本语料（真实语料缺失时的退路）：重复行 + 伪随机词，保留局部冗余
fn synthetic_text(len: usize) -> Vec<u8> {
    let mut s: u32 = 0x1357_2468;
    let mut out = Vec::with_capacity(len);
    let mut line = 0u32;
    while out.len() < len {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let word = format!("wd_lzss_bench_{:04x}_", (s >> 8) & 0xFFFF);
        let l = format!("line={:06} word={} tail=恒定尾串重复冗余\n", line, word);
        out.extend_from_slice(l.as_bytes());
        line = line.wrapping_add(1);
    }
    out.truncate(len);
    out
}

/// xorshift 伪随机字节（不可压缩语料）
fn random_bytes(len: usize) -> Vec<u8> {
    let mut s: u32 = 0x9E37_79B9;
    let mut out = vec![0u8; len];
    for b in out.iter_mut() {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        *b = (s >> 24) as u8;
    }
    out
}

/// 读取语料文件前 4MB（不存在返回 None）
fn load_corpus(p: &Path) -> Option<Vec<u8>> {
    let d = fs::read(p).ok()?;
    Some(d[..d.len().min(4 << 20)].to_vec())
}

/// 对一份语料跑基准：预热 1 次 + 3 轮计时取中位
fn bench(name: &'static str, src: &str, data: &[u8]) -> Bench {
    println!("== 语料：{name}（{src}，{} 字节）", data.len());

    // 预热：让页缓存、分支预测进入稳态，不计入统计
    let warm = wd_pak::lzss::compress(data);
    let _ = wd_pak::lzss::decompress_to_vec(&warm, data.len() as u64).unwrap();

    let mut comp = [0.0f64; 3];
    let mut blob = Vec::new();
    for (i, v) in comp.iter_mut().enumerate() {
        let t = Instant::now();
        blob = wd_pak::lzss::compress(data);
        let secs = t.elapsed().as_secs_f64();
        *v = data.len() as f64 / secs / (1024.0 * 1024.0);
        println!("  压缩 第{}轮: {secs:.3}s  {v:.1} MB/s", i + 1);
    }
    comp.sort_by(|a, b| a.total_cmp(b));

    let mut decomp = [0.0f64; 3];
    for (i, v) in decomp.iter_mut().enumerate() {
        let mut sink = Vec::with_capacity(data.len());
        let t = Instant::now();
        let got = wd_pak::lzss::decompress(&blob, data.len() as u64, &mut sink).unwrap();
        let secs = t.elapsed().as_secs_f64();
        assert_eq!(got as usize, data.len());
        assert_eq!(sink, data, "基准解压结果与原文不一致");
        *v = data.len() as f64 / secs / (1024.0 * 1024.0);
        println!("  解压 第{}轮: {secs:.3}s  {v:.1} MB/s", i + 1);
    }
    decomp.sort_by(|a, b| a.total_cmp(b));

    let ratio = blob.len() as f64 / data.len() as f64;
    println!(
        "  中位：压缩 {:.1} MB/s，解压 {:.1} MB/s，压缩率 {ratio:.2}（输出 {} 字节）",
        comp[1],
        decomp[1],
        blob.len()
    );
    Bench { name, comp: comp[1], decomp: decomp[1], ratio }
}

fn corpus_path(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("etc.pak")
        .join(file)
}

fn main() {
    let mut corpus: Vec<u8> = match load_corpus(&corpus_path("server_res_text.txt")) {
        Some(d) => d,
        None => {
            eprintln!("警告：无法读取病态语料，改用合成文本");
            synthetic_text(4 << 20)
        }
    };
    let pathological = bench(
        "病态文本",
        "etc.pak/server_res_text.txt 前 4MB（缺失时合成文本）",
        &corpus,
    );
    corpus.shrink_to_fit();

    let normal = match load_corpus(&corpus_path("gbsn00lp.ttf")) {
        Some(d) => bench("常规字体", "etc.pak/gbsn00lp.ttf 前 4MB", &d),
        None => {
            eprintln!("警告：常规语料缺失，跳过");
            Bench { name: "常规字体", comp: 0.0, decomp: 0.0, ratio: 0.0 }
        }
    };
    let rnd = random_bytes(4 << 20);
    let random = bench("随机字节", "xorshift 伪随机 4MB（不可压缩）", &rnd);

    println!("\n== 汇总（中位吞吐，MB/s）==");
    println!("{:<8} {:>10} {:>10} {:>8}", "语料", "压缩", "解压", "压缩率");
    for b in [&pathological, &normal, &random] {
        println!("{:<8} {:>10.1} {:>10.1} {:>8.2}", b.name, b.comp, b.decomp, b.ratio);
    }

    // 判定：NFR-01 修订口径（病态语料单线程压缩 ≥10 MB/s）与 NFR-03（解压 ≥50 MB/s）
    let ok_c = pathological.comp >= 10.0;
    let ok_d = pathological.decomp >= 50.0;
    println!(
        "\nNFR-01 病态语料单线程压缩 ≥10 MB/s: {}（{:.1} MB/s，为 Python 基线 0.14-0.28 的 {:.0}-{:.0} 倍）",
        if ok_c { "达标" } else { "未达标" },
        pathological.comp,
        pathological.comp / 0.28,
        pathological.comp / 0.14,
    );
    println!(
        "NFR-03 单线程解压 ≥50 MB/s: {}（{:.1} MB/s，为 Python 基线 4.45 的 {:.0} 倍）",
        if ok_d { "达标" } else { "未达标" },
        pathological.decomp,
        pathological.decomp / 4.45,
    );
    if !ok_c {
        eprintln!(
            "注：压缩吞吐受 Python 语义强制的串行哈希链遍历限制 \
             （高重复语料 1.21 亿次候选 × L1 load 依赖），单线程物理极限约 17-23 MB/s"
        );
    }
    std::process::exit(if ok_c && ok_d { 0 } else { 1 });
}