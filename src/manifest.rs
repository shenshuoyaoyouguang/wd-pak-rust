//! _pak_manifest.json —— 保序序列化与宽松读入。
//!
//! 序列化逐字节对齐 Python `json.dump(mf, fh, indent=1, ensure_ascii=False)`：
//!   * 键序：顶层 pak -> files -> order（Python dict 字面量插入序）；
//!     files 键序 = walk 遍历序（依赖 serde_json 的 preserve_order feature）；
//!   * 缩进 1 空格：`PrettyFormatter::with_indent(b" ")` 与 Python indent=1
//!     完全同构 —— 冒号后单空格、项间 `,\n`、空容器输出 `{}` / `[]`、末尾无换行；
//!   * 非 ASCII 直出 UTF-8（ensure_ascii=False 语义），控制字符与 `"` `\`
//!     的转义规则两者一致（serde_json ESCAPE 表仅覆盖 <0x20 与两个标点）。
//!
//! 读入对齐 Python `_load_manifest`：文件缺失或 JSON 解析失败静默返回 None；
//! 字段缺失按空值处理。mtime 数值域校验（FR-37）属 pack 侧职责，不在此处。

use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::PakError;

/// manifest 文件名（extract 写出 / pack 读取并自动跳过）
pub const MANIFEST_NAME: &str = "_pak_manifest.json";

/// manifest files 表的值域（子键序固定 attrib / mtime / stored / orig）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub attrib: u32,
    pub mtime: u32,
    pub stored: u64,
    pub orig: u64,
}

/// manifest 数据。files 保插入序（= 遍历序）；order 为全部条目序（含目录）。
#[derive(Debug, Default)]
pub struct Manifest {
    pub pak: String,
    pub files: Vec<(String, FileStat)>,
    pub order: Vec<String>,
}

impl Manifest {
    /// 序列化到内存缓冲（UTF-8、1 空格缩进、无尾换行）。
    /// Value 序列化无 io 失败面，Err 仅在理论上存在，保留签名以统一调用侧。
    pub fn to_bytes(&self) -> Result<Vec<u8>, PakError> {
        let mut buf = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(
            &mut buf,
            serde_json::ser::PrettyFormatter::with_indent(b" "),
        );
        self.to_value()
            .serialize(&mut ser)
            .map_err(|e| PakError::Manifest(format!("写出 manifest 失败: {e}")))?;
        Ok(buf)
    }

