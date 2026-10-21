//! PAK 读取器 —— 对齐 Python `Pak` 类（open/walk/find/read_file/read_stored）。
//!
//! 兼容性要点：
//!   * 条目名为 Latin-1 字节串，relpath 一律用 `format::latin1` 逐字节映射
//!     （U+0000..U+00FF），绝不做 UTF-8 有损解码（会替换非法字节导致 find 失配）。
//!   * 所有 offset+size 越界判断一律 u64 域计算（NFR-08，防 u32 回绕）。
//!   * 文件句柄包 Mutex：满足 `&self` 读取接口的同时保持 Sync（供后续并行 extract）。

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::PakError;
use crate::format::{latin1, CatalogEntry, Header, HDR_SIZE};
use crate::tree::{parse_tree, NodePayload, TreeNode};

/// 已解析的 PAK 读取器。
#[derive(Debug)]
pub struct PakReader {
    /// pak 文件句柄（Mutex 使 `&self` 读取可行且线程安全）
    file: Mutex<std::fs::File>,
    /// pak 文件总字节数
    pub file_size: u64,
    /// 头部声明的根目录表字节数（rootDirSize）
    pub root_size: u32,
    /// 根目录表解析结果
    pub root: Vec<TreeNode>,
    /// pak 文件路径
    pub path: PathBuf,
}

impl PakReader {
    /// 打开并解析（对应 Python `Pak.__init__`）：
    /// 16B 头校验（太小 → Truncated、魔数 → BadMagic）+ 根目录表解析。
    pub fn open(path: &Path) -> Result<Self, PakError> {
        let mut file = std::fs::File::open(path)?;
        let file_size = file.metadata()?.len();
        if file_size < HDR_SIZE as u64 {
            return Err(PakError::Truncated);
        }
        let mut hdr = [0u8; HDR_SIZE];
        file.read_exact(&mut hdr)?;
        let header = Header::from_bytes(&hdr)?; // BadMagic 在此抛出
        let mut seen = HashSet::new();
        let root = parse_tree(
            &mut file,
            HDR_SIZE as u64,
            header.root_dir_size as u64,
            0,
            &mut seen,
            file_size,
        )?;
        Ok(Self {
            file: Mutex::new(file),
            file_size,
            root_size: header.root_dir_size,
            root,
            path: path.to_path_buf(),
        })
    }

    /// 深度优先遍历（对应 Python `walk`）：返回 (relpath, 节点) 列表。
    /// relpath 以 '/' 连接，目录条目先于其子项（Python rel + '/' 前缀拼接）。
    pub fn walk(&self) -> Vec<(String, &TreeNode)> {
        let mut out = Vec::new();
        self.walk_with_progress(|rel, node| out.push((rel.to_string(), node)));
        out
    }

