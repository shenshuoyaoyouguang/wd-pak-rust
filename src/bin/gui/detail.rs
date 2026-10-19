//! 详情面板：基本信息、技术信息、attrib 位解释与空选中统计（G2-04）。

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SetWindowTextW, WS_CHILD, WS_VISIBLE,
};

use super::state::AppState;
use super::util::{mtime_text, place, wide};
use wd_pak::gui::layout::Rect;
use wd_pak::session::SessionEntry;

pub struct DetailPanel {
    hwnd: HWND,
}

impl DetailPanel {
    pub unsafe fn create(parent: HWND, instance: *mut c_void) -> Option<Self> {
        let hwnd = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            parent,
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        Some(Self { hwnd })
    }

    pub unsafe fn layout(&self, rect: Rect, dpi: u32) {
        let px = rect.scaled(dpi);
        place(self.hwnd, px.x, px.y, px.w, px.h);
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub unsafe fn refresh(&self, state: &AppState, entry: Option<&SessionEntry>) {
        let text = wide(&detail_text(state, entry));
        SetWindowTextW(self.hwnd, text.as_ptr());
    }
}

fn detail_text(state: &AppState, entry: Option<&SessionEntry>) -> String {
    // 无包与空包由 `notice` 统一承载，详情面板不再重复输出
    let Some(session) = &state.session else {
        return String::new();
    };
    if let Some(entry) = entry {
        let kind = if entry.is_dir() { "目录" } else { "文件" };
        let compression = if entry.is_dir() {
            "—"
        } else if entry.is_raw() {
            "原样 (RAW)"
        } else {
            "LZSS"
        };
        let ratio = if entry.is_dir() || entry.org == 0 {
            "—".into()
        } else {
            format!("{:.3}", entry.size as f64 / entry.org as f64)
        };
        return format!(
            "名称: {}\n包内路径: {}\n类型: {}\n原始大小: {} 字节\n存储大小: {} 字节\n压缩: {}\n存储比: {}\nattrib: {}\noffset: 0x{:08x}\nmtime: {}",
            entry.path,
            entry.path,
            kind,
            entry.org,
            entry.size,
            compression,
            ratio,
            attrib_text(entry.attrib),
            entry.offset,
            mtime_text(entry.mtime)
        );
    }

    let stats = session.stats();
    let distribution = stats
        .attribs
        .iter()
        .map(|(attrib, count)| format!("0x{attrib:08x}x{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "包统计\n文件: {}\n目录: {}\nrootDirSize: {}\nfileSize: {}\n存储/原始: {} / {}\n存储比: {:.3}\nattrib 分布: {}\n\n选择一个条目查看详情",
        stats.files,
        stats.dirs,
        stats.root_dir_size,
        stats.file_size,
        stats.total_stored,
        stats.total_org,
        stats.ratio,
        distribution
    )
}

fn attrib_text(attrib: u32) -> String {
    let mut parts = Vec::new();
    if attrib & 1 != 0 {
        parts.push("bit0=目录");
    }
    if attrib & 0x8000_0000 != 0 {
        parts.push("bit31=原样");
    }
    let rest = attrib & !1 & !0x8000_0000;
    if rest != 0 {
        parts.push("其余位按原值");
    }
    if parts.is_empty() {
        // attrib 为 0 不是「未定义」：bit31 未置位即按 LZSS 读取（硬约束 4）
        parts.push("bit0/bit31 均未置位，按 LZSS 压缩读取");
    }
    format!("0x{attrib:08x}（{}）", parts.join("，"))
}