    /// 构造 JSON 值（键序由 preserve_order 的 Map 即 IndexMap 保证）。
    fn to_value(&self) -> Value {
        let mut files = Map::new();
        for (rel, st) in &self.files {
            let mut m = Map::new();
            m.insert("attrib".into(), Value::from(st.attrib));
            m.insert("mtime".into(), Value::from(st.mtime));
            m.insert("stored".into(), Value::from(st.stored));
            m.insert("orig".into(), Value::from(st.orig));
            files.insert(rel.clone(), Value::Object(m));
        }
        let mut root = Map::new();
        root.insert("pak".into(), Value::String(self.pak.clone()));
        root.insert("files".into(), Value::Object(files));
        root.insert(
            "order".into(),
            Value::Array(
                self.order
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
        Value::Object(root)
    }

    /// 宽松读入（对齐 Python `_load_manifest`）：缺失 / 语法错误 -> None；
    /// 结构字段缺失或类型不符按空值 / 0 处理（Python `.get(..., 默认)` 语义）。
    pub fn load(dir: &std::path::Path) -> Option<Manifest> {
        let text = std::fs::read_to_string(dir.join(MANIFEST_NAME)).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        let pak = v
            .get("pak")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let order = v
            .get("order")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let files = v
            .get("files")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter(|(_, x)| x.is_object())
                    .map(|(rel, x)| {
                        let num = |k: &str| x.get(k).and_then(Value::as_u64).unwrap_or(0);
                        (
                            rel.clone(),
                            FileStat {
                                attrib: num("attrib") as u32,
                                mtime: num("mtime") as u32,
                                stored: num("stored"),
                                orig: num("orig"),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Manifest { pak, files, order })
    }
}

// ---------------------------------------------------------------------------
// 单元测试：格式锚点手写期望串（与 Python json.dump 输出逐字符对照）
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ser_matches_python_indent1() {
        let mf = Manifest {
            pak: "etc.pak".into(),
            files: vec![(
                "100heis.list".into(),
                FileStat {
                    attrib: 0,
                    mtime: 1150388208,
                    stored: 1506,
                    orig: 7291,
                },
            )],
            order: vec!["100heis.list".into()],
        };
        let expect = concat!(
            "{\n",
            " \"pak\": \"etc.pak\",\n",
            " \"files\": {\n",
            "  \"100heis.list\": {\n",
            "   \"attrib\": 0,\n",
            "   \"mtime\": 1150388208,\n",
            "   \"stored\": 1506,\n",
            "   \"orig\": 7291\n",
            "  }\n",
            " },\n",
            " \"order\": [\n",
            "  \"100heis.list\"\n",
            " ]\n",
            "}",
        );
        assert_eq!(String::from_utf8(mf.to_bytes().unwrap()).unwrap(), expect);
    }

    /// 空容器输出 {} / []（Python json.dump(indent=1) 对空 dict/list 相同）
    #[test]
    fn ser_empty_containers() {
        let mf = Manifest {
            pak: "x.pak".into(),
            files: vec![],
            order: vec![],
        };
        let s = String::from_utf8(mf.to_bytes().unwrap()).unwrap();
        assert_eq!(s, "{\n \"pak\": \"x.pak\",\n \"files\": {},\n \"order\": []\n}");
    }

    /// 非 ASCII 键直出 UTF-8（ensure_ascii=False）+ load 往返保序
    #[test]
    fn ser_non_ascii_and_load_roundtrip() {
        let mf = Manifest {
            pak: "x.pak".into(),
            files: vec![(
                "caf\u{e9}.list".into(),
                FileStat {
                    attrib: 1,
                    mtime: 2,
                    stored: 3,
                    orig: 4,
                },
            )],
            order: vec!["caf\u{e9}.list".into(), "z".into()],
        };
        let bytes = mf.to_bytes().unwrap();
        // é (U+00E9) 的 UTF-8 编码为 C3 A9，应原样出现在键名字节中
        assert!(bytes.starts_with(
            b"{\n \"pak\": \"x.pak\",\n \"files\": {\n  \"caf\xc3\xa9.list\"".as_slice()
        ));
        assert!(!bytes.windows(3).any(|w| w == b"\\u"), "不应出现 \\u 转义");

        let dir = std::env::temp_dir().join("wd_pak_manifest_roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_NAME), &bytes).unwrap();
        let back = Manifest::load(&dir).unwrap();
        assert_eq!(back.pak, "x.pak");
        assert_eq!(back.order, vec!["caf\u{e9}".to_string() + ".list", "z".to_string()]);
        assert_eq!(back.files.len(), 1);
        assert_eq!(back.files[0].0, "caf\u{e9}.list");
        assert_eq!(
            back.files[0].1,
            FileStat {
                attrib: 1,
                mtime: 2,
                stored: 3,
                orig: 4
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 宽松读入：不存在 -> None；坏 JSON -> None
    #[test]
    fn load_lenient() {
        let dir = std::env::temp_dir().join("wd_pak_manifest_lenient");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Manifest::load(&dir).is_none());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_NAME), b"{oops").unwrap();
        assert!(Manifest::load(&dir).is_none());
        // 空 JSON 对象：pak 缺失为空串、order/files 为空（Python .get 默认值语义）
        std::fs::write(dir.join(MANIFEST_NAME), b"{}").unwrap();
        let mf = Manifest::load(&dir).unwrap();
        assert_eq!(mf.pak, "");
        assert!(mf.order.is_empty());
        assert!(mf.files.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}