//! LZSS（Okumura 变体）压缩/解压 —— 与 Python 版 wd_pak_tool.py 逐字节一致。
//!
//! 关键语义（对照 wd_pak_tool.py 第 67-173 行，属格式红线，不得偏离）：
//! - 环形窗口 N=4096；解压初始窗口 = 4078×0x20(空格) + 18×0x00（尾部是零不是空格），
//!   写指针初值 4078
//! - flag 字节每 8 项一组，位=1 为字面量（1B），位=0 为匹配（2B）
//! - 匹配编码：低字节 = off & 0xFF，高字节 = ((off>>8)&0x0F)<<4 | (len-3)；
//!   off 为窗口绝对位置 0..4095，len ∈ 3..=18
//! - 解压在每个编码项开始前检查产出量是否达到 org，匹配项内部不中断
//!   （损坏流可能使输出超过 org，长度校验由调用方按 != 报错）
//! - 压缩器：24 位 key（连续 3 字节）直索引哈希链，候选上限 128，
//!   匹配接受条件 1 ≤ d ≤ 4078 且 d ≤ i；仅当剩余 ≥ 3 字节时才插链（末尾 2 字节不插）
//!
//! 性能说明：压缩的热点为哈希链候选遍历（高重复文本语料 4MB 输入约 1.2 亿次
//! 候选检查，56% 的编码项打满 128 候选上限）。Python 语义要求逐步读取 prev
//! 单链的当前状态（环形窗口复用会覆写 prev 槽，任何预计算跳链都会因覆写
//! 过期而导致遍历序漂移），因此每候选一次串行 L1 load（4-5 周期）是物理下限，
//! 单线程吞吐极限约 20 MB/s 量级，无法通过向量化/缓存等常规手段突破。

use std::io::{self, Write};

/// 环形窗口大小
pub const N: usize = 4096;
/// 最大匹配长度（Okumura F 参数）
pub const F: usize = 18;
/// 最小匹配长度
pub const MIN_MATCH: usize = 3;
/// 最大匹配长度
pub const MAX_MATCH: usize = 18;
/// N - F = 4078：初始写指针位置，同时是最大匹配距离
pub const N_F: usize = N - F;

/// 流式解压的块缓冲大小
const CHUNK: usize = 64 * 1024;

/// 比较两个位置起始的最长公共前缀，上限 max_len（3..=18）。
///
/// 调用方保证 a、b 各自后至少 max_len 字节可读（压缩循环中
/// j+max_len ≤ i+max_len ≤ n 恒成立）。8 字节块比较：
/// 首个差异字节索引 = (x^y) 尾零数 >> 3，from_le_bytes 显式定义
/// 字节→数值映射，该结论与平台字节序无关。
#[inline(always)]
fn cmp_len(a: &[u8], b: &[u8], max_len: usize) -> usize {
    let mut l = 0;
    while l + 8 <= max_len {
        let x = u64::from_le_bytes(a[l..l + 8].try_into().unwrap());
        let y = u64::from_le_bytes(b[l..l + 8].try_into().unwrap());
        if x != y {
            return l + ((x ^ y).trailing_zeros() >> 3) as usize;
        }
        l += 8;
    }
    while l < max_len && a[l] == b[l] {
        l += 1;
    }
    l
}

