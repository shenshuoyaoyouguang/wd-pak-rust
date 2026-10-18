//! 二进制格式层：magic_Linux32 (Fm_Header / Fm_Catalog) PAK 的头部与目录项编解码。
//!
//! 格式（逆向自 magic_Linux32 调试信息，与 Python 版 wd_pak_tool.py 一致）：
//!   Header (16B):  u32 magic=0x11223344, u32 rootDirSize, u32 unused1, u32 unused2
//!   Catalog(64B):  u32 nAttrib, u32 nOffset, u32 nSize, u32 nOrgSize,
//!                  u32 nModifiedTime, char name[44]  (0 结尾, 不足补 0)
//!   nAttrib:  bit0=1  目录（其数据=子目录表, 原样存储, nSize=目录表字节数）
//!             bit31=1 文件原样存储（未压缩）
//!             否则    文件为 LZSS 压缩（LZSS 常量 N/F 等属于压缩层，见后续 lzss.rs）
//! 布局: header -> 根目录表 -> 深度优先排列的各条目数据（目录=子目录表）。
//! 所有多字节整数均为小端；解码显式使用 from_le_bytes，禁止 packed 强转。

use crate::error::PakError;

/// 魔数（magic_Linux32 固定值）
pub const MAGIC: u32 = 0x1122_3344;
/// 头部字节数
pub const HDR_SIZE: usize = 16;
/// 目录项字节数
pub const CAT_SIZE: usize = 64;
/// name 字段字节数
pub const NAME_LEN: usize = 44;
/// 名字最大字节数（44 含结尾 0）
pub const MAX_NAME: usize = 43;
/// attrib bit0：目录
pub const DIR_BIT: u32 = 0x0000_0001;
/// attrib bit31：原样存储（未压缩）
pub const RAW_BIT: u32 = 0x8000_0000;

/// Latin-1 字节按同值码点映射为 String（对齐 Python bytes.decode('latin1')），
/// 供错误消息构造使用；名字的比较与编码始终使用原始字节。
pub(crate) fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// 文件头（16 字节，小端，Python struct '<4I'）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub magic: u32,
    pub root_dir_size: u32,
    pub unused1: u32,
    pub unused2: u32,
}

impl Header {
    /// 从字节解码（检查对齐 Python Pak.__init__）：
    /// 不足 16 字节 -> Truncated；魔数不符 -> BadMagic。
    pub fn from_bytes(b: &[u8]) -> Result<Self, PakError> {
        if b.len() < HDR_SIZE {
            return Err(PakError::Truncated);
        }
        let magic = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        if magic != MAGIC {
            return Err(PakError::BadMagic(magic));
        }
        Ok(Self {
            magic,
            root_dir_size: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            unused1: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            unused2: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
        })
    }

    /// 编码为 16 字节小端。
    pub fn to_bytes(&self) -> [u8; HDR_SIZE] {
        let mut b = [0u8; HDR_SIZE];
        b[0..4].copy_from_slice(&self.magic.to_le_bytes());
        b[4..8].copy_from_slice(&self.root_dir_size.to_le_bytes());
        b[8..12].copy_from_slice(&self.unused1.to_le_bytes());
        b[12..16].copy_from_slice(&self.unused2.to_le_bytes());
        b
    }
}

/// 目录项（64 字节，小端，Python struct '<5I44s'）。
/// name 为原始 Latin-1 字节（到首个 \0 为止），绝不 decode 为 UTF-8。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub attrib: u32,
    pub offset: u32,
    pub size: u32,
    pub org: u32,
    pub mtime: u32,
    pub name: Vec<u8>,
}

