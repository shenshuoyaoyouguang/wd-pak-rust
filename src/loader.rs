//! 后台打开包（G1-03.3）：解析放在独立线程，界面轮询结果并可就地取消。
//!
//! 只负责一次加载；不缓存、不重试、不驱动窗口。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use crate::error::PakError;
use crate::session::PakSession;

/// 一次加载的最终结果；`poll` 只交付一次。
#[derive(Debug)]
pub enum LoadResult {
    Ready(Box<PakSession>),
    Failed(PakError),
    Cancelled,
}

enum Message {
    Progress(usize),
    Done(Result<PakSession, PakError>),
}

pub struct Loader {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Message>,
    progress: Option<usize>,
    done: bool,
}

impl Loader {
    pub fn start(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_path = path.clone();
        let thread_cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            if thread_cancel.load(Ordering::Relaxed) {
                return;
            }
            let progress_tx = tx.clone();
            let result = PakSession::open_with_progress(&thread_path, |count| {
                let _ = progress_tx.send(Message::Progress(count));
            });
            let _ = tx.send(Message::Done(result));
        });
        Self {
            path,
            cancel,
            rx,
            progress: None,
            done: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 最近一次回报的已读取条目数；尚未回报时为 None。
    pub fn progress(&self) -> Option<usize> {
        self.progress
    }

    /// 取消：即使解析已经完成，也以取消为准，结果被丢弃。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// 尚未完成时返回 None；完成后返回结果且只返回一次。
    pub fn poll(&mut self) -> Option<LoadResult> {
        if self.done {
            return None;
        }
        let cancelled = self.cancel.load(Ordering::Relaxed);
        loop {
            let result = match self.rx.try_recv() {
                Ok(Message::Progress(count)) => {
                    self.progress = Some(count);
                    continue;
                }
                Ok(Message::Done(_)) if cancelled => LoadResult::Cancelled,
                Ok(Message::Done(Ok(session))) => LoadResult::Ready(Box::new(session)),
                Ok(Message::Done(Err(error))) => LoadResult::Failed(error),
                Err(TryRecvError::Empty) if cancelled => LoadResult::Cancelled,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) if cancelled => LoadResult::Cancelled,
                Err(TryRecvError::Disconnected) => LoadResult::Failed(PakError::Io(
                    io::Error::other("加载线程已退出，未能读取目录表"),
                )),
            };
            self.done = true;
            return Some(result);
        }
    }
}