/// 解压 src 为期望 org 字节，流式写入 w，返回实际产出字节数。
///
/// 绝不按 org 预分配（org 可能被篡改为 0xFFFFFFFF）；压缩流提前耗尽时
/// 安全终止并返回已产出数，长度是否合规由调用方校验（对应 Python
/// read_file 的"解压大小不符"检查）。
pub fn decompress(src: &[u8], org: u64, w: &mut impl Write) -> io::Result<u64> {
    // 初始窗口：4078 空格 + 18 零（尾部 18 字节是零，不是空格）
    let mut buf = [0u8; N];
    buf[..N_F].fill(0x20);
    let mut r = N_F; // 窗口写指针
    let mut flags: u32 = 0; // flag 位流状态（高位哨兵 0xFF00）
    let mut pos = 0usize; // 输入读指针
    let n = src.len();
    let mut chunk: Vec<u8> = Vec::with_capacity(CHUNK);
    let mut total: u64 = 0;
    while total < org {
        flags >>= 1;
        if flags & 0x100 == 0 {
            if pos >= n {
                break;
            }
            flags = src[pos] as u32 | 0xFF00;
            pos += 1;
        }
        if flags & 1 != 0 {
            // 字面量：拷 1 字节
            if pos >= n {
                break;
            }
            let c = src[pos];
            pos += 1;
            chunk.push(c);
            buf[r] = c;
            r = (r + 1) & (N - 1);
            total += 1;
        } else {
            // 匹配：2 字节 → 窗口绝对位置 + 长度
            if pos + 1 >= n {
                break;
            }
            let b0 = src[pos] as usize;
            let b1 = src[pos + 1] as usize;
            pos += 2;
            let mut i = b0 | ((b1 & 0xF0) << 4); // 位置 0..4095
            let len = (b1 & 0x0F) + 3; // 3..=18（Python: (b&0x0F)+2 再 +1）
            for _ in 0..len {
                // 重叠匹配必须逐字节自复制：目标区会读到刚更新的窗口内容
                let c = buf[i & (N - 1)];
                chunk.push(c);
                buf[r] = c;
                r = (r + 1) & (N - 1);
                i += 1;
                total += 1;
            }
        }
        if chunk.len() >= CHUNK {
            w.write_all(&chunk)?;
            chunk.clear();
        }
    }
    w.write_all(&chunk)?;
    Ok(total)
}

/// 便捷版：解压到 Vec。仅给适度初始容量（防 org 篡改导致巨额预分配），
/// 超出部分由 Vec 倍增增长。
pub fn decompress_to_vec(src: &[u8], org: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(org.min(4 * 1024 * 1024) as usize);
    decompress(src, org, &mut out)?;
    Ok(out)
}