    /// 与 [`Self::walk`] 相同的深度优先顺序，但逐条目回调，不先物化整个列表。
    pub fn walk_with_progress<'a, F: FnMut(&str, &'a TreeNode)>(&'a self, mut f: F) {
        fn rec<'a, F: FnMut(&str, &'a TreeNode)>(nodes: &'a [TreeNode], prefix: &str, f: &mut F) {
            for node in nodes {
                let rel = format!("{prefix}{}", latin1(&node.entry.name));
                f(&rel, node);
                if let NodePayload::Dir(children) = &node.payload {
                    rec(children, &format!("{rel}/"), f);
                }
            }
        }
        rec(&self.root, "", &mut f);
    }

    /// 按包内路径查找（对应 Python `find`）：去除首尾 `/`、`\`→`/`、逐段匹配；
    /// 中间段必须是目录；名字比较用 Latin-1 字符串（与 Python str 比较等价）。
    pub fn find(&self, inner: &str) -> Option<&TreeNode> {
        let inner = inner.trim_matches('/').replace('\\', "/");
        let parts: Vec<&str> = inner.split('/').collect();
        let mut nodes: &[TreeNode] = &self.root;
        let mut node: Option<&TreeNode> = None;
        for (k, p) in parts.iter().enumerate() {
            let n = nodes.iter().find(|n| latin1(&n.entry.name) == *p)?;
            if k + 1 < parts.len() {
                match &n.payload {
                    NodePayload::Dir(children) => nodes = children,
                    _ => return None, // 中间段不是目录，路径必然不匹配
                }
            }
            node = Some(n);
        }
        node
    }

    /// 读取并解压文件内容（对应 Python `read_file`）：
    /// 目录 → IsDir；RAW → 取前 org 字节（size < org → DataShort）；
    /// LZSS → 解压后长度 != org → SizeMismatch（压缩流提前耗尽同样落入此分支）。
    pub fn read_file(&self, node: &TreeNode) -> Result<Vec<u8>, PakError> {
        let name = latin1(&node.entry.name);
        if node.is_dir() {
            return Err(PakError::IsDir(name));
        }
        let blob = self.read_blob(&node.entry)?;
        if node.entry.is_raw() {
            let (stored, org) = (node.entry.size as u64, node.entry.org as u64);
            if stored < org {
                return Err(PakError::DataShort { name, stored, org });
            }
            return Ok(blob[..node.entry.org as usize].to_vec());
        }
        let out = crate::lzss::decompress_to_vec(&blob, node.entry.org as u64)?;
        if out.len() as u64 != node.entry.org as u64 {
            return Err(PakError::SizeMismatch {
                name,
                got: out.len() as u64,
                declared: node.entry.org as u64,
            });
        }
        Ok(out)
    }

    /// 按包内路径读取（导出/cat 语义）：缺失为 `不存在: …`，目录为“是目录”。
    /// 这是 GUI 与 CLI 共用的查找 + 读取序列，避免两处各写一遍。
    pub(crate) fn read_entry_by_path(&self, inner: &str) -> Result<Vec<u8>, PakError> {
        let node = self
            .find(inner)
            .ok_or_else(|| PakError::Missing(inner.to_string()))?;
        if node.is_dir() {
            return Err(PakError::IsDir(inner.to_string()));
        }
        self.read_file(node)
    }

    /// 原始存储字节（不解压，对应 Python `read_stored`；含 u64 越界校验）。
    pub fn read_stored(&self, node: &TreeNode) -> Result<Vec<u8>, PakError> {
        self.read_blob(&node.entry)
    }

    /// 内部：u64 域校验 offset+size ≤ 文件总长后 seek + 精确读取条目数据区。
    ///
    /// size=0 时不做越界预检（对齐 Python `f.read(0)` 语义：空读永不短读，
    /// offset 越过文件尾也返回空串）。etc.pak 实际存在 size=org=0 的幽灵条目
    /// （task_score.list），截断 pak 的 check --deep 依赖此行为不报解压失败。
    fn read_blob(&self, entry: &CatalogEntry) -> Result<Vec<u8>, PakError> {
        let (offset, size) = (entry.offset as u64, entry.size as u64);
        if size > 0 && offset.saturating_add(size) > self.file_size {
            return Err(PakError::DataOob(latin1(&entry.name)));
        }
        let mut buf = vec![0u8; size as usize];
        let mut f = self.file.lock().unwrap(); // 持锁期间无 panic，不会中毒
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(&mut buf)?;
        Ok(buf)
    }
}