impl CatalogEntry {
    /// 从 64 字节解码：name 取到首个 \0 为止（无 \0 取满 44 字节，
    /// 与 Python name.split(b'\0',1)[0] 语义一致），随后做名字合法性校验。
    pub fn from_bytes(b: &[u8; CAT_SIZE]) -> Result<Self, PakError> {
        let attrib = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let offset = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        let size = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        let org = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
        let mtime = u32::from_le_bytes([b[16], b[17], b[18], b[19]]);
        let raw = &b[20..20 + NAME_LEN];
        let name = match raw.iter().position(|&c| c == 0) {
            Some(i) => raw[..i].to_vec(),
            None => raw.to_vec(),
        };
        Self::validate_name(&name)?;
        Ok(Self {
            attrib,
            offset,
            size,
            org,
            mtime,
            name,
        })
    }

    /// 编码为 64 字节：name 校验通过后零填充到 44 字节。
    pub fn to_bytes(&self) -> Result<[u8; CAT_SIZE], PakError> {
        Self::validate_name(&self.name)?;
        let mut b = [0u8; CAT_SIZE];
        b[0..4].copy_from_slice(&self.attrib.to_le_bytes());
        b[4..8].copy_from_slice(&self.offset.to_le_bytes());
        b[8..12].copy_from_slice(&self.size.to_le_bytes());
        b[12..16].copy_from_slice(&self.org.to_le_bytes());
        b[16..20].copy_from_slice(&self.mtime.to_le_bytes());
        b[20..20 + self.name.len()].copy_from_slice(&self.name);
        Ok(b)
    }

    /// 是否目录项（attrib bit0；其数据为子目录表）。
    pub fn is_dir(&self) -> bool {
        self.attrib & DIR_BIT != 0
    }

    /// 是否原样存储（attrib bit31；未压缩）。
    pub fn is_raw(&self) -> bool {
        self.attrib & RAW_BIT != 0
    }

    /// 名字按 UTF-8 有损解码返回，仅用于显示（比较与编码请用原始字节）。
    pub fn name_lossy(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }

    /// 名字合法性校验（合并 Python 写侧 _validate_name 与读侧 _check_entry_name）：
    /// 拒空、拒 '.' '..'、拒含 '/' '\\' '\\0'（-> BadName）、拒超 43 字节（-> NameTooLong）。
    /// Latin-1 单字节编码对 &[u8] 天然满足，无需额外检查。
    pub fn validate_name(name: &[u8]) -> Result<(), PakError> {
        let bad = name.is_empty()
            || name == b"."
            || name == b".."
            || name.iter().any(|&c| c == b'/' || c == b'\\' || c == 0);
        if bad {
            return Err(PakError::BadName(latin1(name)));
        }
        if name.len() > MAX_NAME {
            return Err(PakError::NameTooLong(latin1(name)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &[u8]) -> CatalogEntry {
        CatalogEntry {
            attrib: DIR_BIT | RAW_BIT,
            offset: 0x0102_0304,
            size: 0x0A0B_0C0D,
            org: 0x1122_3344,
            mtime: 0x7FFF_FFFF,
            name: name.to_vec(),
        }
    }

    #[test]
    fn header_roundtrip() {
        let h = Header {
            magic: MAGIC,
            root_dir_size: 64,
            unused1: 0,
            unused2: 0,
        };
        assert_eq!(Header::from_bytes(&h.to_bytes()).unwrap(), h);
    }

    #[test]
    fn header_little_endian_layout() {
        // 手工构造小端字节：magic=0x11223344 -> 44 33 22 11；root=0x0A0B0C0D -> 0D 0C 0B 0A
        let mut b = [0u8; HDR_SIZE];
        b[0..4].copy_from_slice(&[0x44, 0x33, 0x22, 0x11]);
        b[4..8].copy_from_slice(&[0x0D, 0x0C, 0x0B, 0x0A]);
        let h = Header::from_bytes(&b).unwrap();
        assert_eq!(h.magic, 0x11223344);
        assert_eq!(h.root_dir_size, 0x0A0B0C0D);
        assert_eq!(
            &h.to_bytes()[0..8],
            &[0x44, 0x33, 0x22, 0x11, 0x0D, 0x0C, 0x0B, 0x0A]
        );
    }

    #[test]
    fn header_rejects_short_and_bad_magic() {
        assert!(matches!(Header::from_bytes(&[0u8; 15]), Err(PakError::Truncated)));
        let mut b = [0u8; HDR_SIZE];
        b[0..4].copy_from_slice(&0x5566_7788u32.to_le_bytes());
        assert!(matches!(Header::from_bytes(&b), Err(PakError::BadMagic(0x5566_7788))));
    }

    #[test]
    fn catalog_roundtrip() {
        let e = entry(b"etc.pak");
        assert_eq!(CatalogEntry::from_bytes(&e.to_bytes().unwrap()).unwrap(), e);
        // 名字恰好 43 字节（合法上限，无 \0 结尾空间）也能 roundtrip
        let e43 = entry(&[b'x'; MAX_NAME]);
        assert_eq!(CatalogEntry::from_bytes(&e43.to_bytes().unwrap()).unwrap(), e43);
    }

    #[test]
    fn catalog_name_padded_and_truncated() {
        // to_bytes：短名零填充到 44 字节
        let b = entry(b"ab").to_bytes().unwrap();
        assert_eq!(&b[20..22], b"ab");
        assert!(b[22..20 + NAME_LEN].iter().all(|&c| c == 0));
        // from_bytes：\0 终止符之后的填充/垃圾字节被截断（Python split(b'\0',1)[0] 语义）。
        // 保留 b[22]=0 为终止符，仅污染其后的字节。
        let mut b2 = b;
        for c in &mut b2[23..20 + NAME_LEN] {
            *c = 0xFF;
        }
        assert_eq!(CatalogEntry::from_bytes(&b2).unwrap().name, b"ab".to_vec());
    }

    #[test]
    fn catalog_name_full_44_rejected() {
        // 无 \0 的满 44 字节名：提取满 44 字节，因超过 43 字节上限被拒
        let mut b = [0u8; CAT_SIZE];
        b[20..].fill(b'z');
        assert!(matches!(
            CatalogEntry::from_bytes(&b),
            Err(PakError::NameTooLong(_))
        ));
    }

    #[test]
    fn catalog_little_endian_layout() {
        // attrib=DIR_BIT|RAW_BIT=0x80000001 编码为 01 00 00 80；offset 最低字节在前
        let b = entry(b"a").to_bytes().unwrap();
        assert_eq!(&b[0..4], &[0x01, 0x00, 0x00, 0x80]);
        assert_eq!(&b[4..8], &0x0102_0304u32.to_le_bytes());
        assert_eq!(&b[8..12], &0x0A0B_0C0Du32.to_le_bytes());
    }

    #[test]
    fn validate_name_rules() {
        // 合法：普通名、恰好 43 字节、Latin-1 高位字节（0xE9 = é）
        assert!(CatalogEntry::validate_name(b"abc").is_ok());
        assert!(CatalogEntry::validate_name(&[b'x'; MAX_NAME]).is_ok());
        assert!(CatalogEntry::validate_name(&[0xE9]).is_ok());
        // 非法：空 / '.' / '..' / 含 '/' '\\' '\\0' -> BadName
        for bad in [&b""[..], b".", b"..", b"a/b", b"a\\b", b"a\0b"] {
            assert!(
                matches!(CatalogEntry::validate_name(bad), Err(PakError::BadName(_))),
                "应拒绝 {:?}",
                bad
            );
        }
        // 超长 -> NameTooLong
        assert!(matches!(
            CatalogEntry::validate_name(&[b'x'; MAX_NAME + 1]),
            Err(PakError::NameTooLong(_))
        ));
    }

    #[test]
    fn flags_and_lossy_name() {
        assert!(entry(b"d").is_dir());
        assert!(entry(b"f").is_raw());
        let mut e = entry(b"caf\xe9"); // Latin-1 é，显示时应替换为 U+FFFD
        e.attrib = DIR_BIT;
        assert!(e.is_dir() && !e.is_raw());
        assert_eq!(e.name_lossy(), "caf\u{FFFD}");
    }

    #[test]
    fn latin1_mapping() {
        assert_eq!(latin1(b"ab\xe9\xff"), "ab\u{e9}\u{ff}");
    }
}