/// 压缩 data，输出与 Python lzss_compress 逐字节一致。空输入 → 空输出。
pub fn compress(data: &[u8]) -> Vec<u8> {
    let n = data.len();
    if n == 0 {
        return Vec::new();
    }
    // 24 位 key 直索引哈希链，u16 存 pos+1（0=空）：
    // 32MB 零初始化（alloc_zeroed 惰性零页）+ 同 cache 行 key 数翻倍。
    // 只有直索引才能保证候选遍历顺序 = 最新插入优先，与 Python dict 语义
    // 完全一致（改用小哈希表+链地址会改变匹配决策导致输出漂移）。
    let mut head = vec![0u16; 1 << 24];
    let mut prev = [0u16; N];
    let mut r = N_F; // 窗口写指针（恒等于 (4078 + i) & 4095）
    // 最坏情况全字面量：n 字节 + 每 8 项 1 个 flag 字节
    let mut out = Vec::with_capacity(n + n / 8 + 8);
    let mut flag_pos: Option<usize> = None; // 当前组 flag 字节在 out 中的位置
    let mut flags: u8 = 0;
    let mut count: u32 = 0; // 组内项序号 0..8
    let mut i = 0usize;
    while i < n {
        if flag_pos.is_none() {
            out.push(0); // flag 占位字节，凑满 8 项或结束时回填
            flag_pos = Some(out.len() - 1);
            flags = 0;
            count = 0;
        }
        let mut best_len = 0usize;
        let mut best_off = 0usize;
        let max_len = MAX_MATCH.min(n - i);
        // 链头缓存：搜索与后续 k=0/字面量插链用同一 key，读一次 head 复用
        //（匹配输出时 max_len>=3 蕴含 i+2<n，k=0 必插链；字面量插链条件
        //  i+2<n 同样蕴含 max_len>=3，即 head0 必已初始化）
        let mut head0: u16 = 0;
        let mut key: usize = 0;
        if max_len >= MIN_MATCH {
            key =
                (data[i] as u32 | (data[i + 1] as u32) << 8 | (data[i + 2] as u32) << 16) as usize;
            head0 = head[key];
            let mut cand = head0; // pos+1 编码，0=空
            let mut checked = 0u32;
            // d 合法域 [1, min(4078, i)]，合一为无符号比较：d-1 < limit
            //（d=0 时下溢为 usize::MAX 必被拒，与 Python d>=1 等价）
            let limit = N_F.min(i);
            while cand != 0 && checked < 128 {
                let pos = (cand - 1) as usize; // 候选的窗口绝对位置
                // d = 环形距离；d<=i 保证匹配起点 j=i-d 不越过输入头部
                //（因此 Python 压缩流永不引用初始窗口）
                let d = r.wrapping_sub(pos) & (N - 1);
                if d.wrapping_sub(1) < limit {
                    let j = i - d;
                    // 剪枝：best_len>0 时先看其后一字节是否相等再做完整比较
                    //（best_len < max_len ≤ n-i，两侧索引均 < n）
                    if best_len == 0 || data[j + best_len] == data[i + best_len] {
                        let l = cmp_len(&data[j..], &data[i..], max_len);
                        if l > best_len {
                            best_len = l;
                            best_off = pos;
                            if l >= max_len {
                                break;
                            }
                        }
                    }
                }
                cand = prev[pos];
                checked += 1;
            }
        }
        if best_len >= MIN_MATCH {
            out.push((best_off & 0xFF) as u8);
            out.push(((((best_off >> 8) & 0x0F) << 4) | (best_len - 3)) as u8);
            for k in 0..best_len {
                // 仅当还有完整 3 字节时才插链（Python 行为，末尾 2 字节不插）
                if i + k + 2 < n {
                    if k == 0 {
                        // k=0 的 key 与搜索 key 相同，复用链头
                        prev[r] = head0;
                        head[key] = (r + 1) as u16;
                    } else {
                        let kk = (data[i + k] as u32 | (data[i + k + 1] as u32) << 8
                            | (data[i + k + 2] as u32) << 16) as usize;
                        prev[r] = head[kk];
                        head[kk] = (r + 1) as u16;
                    }
                }
                r = (r + 1) & (N - 1);
            }
            i += best_len;
        } else {
            flags |= 1 << count;
            out.push(data[i]);
            if i + 2 < n {
                // key 与搜索 key 相同，复用链头
                prev[r] = head0;
                head[key] = (r + 1) as u16;
            }
            r = (r + 1) & (N - 1);
            i += 1;
        }
        count += 1;
        if count == 8 {
            if let Some(p) = flag_pos {
                out[p] = flags;
            }
            flag_pos = None;
        }
    }
    if let Some(p) = flag_pos {
        out[p] = flags;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定种子伪随机字节（xorshift32），无外部依赖，测试可复现
    fn pseudo_random(seed: u32, len: usize) -> Vec<u8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s >> 24) as u8
            })
            .collect()
    }

    /// 边界输入往返：compress 后 decompress_to_vec 必须等于原文
    #[test]
    fn roundtrip_boundaries() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("1byte", vec![0x41]),
            ("2byte", vec![0x41, 0x42]),
            ("3byte", vec![0x41, 0x42, 0x43]),
            ("repeat-17", vec![b'Z'; 17]),
            ("repeat-18", vec![b'Z'; 18]),
            ("repeat-19", vec![b'Z'; 19]),
            ("repeat-20", vec![b'Z'; 20]),
            ("same-4077", vec![b'Q'; 4077]),
            ("same-4078", vec![b'Q'; 4078]),
            ("same-4079", vec![b'Q'; 4079]),
            ("same-4096", vec![b'Q'; 4096]),
            ("spaces-512", vec![b' '; 512]),
            (
                "abc-x1000",
                b"abc".iter().copied().cycle().take(3000).collect(),
            ),
            ("random-4KB", pseudo_random(0x12345678, 4096)),
            ("random-64KB", pseudo_random(0x9E37_79B9, 65536)),
            ("random-256KB", pseudo_random(0xDEAD_BEEF, 262144)),
        ];
        for (name, data) in cases {
            let blob = compress(&data);
            let back = decompress_to_vec(&blob, data.len() as u64).unwrap();
            assert_eq!(back, data, "往返失败: {name}");
        }
    }

    /// 空输入：压缩输出为空（FR-56），空流解压返回 0
    #[test]
    fn compress_empty_is_empty() {
        assert!(compress(&[]).is_empty());
        assert!(decompress_to_vec(&[], 0).unwrap().is_empty());
    }

    /// 手工压缩向量：不满 8 项的 flag 回填 + 末尾 2 字节不插链路径
    #[test]
    fn compress_partial_flag_group_backfill() {
        assert_eq!(compress(b"ABC"), vec![0x07, b'A', b'B', b'C']);
    }

    /// 手工压缩向量：i=1 处匹配（off=4078, len=3），匹配内 k=0 插链、k=1/2 不插
    #[test]
    fn compress_match_encoding_manual() {
        assert_eq!(compress(b"AAAA"), vec![0x01, b'A', 0xEE, 0xF0]);
    }

    /// 初始窗口语义：匹配引用位置 0（空格区），输出 18 个 0x20
    #[test]
    fn decompress_initial_window_spaces() {
        // flag=0x00（第 0 项=匹配）；off=0, len=18 → 字节 (0x00, 0x0F)
        let stream = [0x00u8, 0x00, 0x0F];
        let out = decompress_to_vec(&stream, 18).unwrap();
        assert_eq!(out, vec![b' '; 18]);
    }

    /// 初始窗口语义：匹配引用位置 4078（r 初值，尾部零区），输出 3 个 0x00。
    /// 若实现误把尾部 18 字节填成空格，此测试立即失败。
    #[test]
    fn decompress_initial_window_zero_tail() {
        // off=4078=0xFEE → (0xEE, 0xF0)，len=3
        let stream = [0x00u8, 0xEE, 0xF0];
        let out = decompress_to_vec(&stream, 3).unwrap();
        assert_eq!(out, vec![0u8; 3]);
    }

    /// 重叠匹配：字面量 'a' 后接 off=4078/len=18 的匹配，逐字节自复制出 19 个 'a'。
    /// 若实现误用切片拷贝，会读到未更新的初始零区导致失败。
    #[test]
    fn decompress_overlap_match() {
        // flag=0x01（bit0=字面量）；匹配 off=4078, len=18 → (0xEE, 0xFF)
        let stream = [0x01u8, 0x61, 0xEE, 0xFF];
        let out = decompress_to_vec(&stream, 19).unwrap();
        assert_eq!(out, vec![b'a'; 19]);
    }

    /// 流式解压正确性：慢速 sink（每次最多收 7 字节）与 to_vec 输出一致
    #[test]
    fn decompress_streaming_consistency() {
        struct TrickleSink(Vec<u8>);
        impl Write for TrickleSink {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                let take = buf.len().min(7);
                self.0.extend_from_slice(&buf[..take]);
                Ok(take)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let data = pseudo_random(42, 5000);
        let blob = compress(&data);
        let mut sink = TrickleSink(Vec::new());
        let got = decompress(&blob, data.len() as u64, &mut sink).unwrap();
        assert_eq!(got as usize, data.len());
        assert_eq!(sink.0, decompress_to_vec(&blob, data.len() as u64).unwrap());
    }

    /// 解压数据耗尽：截断流安全终止（返回数 < org、不 panic），
    /// 且已产出部分是完整解压输出的前缀（解压逐项确定性）
    #[test]
    fn decompress_truncated_stream_safe_stop() {
        let data = pseudo_random(7, 3000);
        let blob = compress(&data);
        let cut = &blob[..blob.len() / 2];
        let out = decompress_to_vec(cut, data.len() as u64).unwrap();
        assert!(out.len() < data.len(), "截断流不应产出完整结果");
        assert_eq!(&out[..], &data[..out.len()]);
        // 空流 + 非零 org：立即终止
        assert!(decompress_to_vec(&[], 100).unwrap().is_empty());
    }

    /// 解压达到 org 即停止：压缩流尾部多余字节被忽略
    #[test]
    fn decompress_stops_at_org_ignoring_trailing_bytes() {
        let data = pseudo_random(9, 100);
        let mut blob = compress(&data);
        blob.extend_from_slice(&[0xFF; 8]); // 追加垃圾
        let out = decompress_to_vec(&blob, data.len() as u64).unwrap();
        assert_eq!(out, data);
    }
}