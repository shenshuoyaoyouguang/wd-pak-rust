//! format 层集成测试：通过库公开 API 验证编解码 roundtrip、小端布局与非法输入。
//! 期望值参考 Python 版 test_wd_pak_tool.py 的 write_flat_pak 构造方式
//! （HDR.pack(MAGIC, 64*n, 0, 0) 与 CAT.pack(..., name.ljust(44, b'\0'))）。

use wd_pak::error::PakError;
use wd_pak::format::{
    CatalogEntry, Header, CAT_SIZE, DIR_BIT, HDR_SIZE, MAGIC, MAX_NAME, NAME_LEN, RAW_BIT,
};

fn entry(attrib: u32, name: &[u8]) -> CatalogEntry {
    CatalogEntry {
        attrib,
        offset: 16,
        size: 0x5678,
        org: 0x9ABC,
        mtime: 0x0DEF_0123,
        name: name.to_vec(),
    }
}

#[test]
fn header_roundtrip_and_size() {
    let h = Header {
        magic: MAGIC,
        root_dir_size: 128,
        unused1: 0,
        unused2: 0,
    };
    let b = h.to_bytes();
    assert_eq!(b.len(), HDR_SIZE);
    assert_eq!(Header::from_bytes(&b).unwrap(), h);
}

#[test]
fn header_matches_python_layout() {
    // Python: HDR.pack(MAGIC, 64*2, 0, 0) 对应的手工小端字节
    let mut b = [0u8; HDR_SIZE];
    b[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    b[4..8].copy_from_slice(&128u32.to_le_bytes());
    let h = Header::from_bytes(&b).unwrap();
    assert_eq!(h.magic, MAGIC);
    assert_eq!(h.root_dir_size, 128);
    assert_eq!(h.to_bytes(), b);
}

#[test]
fn header_rejects_bad_magic() {
    let mut b = [0u8; HDR_SIZE];
    b[0..4].copy_from_slice(&0x8877_6655u32.to_le_bytes());
    assert!(matches!(
        Header::from_bytes(&b),
        Err(PakError::BadMagic(0x8877_6655))
    ));
}

#[test]
fn header_rejects_truncated() {
    assert!(matches!(Header::from_bytes(&[0u8; 15]), Err(PakError::Truncated)));
    // 足 16 字节但全 0：魔数不对（而非截断）
    assert!(matches!(
        Header::from_bytes(&[0u8; 16]),
        Err(PakError::BadMagic(_))
    ));
}

#[test]
fn catalog_roundtrip_all_field_combinations() {
    let cases: Vec<(u32, Vec<u8>)> = vec![
        (0, b"file.txt".to_vec()),
        (RAW_BIT, b"a".to_vec()),
        (DIR_BIT, vec![b'n'; MAX_NAME]),
    ];
    for (attrib, name) in cases {
        let e = entry(attrib, &name);
        let b = e.to_bytes().unwrap();
        assert_eq!(b.len(), CAT_SIZE);
        assert_eq!(CatalogEntry::from_bytes(&b).unwrap(), e);
    }
}

#[test]
fn catalog_layout_matches_python_cat_pack() {
    // Python: CAT.pack(attrib, offset, size, org, mt, name.ljust(44, b'\0'))
    let e = entry(DIR_BIT, b"dir");
    let b = e.to_bytes().unwrap();
    assert_eq!(&b[0..4], &DIR_BIT.to_le_bytes());
    assert_eq!(&b[4..8], &16u32.to_le_bytes());
    assert_eq!(&b[8..12], &0x5678u32.to_le_bytes());
    assert_eq!(&b[12..16], &0x9ABCu32.to_le_bytes());
    assert_eq!(&b[16..20], &0x0DEF_0123u32.to_le_bytes());
    assert_eq!(&b[20..23], b"dir");
    assert!(b[23..20 + NAME_LEN].iter().all(|&c| c == 0));
}

#[test]
fn catalog_name_nul_split_semantics() {
    // 与 Python split(b'\0',1)[0] 一致：\0 之后的填充/垃圾字节不影响解码
    let mut b = entry(0, b"ok").to_bytes().unwrap();
    for c in &mut b[23..20 + NAME_LEN] {
        *c = 0xEE;
    }
    assert_eq!(CatalogEntry::from_bytes(&b).unwrap().name, b"ok".to_vec());
    // 无 \0 的满 44 字节名：提取满 44 字节后因超 43 字节被拒
    let mut b2 = [0u8; CAT_SIZE];
    b2[20..].fill(b'q');
    assert!(matches!(
        CatalogEntry::from_bytes(&b2),
        Err(PakError::NameTooLong(_))
    ));
}

#[test]
fn catalog_to_bytes_rejects_invalid_names() {
    assert!(matches!(entry(0, b"").to_bytes(), Err(PakError::BadName(_))));
    assert!(matches!(entry(0, b"..").to_bytes(), Err(PakError::BadName(_))));
    assert!(matches!(entry(0, b"a/b").to_bytes(), Err(PakError::BadName(_))));
    assert!(matches!(entry(0, b"a\\b").to_bytes(), Err(PakError::BadName(_))));
    assert!(matches!(
        entry(0, &[b'x'; MAX_NAME + 1]).to_bytes(),
        Err(PakError::NameTooLong(_))
    ));
}

#[test]
fn catalog_flag_queries() {
    assert!(entry(DIR_BIT, b"d").is_dir());
    assert!(!entry(DIR_BIT, b"d").is_raw());
    assert!(entry(RAW_BIT, b"f").is_raw());
    assert!(!entry(RAW_BIT, b"f").is_dir());
    assert!(!entry(0, b"f").is_dir());
    assert!(!entry(0, b"f").is_raw());
}