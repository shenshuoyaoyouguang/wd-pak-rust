//! 打开链路：文件对话框与包会话加载（G1-03.2）。

use std::mem::size_of;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

use super::util::wide;

/// 选择 PAK 文件；取消或无路径时返回 None。
pub unsafe fn pick(hwnd: HWND, initial: Option<&Path>) -> Option<PathBuf> {
    let initial = initial.map(|path| wide(&path.to_string_lossy()));
    let filter: Vec<u16> = "PAK 文件\0*.pak\0\0".encode_utf16().collect();
    let title = wide("打开 PAK");
    let mut file = [0u16; 1024];
    let mut ofn: OPENFILENAMEW = std::mem::zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = hwnd;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = file.as_mut_ptr();
    ofn.nMaxFile = file.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    if let Some(initial) = &initial {
        ofn.lpstrInitialDir = initial.as_ptr();
    }
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;

    if GetOpenFileNameW(&mut ofn) == 0 {
        return None;
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(PathBuf::from(String::from_utf16_lossy(&file[..len])))
}
