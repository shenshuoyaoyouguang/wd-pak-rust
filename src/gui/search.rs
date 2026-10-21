//! 搜索与筛选状态模型（G2-05）。
//!
//! 只定义搜索条件与筛选状态，不涉及 Win32 控件。

pub use crate::browse::{CompressionFilter, KindFilter};

/// 搜索范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchScope {
    #[default]
    Package,
    Directory,
}

/// 搜索条件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub keyword: String,
    pub scope: SearchScope,
    pub kind: KindFilter,
    pub compression: CompressionFilter,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            keyword: String::new(),
            scope: SearchScope::Package,
            kind: KindFilter::All,
            compression: CompressionFilter::All,
        }
    }
}

impl SearchQuery {
    pub fn is_empty(&self) -> bool {
        self.keyword.is_empty()
            && self.scope == SearchScope::Package
            && self.kind == KindFilter::All
            && self.compression == CompressionFilter::All
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// 搜索状态。
#[derive(Debug, Default, Clone)]
pub struct SearchState {
    pub query: SearchQuery,
    pub hit_count: usize,
    pub debounce_ms: u32,
    pub composing: bool, // 输入法组词中
}

impl SearchState {
    pub fn new(debounce_ms: u32) -> Self {
        Self {
            query: SearchQuery::default(),
            hit_count: 0,
            debounce_ms,
            composing: false,
        }
    }

    pub fn set_keyword(&mut self, keyword: &str) {
        self.query.keyword = keyword.to_string();
    }

    pub fn set_scope(&mut self, scope: SearchScope) {
        self.query.scope = scope;
    }

    pub fn set_kind(&mut self, kind: KindFilter) {
        self.query.kind = kind;
    }

    pub fn set_compression(&mut self, compression: CompressionFilter) {
        self.query.compression = compression;
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.hit_count = 0;
    }

    pub fn update_hits(&mut self, count: usize) {
        self.hit_count = count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_query_is_empty() {
        let q = SearchQuery::default();
        assert!(q.is_empty());
    }

    #[test]
    fn clear_resets_query() {
        let mut q = SearchQuery {
            keyword: "test".into(),
            kind: KindFilter::Files,
            ..Default::default()
        };
        q.clear();
        assert!(q.is_empty());
    }

    #[test]
    fn search_state_tracks_hits() {
        let mut state = SearchState::new(200);
        state.set_keyword("abc");
        state.update_hits(5);
        assert_eq!(state.hit_count, 5);
        assert_eq!(state.query.keyword, "abc");
    }

    #[test]
    fn clear_resets_hits() {
        let mut state = SearchState::new(200);
        state.set_keyword("abc");
        state.update_hits(5);
        state.clear();
        assert_eq!(state.hit_count, 0);
        assert!(state.query.is_empty());
    }

    #[test]
    fn composing_flag() {
        let mut state = SearchState::new(200);
        assert!(!state.composing);
        state.composing = true;
        assert!(state.composing);
    }
}