// ---------------------------------------------------------------------------
// 单元测试：真实小 pak 构造（深度优先布局，对齐 Python _serialize）+ 防御路径
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{CatalogEntry, Header, CAT_SIZE, DIR_BIT, MAGIC, RAW_BIT};

    // ---- 测试 pak 构造辅助 -------------------------------------------------

    /// 测试节点：目录或文件（attrib/org/存储字节可完全自定）
    #[derive(Clone)]
    enum TestNode {
        Dir {
            name: &'static [u8],
            children: Vec<TestNode>,
        },
        File {
            name: &'static [u8],
            attrib: u32,
            org: u32,
            data: Vec<u8>,
        },
    }

    fn dir(name: &'static [u8], children: Vec<TestNode>) -> TestNode {
        TestNode::Dir { name, children }
    }

    /// RAW 文件（org = 数据长度）
    fn raw_file(name: &'static [u8], data: &[u8]) -> TestNode {
        TestNode::File {
            name,
            attrib: RAW_BIT,
            org: data.len() as u32,
            data: data.to_vec(),
        }
    }

    /// LZSS 文件（attrib=0，存储字节与 org 独立指定，可构造任意压缩流）
    fn lzss_file(name: &'static [u8], data: Vec<u8>, org: u32) -> TestNode {
        TestNode::File {
            name,
            attrib: 0,
            org,
            data,
        }
    }

    impl TestNode {
        /// 节点序列化后的总字节数（目录 = 子表 + 全部子孙数据，对齐 _node_size）
        fn node_size(&self) -> usize {
            match self {
                TestNode::Dir { children, .. } => {
                    CAT_SIZE * children.len() + children.iter().map(|c| c.node_size()).sum::<usize>()
                }
                TestNode::File { data, .. } => data.len(),
            }
        }

        fn fields(&self) -> (&'static [u8], u32, u32, u32) {
            match self {
                TestNode::Dir { name, children } => {
                    let sz = (CAT_SIZE * children.len()) as u32;
                    (*name, DIR_BIT, sz, sz)
                }
                TestNode::File { name, attrib, org, data } => {
                    (*name, *attrib, data.len() as u32, *org)
                }
            }
        }
    }

    /// 深度优先序列化（布局对齐 Python _serialize）：目录表在数据前
    fn serialize(out: &mut Vec<u8>, base: usize, nodes: &[TestNode]) {
        // 第一遍：深度优先分配各条目数据区 offset
        let mut running = base + CAT_SIZE * nodes.len();
        let mut offsets = Vec::with_capacity(nodes.len());
        for n in nodes {
            offsets.push(running);
            running += n.node_size();
        }
        // 第二遍：写目录表
        for (n, &off) in nodes.iter().zip(&offsets) {
            let (name, attrib, size, org) = n.fields();
            let e = CatalogEntry {
                attrib,
                offset: off as u32,
                size,
                org,
                mtime: 0,
                name: name.to_vec(),
            };
            out.extend_from_slice(&e.to_bytes().unwrap());
        }
        // 第三遍：按条目顺序写数据（目录递归写子表，恰好落在其 offset 处）
        for (n, &off) in nodes.iter().zip(&offsets) {
            match n {
                TestNode::Dir { children, .. } => serialize(out, off, children),
                TestNode::File { data, .. } => out.extend_from_slice(data),
            }
        }
    }

    /// 构造完整 pak 字节（Header + 根表 + 深度优先数据）
    fn build_pak(root: Vec<TestNode>) -> Vec<u8> {
        let mut out = Vec::new();
        let hdr = Header {
            magic: MAGIC,
            root_dir_size: (CAT_SIZE * root.len()) as u32,
            unused1: 0,
            unused2: 0,
        };
        out.extend_from_slice(&hdr.to_bytes());
        serialize(&mut out, HDR_SIZE, &root);
        out
    }

    /// 手工拼 64 字节目录项（绕过 CatalogEntry 名字校验，用于非法名/恶意字段测试）
    fn raw_entry(attrib: u32, offset: u32, size: u32, org: u32, name: &[u8]) -> Vec<u8> {
        let mut b = Vec::with_capacity(CAT_SIZE);
        b.extend_from_slice(&attrib.to_le_bytes());
        b.extend_from_slice(&offset.to_le_bytes());
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&org.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // mtime
        b.extend_from_slice(name);
        b.resize(CAT_SIZE, 0);
        b
    }

    fn header_bytes(root_size: u32) -> Vec<u8> {
        Header {
            magic: MAGIC,
            root_dir_size: root_size,
            unused1: 0,
            unused2: 0,
        }
        .to_bytes()
        .to_vec()
    }

    /// 写临时文件后打开（打开成功即删除，Linux 下 fd 仍有效）
    fn open_bytes(bytes: &[u8], tag: &str) -> Result<PakReader, PakError> {
        let path = std::env::temp_dir().join(format!("wd_pak_test_{tag}.pak"));
        std::fs::write(&path, bytes).unwrap();
        let r = PakReader::open(&path);
        let _ = std::fs::remove_file(&path);
        r
    }

    fn walk_rel(pak: &PakReader) -> Vec<String> {
        pak.walk().into_iter().map(|(r, _)| r).collect()
    }

    // ---- 1. 扁平结构：open / walk / find / read_file / read_stored ----------

    #[test]
    fn flat_open_walk_find_read() {
        let pak = open_bytes(
            &build_pak(vec![
                raw_file(b"a.txt", b"hello"),
                // LZSS 手工压缩流：flag=0xFF 后跟 8 个字面量 -> 解压 "ABCDEFGH"
                lzss_file(b"b.bin", vec![0xFF, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H'], 8),
                // RAW 且 org < size：有效数据取前 3 字节
                TestNode::File {
                    name: b"c.dat",
                    attrib: RAW_BIT,
                    org: 3,
                    data: b"abcdef".to_vec(),
                },
            ]),
            "flat",
        )
        .unwrap();

        assert_eq!(pak.root_size, 3 * CAT_SIZE as u32);
        assert_eq!(pak.file_size as usize, 16 + 3 * CAT_SIZE + 5 + 9 + 6);
        // 扁平结构：3 个条目，无目录
        assert_eq!(walk_rel(&pak), ["a.txt", "b.bin", "c.dat"]);

        let a = pak.find("a.txt").unwrap();
        assert!(!a.is_dir());
        assert_eq!(pak.read_file(a).unwrap(), b"hello");
        let b = pak.find("b.bin").unwrap();
        assert_eq!(pak.read_file(b).unwrap(), b"ABCDEFGH");
        let c = pak.find("c.dat").unwrap();
        assert_eq!(pak.read_file(c).unwrap(), b"abc"); // 只取前 org 字节
        assert_eq!(pak.read_stored(c).unwrap(), b"abcdef"); // 原始字节不截断

        // 路径归一化：首尾 '/'；反斜杠仅作段内分隔（对齐 Python 顺序：
        // strip('/') 先于 replace('\\','/')，前导反斜杠替换出的空首段必然失配）
        assert!(pak.find("/a.txt").is_some());
        assert!(pak.find("//a.txt//").is_some());
        assert!(pak.find("a.txt/x").is_none()); // 文件后不能有下一段
        assert!(pak.find("\\a.txt").is_none()); // 与 Python 一致：'\\a.txt' -> '/a.txt' -> 首段空
    }

    // ---- 2. 目录树结构：walk 深度优先顺序 + find 逐段匹配 --------------------

    #[test]
    fn nested_tree_walk_find() {
        let pak = open_bytes(
            &build_pak(vec![
                dir(
                    b"dir1",
                    vec![
                        raw_file(b"f1.txt", b"one"),
                        dir(b"sub", vec![raw_file(b"f2.txt", b"two")]),
                    ],
                ),
                raw_file(b"f0.txt", b"zero"),
            ]),
            "nested",
        )
        .unwrap();

        // 深度优先、目录先于其子项
        assert_eq!(
            walk_rel(&pak),
            ["dir1", "dir1/f1.txt", "dir1/sub", "dir1/sub/f2.txt", "f0.txt"]
        );

        let f2 = pak.find("dir1/sub/f2.txt").unwrap();
        assert!(!f2.is_dir());
        assert_eq!(pak.read_file(f2).unwrap(), b"two");
        assert!(pak.find("dir1/sub").unwrap().is_dir());
        assert!(pak.find("dir1\\sub\\f2.txt").is_some()); // 反斜杠分隔
        assert!(pak.find("/dir1/sub/").is_some()); // 首尾斜杠

        // 逐段匹配语义
        assert!(pak.find("dir1/f2.txt").is_none()); // f2 不在 dir1 直接层
        assert!(pak.find("f0.txt/x").is_none()); // 中间段不是目录
        assert!(pak.find("dir1/nope").is_none());

        // 对目录读内容 -> IsDir
        let d = pak.find("dir1").unwrap();
        let err = pak.read_file(d).unwrap_err();
        assert!(matches!(err, PakError::IsDir(_)));
        assert_eq!(err.to_string(), "dir1 是目录");
    }

    // ---- 3. Latin-1 文件名：逐字节映射，绝不做 UTF-8 有损解码 ----------------

    #[test]
    fn latin1_names() {
        // 0xE9 在 Latin-1 是 é（U+00E9）；名字字节 63 61 66 E9 解码为 4 字符 "café"
        // 文件名字节 E4 B8 AD 解码为 "ä¸­"（3 个 Latin-1 字符），其字节恰好
        // 等于 "中" 的 UTF-8 编码——若按字节比较 find("中") 会误命中，
        // 按 Latin-1 字符比较（Python str 语义）则正确区分，此为语义红线
        let pak = open_bytes(
            &build_pak(vec![
                raw_file(b"caf\xe9", b"x"),
                raw_file(b"\xe4\xb8\xad", b"y"),
            ]),
            "latin1",
        )
        .unwrap();
        assert_eq!(walk_rel(&pak), ["caf\u{e9}", "\u{e4}\u{b8}\u{ad}"]);
        assert!(pak.find("caf\u{e9}").is_some()); // Latin-1 字符语义命中
        assert_eq!(pak.read_file(pak.find("caf\u{e9}").unwrap()).unwrap(), b"x");
        assert!(pak.find("\u{e4}\u{b8}\u{ad}").is_some()); // 逐字节映射字符命中
        assert!(pak.find("中").is_none()); // 同字节不同字符，不得误命中
        assert!(pak.find("").is_none()); // 空路径必失配（空名被 BadName 拒绝）
        assert!(pak.find("nope").is_none());
    }

    // ---- 4. 头部防御：Truncated / BadMagic ----------------------------------

    #[test]
    fn header_defense() {
        let err = open_bytes(&[0u8; 15], "tiny").unwrap_err();
        assert!(matches!(err, PakError::Truncated));
        assert_eq!(err.to_string(), "文件太小，不是 PAK");

        let mut bytes = header_bytes(0);
        bytes[0..4].copy_from_slice(&0x00000099u32.to_le_bytes()); // 篡改 magic
        let err = open_bytes(&bytes, "magic").unwrap_err();
        assert!(matches!(err, PakError::BadMagic(0x99)));
        assert_eq!(err.to_string(), "魔数不对: 0x00000099 (期望 0x11223344)");
    }

    // ---- 5. 目录表防御：CatalogBadSize / CatalogOob --------------------------

    #[test]
    fn catalog_bad_size() {
        // root_size=65 非 64 倍数（先于越界检查触发）
        let bytes = header_bytes(65);
        let err = open_bytes(&bytes, "badsize").unwrap_err();
        assert!(matches!(err, PakError::CatalogBadSize { offset: 16, size: 65 }));
        assert_eq!(err.to_string(), "目录表大小 65 不是 64 的倍数 (offset=0x10)");
    }

    #[test]
    fn catalog_oob_root() {
        // 根表声明 64 字节但文件只有 16 字节头
        let bytes = header_bytes(CAT_SIZE as u32);
        let err = open_bytes(&bytes, "oob_root").unwrap_err();
        assert!(matches!(err, PakError::CatalogOob { offset: 16, size: 64 }));
        assert_eq!(err.to_string(), "目录表越界: offset=0x10 size=64");
    }

    /// u64 越界回绕证明（目录表路径）：子目录条目 offset=0xFFFFFFF0, size=0x40，
    /// u32 域相加回绕为 0x30 会绕过检查，u64 域 0x100000030 > 文件长被拦截。
    #[test]
    fn catalog_oob_u64_wraparound() {
        let mut bytes = header_bytes(CAT_SIZE as u32);
        // 根表恰好落在文件内，但其中的目录条目指向回绕区
        bytes.extend_from_slice(&raw_entry(DIR_BIT, 0xFFFFFFF0, 0x40, 0x40, b"d"));
        let err = open_bytes(&bytes, "oob_wrap").unwrap_err();
        assert!(matches!(
            err,
            PakError::CatalogOob { offset: 0xFFFFFFF0, size: 0x40 }
        ));
        assert_eq!(err.to_string(), "目录表越界: offset=0xfffffff0 size=64");
    }

    // ---- 6. 目录树防御：TreeCycle / TreeTooDeep ------------------------------

    /// 循环引用：目录 a 的子表内条目 b 指回 a 的子表自身（(80,64) 二次引用）
    #[test]
    fn tree_cycle_detected() {
        let mut bytes = header_bytes(CAT_SIZE as u32);
        // 根表(16)：Dir "a" -> 子表(80,64)；子表(80)：Dir "b" -> (80,64) 指回自身
        bytes.extend_from_slice(&raw_entry(DIR_BIT, 80, 64, 64, b"a"));
        bytes.extend_from_slice(&raw_entry(DIR_BIT, 80, 64, 64, b"b"));
        let err = open_bytes(&bytes, "cycle").unwrap_err();
        assert!(matches!(err, PakError::TreeCycle(80)));
        assert_eq!(err.to_string(), "目录表被重复引用，疑似损坏/循环 (offset=0x50)");
    }

    /// 深度防御：65 层目录链，第 66 次递归（depth=65）触发 TreeTooDeep。
    /// 末层条目指向虚构 offset=0x100000（深度检查先于读取，无需该位置有效）。
    #[test]
    fn tree_too_deep() {
        let mut bytes = header_bytes(CAT_SIZE as u32);
        // T0(根,16) T1(80) ... T64(16+64*64=4112)，每表 1 个目录条目指向下一表
        for i in 0..=64u32 {
            let offset = if i == 64 { 0x100000 } else { 16 + 64 * (i + 1) };
            bytes.extend_from_slice(&raw_entry(DIR_BIT, offset, 64, 64, b"d"));
        }
        assert_eq!(bytes.len(), 16 + 65 * CAT_SIZE); // 65 张表全部落在文件内
        let err = open_bytes(&bytes, "deep").unwrap_err();
        assert!(matches!(err, PakError::TreeTooDeep(0x100000)));
        assert_eq!(err.to_string(), "目录嵌套过深 (offset=0x100000)");
    }

    // ---- 7. 非法文件名：BadName（空 / '.' / '..' / 含 '/'） ------------------

    #[test]
    fn bad_entry_names() {
        for (name, tag) in [(b"" as &[u8], "empty"), (b".", "dot"), (b"..", "dotdot"), (b"a/b", "slash")] {
            let mut bytes = header_bytes(CAT_SIZE as u32);
            bytes.extend_from_slice(&raw_entry(RAW_BIT, 16 + 64, 0, 0, name));
            let err = open_bytes(&bytes, tag).unwrap_err();
            assert!(matches!(err, PakError::BadName(_)), "名字 {name:?} 应为 BadName");
            assert_eq!(
                err.to_string(),
                format!("包内非法文件名: '{}'（疑似路径穿越或损坏）", latin1(name))
            );
        }
    }

    // ---- 8. 文件数据防御：DataOob / DataShort / SizeMismatch -----------------

    /// u64 越界回绕证明（文件数据路径）：offset=0xFFFFFFF0, size=0x20 在 u32 域
    /// 相加回绕为 0x10 绕过检查；u64 域 0x100000010 > 文件长 -> DataOob。
    #[test]
    fn data_oob_u64_wraparound() {
        let mut bytes = header_bytes(CAT_SIZE as u32);
        bytes.extend_from_slice(&raw_entry(RAW_BIT, 0xFFFFFFF0, 0x20, 0x10, b"x"));
        let pak = open_bytes(&bytes, "data_wrap").unwrap(); // 解析期不读文件数据
        let node = pak.find("x").unwrap();
        let err = pak.read_file(node).unwrap_err();
        assert!(matches!(err, PakError::DataOob(_)));
        assert_eq!(err.to_string(), "读取越界: x");
    }

    /// u32 域满值回绕：offset=size=0xFFFFFFFF，u64 和 0x1FFFFFFFE 必然越界
    #[test]
    fn data_oob_u32_full_wraparound() {
        let mut bytes = header_bytes(CAT_SIZE as u32);
        bytes.extend_from_slice(&raw_entry(RAW_BIT, 0xFFFFFFFF, 0xFFFFFFFF, 1, b"x"));
        let pak = open_bytes(&bytes, "data_full").unwrap();
        let node = pak.find("x").unwrap();
        assert!(matches!(
            pak.read_file(node).unwrap_err(),
            PakError::DataOob(_)
        ));
    }

    #[test]
    fn raw_data_short() {
        let pak = open_bytes(
            &build_pak(vec![TestNode::File {
                name: b"c",
                attrib: RAW_BIT,
                org: 5,
                data: b"ab".to_vec(), // 存储 2 字节 < 声明 org 5
            }]),
            "short",
        )
        .unwrap();
        let node = pak.find("c").unwrap();
        let err = pak.read_file(node).unwrap_err();
        assert!(matches!(err, PakError::DataShort { stored: 2, org: 5, .. }));
        assert_eq!(err.to_string(), "数据不足: c 存储 2 < 原始 5 字节");
    }

    /// size=org=0 的幽灵条目：offset 越过文件尾仍读出空内容而非越界错误
    /// （对齐 Python `read(0)` 永不短读；etc.pak 的 task_score.list 实况，
    /// 截断 pak 的 check --deep 由此不把它计入"解压失败"）。
    #[test]
    fn empty_entry_beyond_eof_reads_empty() {
        let mut bytes = header_bytes(CAT_SIZE as u32);
        bytes.extend_from_slice(&raw_entry(0, 0xFFFFFFF0, 0, 0, b"ghost"));
        let pak = open_bytes(&bytes, "ghost").unwrap();
        let node = pak.find("ghost").unwrap();
        assert_eq!(pak.read_file(node).unwrap(), b"");
        assert_eq!(pak.read_stored(node).unwrap(), b"");
    }

    /// LZSS 压缩流提前耗尽（FR-20）：产出 2 字节即数据用尽，
    /// 安全返回已产出字节后由长度校验报 SizeMismatch，不 panic。
    #[test]
    fn lzss_size_mismatch_on_exhausted_stream() {
        // flag=0x03 -> 两个字面量 'A''B'，org 声明 10 -> 解压耗尽只得 2 字节
        let pak = open_bytes(
            &build_pak(vec![lzss_file(b"b", vec![0x03, b'A', b'B'], 10)]),
            "mismatch",
        )
        .unwrap();
        let node = pak.find("b").unwrap();
        let err = pak.read_file(node).unwrap_err();
        assert!(matches!(err, PakError::SizeMismatch { got: 2, declared: 10, .. }));
        assert_eq!(err.to_string(), "解压大小不符: b 得到 2 != 声明 10 字节（数据损坏）");
    }

    // ---- 9. LZSS 读取路径（手工构造压缩流，格式与解压器对齐） ----------------

    #[test]
    fn lzss_literal_and_match_streams() {
        // 匹配项：flag 位=0，(i=0,j=0) -> off=0、长度 (0+2)+1=3，初始窗口 buf[0..3]=空格
        let pak = open_bytes(
            &build_pak(vec![
                lzss_file(b"lit", vec![0xFF, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H'], 8),
                lzss_file(b"mat", vec![0x00, 0x00, 0x00], 3),
                lzss_file(b"empty", vec![], 0), // 0 字节文件（FR-40）
            ]),
            "lzss",
        )
        .unwrap();
        assert_eq!(pak.read_file(pak.find("lit").unwrap()).unwrap(), b"ABCDEFGH");
        assert_eq!(pak.read_file(pak.find("mat").unwrap()).unwrap(), b"   ");
        assert_eq!(pak.read_file(pak.find("empty").unwrap()).unwrap(), b"");
    }

    // ---- 10. 真实 pak 冒烟（外部大文件，#[ignore]，本地须跑通一次） -----------

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    #[ignore = "依赖外部大文件 release/etc.pak（42MB）"]
    fn real_pak_smoke() {
        let pak = PakReader::open(Path::new("/root/wen1.60/release/etc.pak")).unwrap();
        assert_eq!(pak.file_size, 42_285_854);
        assert_eq!(pak.root_size, 190_720); // 2980 * 64
        let walk = pak.walk();
        assert_eq!(walk.len(), 2980); // 2980 files, 0 dirs（Python 实测）
        assert!(walk.iter().all(|(_, n)| !n.is_dir()));

        // 已知 LZSS 条目（attrib=0x00000000），read_file 结果与 Python 版 SHA256 一致
        let n = pak.find("100heis.list").unwrap();
        assert_eq!(n.entry.attrib, 0);
        assert_eq!(n.entry.size, 1506);
        assert_eq!(n.entry.org, 7291);
        assert_eq!(pak.read_stored(n).unwrap().len(), 1506);
        let data = pak.read_file(n).unwrap();
        use sha2::{Digest, Sha256};
        assert_eq!(
            hex(&Sha256::digest(&data)),
            "b2f1bb8f9588818c5dfb371728b1a5b9d09b8e50857a9f23715e0d9ef1490c83"
        );

        let n2 = pak.find("zzz30011.list").unwrap();
        let d2 = pak.read_file(n2).unwrap();
        assert_eq!(d2.len() as u32, n2.entry.org);
        assert_eq!(
            hex(&Sha256::digest(&d2)),
            "5598f177628c0cc8790702f533b284074cb3476510cec2e3af77c594d2a4e83f"
        );
    }

    /// lib_gs32.pak 为目录树结构，验证真实嵌套目录的解析与 walk
    #[test]
    #[ignore = "依赖外部大文件 release/lib_gs32.pak（49MB）"]
    fn real_tree_pak_smoke() {
        let pak = PakReader::open(Path::new("/root/wen1.60/release/lib_gs32.pak")).unwrap();
        let walk = pak.walk();
        let dirs = walk.iter().filter(|(_, n)| n.is_dir()).count();
        assert!(dirs > 0, "lib_gs32 应含目录条目");
        // 全部 relpath 均可经 find 命中（walk/find 两条路径互验）
        for (rel, _) in &walk {
            assert!(pak.find(rel).is_some(), "find 应命中 walk 产物: {rel}");
        }
        // 深度优先顺序：每个条目的父目录必然先于它出现（隐含目录先于其子项）
        let index: std::collections::HashMap<&str, usize> = walk
            .iter()
            .enumerate()
            .map(|(i, (r, _))| (r.as_str(), i))
            .collect();
        for (rel, _) in &walk {
            if let Some((parent, _)) = rel.rsplit_once('/') {
                assert!(
                    index[parent] < index[rel.as_str()],
                    "父目录 {parent} 应先于 {rel}"
                );
            }
        }
    }
}