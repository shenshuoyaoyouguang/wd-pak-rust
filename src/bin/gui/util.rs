//! 面板共用的小工具：宽字符串、DPI、控件摆放与静态文本配色。

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{FILETIME, HWND, LRESULT, SYSTEMTIME, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    GetSysColor, GetSysColorBrush, SetBkColor, SetTextColor, COLOR_WINDOW, COLOR_WINDOWTEXT, HDC,
};
use windows_sys::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
use windows_sys::Win32::System::Time::FileTimeToSystemTime;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
};

pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub unsafe fn dpi_of(hwnd: HWND) -> u32 {
    let dpi = GetDpiForWindow(hwnd);
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

pub unsafe fn static_color(wparam: WPARAM) -> LRESULT {
    let hdc = wparam as HDC;
    SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
    SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
    GetSysColorBrush(COLOR_WINDOW) as isize
}

pub unsafe fn place(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    SetWindowPos(
        hwnd,
        null_mut(),
        x,
        y,
        width.max(0),
        height.max(0),
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

/// Unix 秒 → 本地时间（规格 §2.4 要求 mtime 以本地时间呈现）。
pub fn mtime_local(mtime: u32) -> Option<String> {
    // 1601-01-01 到 1970-01-01 的秒数；FILETIME 以 100ns 为单位
    let ticks = (mtime as u64 + 11_644_473_600) * 10_000_000;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut local = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut system: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        if FileTimeToLocalFileTime(&file_time, &mut local) == 0 {
            return None;
        }
        if FileTimeToSystemTime(&local, &mut system) == 0 {
            return None;
        }
    }
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        system.wYear, system.wMonth, system.wDay, system.wHour, system.wMinute, system.wSecond
    ))
}

/// 详情面板的双表示；0 与越界值保留原值并标注。
pub fn mtime_text(mtime: u32) -> String {
    if mtime == 0 {
        return "未设置（0）".into();
    }
    if mtime > 4_102_444_800 {
        return format!("异常值（{mtime}）");
    }
    match mtime_local(mtime) {
        Some(local) => format!("{local}（Unix 秒 {mtime}）"),
        None => format!("{mtime}（Unix 秒）"),
    }
}
