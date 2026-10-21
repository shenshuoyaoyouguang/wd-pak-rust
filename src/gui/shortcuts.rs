//! 快捷键纯逻辑（G6-04）。

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Char(char),
    F(u8),
    Enter,
    Escape,
    Tab,
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    pub fn new(ctrl: bool, shift: bool, alt: bool, key: Key) -> Self {
        Self { ctrl, shift, alt, key }
    }

    pub fn ctrl_key(key: Key) -> Self {
        Self::new(true, false, false, key)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShortcutMap {
    pub map: BTreeMap<Shortcut, String>,
}

impl ShortcutMap {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// 绑定快捷键，返回被覆盖的旧动作（无冲突时为 `None`）。
    pub fn bind(&mut self, sc: Shortcut, action: &str) -> Option<String> {
        self.map.insert(sc, action.to_string())
    }

    pub fn resolve(&self, sc: Shortcut) -> Option<&str> {
        self.map.get(&sc).map(|s| s.as_str())
    }

    pub fn has_conflict(&self, sc: Shortcut) -> bool {
        self.map.contains_key(&sc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_and_resolve() {
        let mut m = ShortcutMap::new();
        assert_eq!(m.bind(Shortcut::ctrl_key(Key::Char('o')), "open"), None);
        assert_eq!(m.resolve(Shortcut::ctrl_key(Key::Char('o'))), Some("open"));
        assert_eq!(
            m.bind(Shortcut::ctrl_key(Key::Char('o')), "export"),
            Some("open".into())
        );
        assert_eq!(m.resolve(Shortcut::ctrl_key(Key::Char('o'))), Some("export"));
    }
}
