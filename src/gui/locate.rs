//! 深层定位状态模型（G2-06）。
//!
//! 只定义定位请求队列与高亮状态，不涉及 Win32 控件。

/// 定位来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocateSource {
    Search,
    Validation,
    Task,
}

/// 单个定位请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocateRequest {
    pub path: String,
    pub source: LocateSource,
    pub highlight: bool,
}

impl LocateRequest {
    pub fn new(path: &str, source: LocateSource) -> Option<Self> {
        let path = path.replace('\\', "/").trim_matches('/').to_string();
        if path.is_empty() {
            return None;
        }
        Some(Self {
            path,
            source,
            highlight: true,
        })
    }
}

/// 定位队列：去重、合并。
#[derive(Debug, Default, Clone)]
pub struct LocateQueue {
    pub requests: Vec<LocateRequest>,
}

impl LocateQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 推入一个定位请求（同路径去重，保留首个来源）。
    pub fn push(&mut self, request: LocateRequest) {
        if self.requests.iter().any(|r| r.path == request.path) {
            return;
        }
        self.requests.push(request);
    }

    /// 推入并返回是否新增。
    pub fn try_push(&mut self, request: LocateRequest) -> bool {
        if self.requests.iter().any(|r| r.path == request.path) {
            return false;
        }
        self.requests.push(request);
        true
    }

    /// 取出下一个定位请求。
    pub fn pop(&mut self) -> Option<LocateRequest> {
        if self.requests.is_empty() {
            return None;
        }
        Some(self.requests.remove(0))
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }

    pub fn clear(&mut self) {
        self.requests.clear();
    }
}

/// 高亮状态：定位后高亮 N 秒。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightState {
    pub path: String,
    pub remaining_ms: u32,
}

impl HighlightState {
    pub fn new(path: &str, duration_ms: u32) -> Self {
        Self {
            path: path.to_string(),
            remaining_ms: duration_ms,
        }
    }

    pub fn tick(&mut self, elapsed_ms: u32) {
        self.remaining_ms = self.remaining_ms.saturating_sub(elapsed_ms);
    }

    pub fn is_active(&self) -> bool {
        self.remaining_ms > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_normalizes_path() {
        let req = LocateRequest::new("\\sub\\a.txt", LocateSource::Search).unwrap();
        assert_eq!(req.path, "sub/a.txt");
        assert!(req.highlight);
    }

    #[test]
    fn request_rejects_empty() {
        assert!(LocateRequest::new("/", LocateSource::Task).is_none());
        assert!(LocateRequest::new("", LocateSource::Task).is_none());
    }

    #[test]
    fn queue_dedupes() {
        let mut q = LocateQueue::new();
        let r1 = LocateRequest::new("a.txt", LocateSource::Search).unwrap();
        let r2 = LocateRequest::new("a.txt", LocateSource::Task).unwrap();
        assert!(q.try_push(r1));
        assert!(!q.try_push(r2));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn queue_pop_fifo() {
        let mut q = LocateQueue::new();
        q.push(LocateRequest::new("a.txt", LocateSource::Search).unwrap());
        q.push(LocateRequest::new("b.txt", LocateSource::Task).unwrap());
        let first = q.pop().unwrap();
        assert_eq!(first.path, "a.txt");
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn highlight_ticks_down() {
        let mut h = HighlightState::new("x.txt", 2000);
        assert!(h.is_active());
        h.tick(500);
        assert!(h.is_active());
        assert_eq!(h.remaining_ms, 1500);
        h.tick(2000);
        assert!(!h.is_active());
        assert_eq!(h.remaining_ms, 0);
    }
}
