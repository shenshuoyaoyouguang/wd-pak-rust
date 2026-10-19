//! 打开失败的对话框：标题/原因/建议/技术详情/复制（G1-03.5、§4.7）。
//!
//! 标题与建议逐条对应 `docs/windows-ui-plan.md` §4.7 的错误映射表；
//! 不同错误不得合并成同一句话（硬约束 2）。

use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    GlobalFree, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetStockObject, COLOR_WINDOW, DEFAULT_GUI_FONT,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW,
    IsWindow, LoadCursorW, MessageBoxW, RegisterClassW, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, BN_CLICKED,
    CS_HREDRAW, CS_VREDRAW, ES_MULTILINE, ES_READONLY, GWLP_USERDATA, IDC_ARROW, MB_ICONERROR,
    MB_OK, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORSTATIC, WM_KEYDOWN, WM_NCDESTROY, WM_SIZE, WM_SETFONT, WNDCLASSW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use super::util::{dpi_of, place, static_color, wide};
use wd_pak::error::PakError;
use wd_pak::gui::layout::scale;

const DETAIL_BUTTON_ID: usize = 2001;
const CLOSE_BUTTON_ID: usize = 2002;
const COPY_BUTTON_ID: usize = 2003;
const ERROR_CLASS: &str = "WdPakErrorDialog";

pub unsafe fn show_error(hwnd: HWND, error: &PakError) {
    let title = error_title(error);
    let reason = error_reason(error);
    let suggestion = format!("建议: {}", error_suggestion(error));
    let details = error.to_string();

    let instance = GetModuleHandleW(null());
    let class_name = wide(ERROR_CLASS);
    let mut wc: WNDCLASSW = zeroed();
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = Some(error_wndproc);
    wc.hInstance = instance;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.hbrBackground = (COLOR_WINDOW + 1) as usize as *mut c_void;
    wc.lpszClassName = class_name.as_ptr();
    let _ = RegisterClassW(&wc);

    let dpi = dpi_of(hwnd);
    let width = scale(560, dpi);
    let height = scale(280, dpi);
    let mut owner_rect: RECT = zeroed();
    GetWindowRect(hwnd, &mut owner_rect);
    let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - width) / 2;
    let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - height) / 2;

    let window_title = wide("wd-pak");
    let dialog = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        class_name.as_ptr(),
        window_title.as_ptr(),
        WS_POPUP | WS_CAPTION | WS_SYSMENU,
        x,
        y,
        width,
        height,
        hwnd,
        null_mut(),
        instance,
        null(),
    );
    if dialog.is_null() {
        let fallback = wide(&format!("{title}\n\n{reason}\n\n{details}"));
        MessageBoxW(
            hwnd,
            fallback.as_ptr(),
            window_title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
        return;
    }

    let title_text = wide(title);
    let reason_text = wide(reason);
    let suggestion_text = wide(&suggestion);
    let details_text = wide(&details);
    let title_control = create_static(dialog, title_text.as_ptr(), instance);
    let reason_control = create_static(dialog, reason_text.as_ptr(), instance);
    let suggestion_control = create_static(dialog, suggestion_text.as_ptr(), instance);
    let details_control = CreateWindowExW(
        0,
        wide("EDIT").as_ptr(),
        details_text.as_ptr(),
        WS_CHILD | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32,
        0,
        0,
        0,
        0,
        dialog,
        null_mut(),
        instance,
        null(),
    );
    let toggle = CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide("技术详情").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        0,
        0,
        0,
        0,
        dialog,
        DETAIL_BUTTON_ID as *mut c_void,
        instance,
        null(),
    );
    let copy = CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide("复制详情").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        0,
        0,
        0,
        0,
        dialog,
        COPY_BUTTON_ID as *mut c_void,
        instance,
        null(),
    );
    let close = CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide("关闭").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        0,
        0,
        0,
        0,
        dialog,
        CLOSE_BUTTON_ID as *mut c_void,
        instance,
        null(),
    );
    if title_control.is_null()
        || reason_control.is_null()
        || suggestion_control.is_null()
        || details_control.is_null()
        || toggle.is_null()
        || copy.is_null()
        || close.is_null()
    {
        DestroyWindow(dialog);
        return;
    }

    let font = GetStockObject(DEFAULT_GUI_FONT);
    for control in [
        title_control,
        reason_control,
        suggestion_control,
        details_control,
        toggle,
        copy,
        close,
    ] {
        SendMessageW(control, WM_SETFONT, font as usize, 1);
    }

    let state = Box::new(ErrorState {
        title: title_control,
        reason: reason_control,
        suggestion: suggestion_control,
        details: details_control,
        toggle,
        copy,
        close,
        expanded: false,
        base_h: height,
        details_h: scale(180, dpi),
    });
    SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
    layout_error(dialog);

    EnableWindow(hwnd, 0);
    ShowWindow(dialog, SW_SHOW);
    SetFocus(close);

    let mut msg: MSG = zeroed();
    while IsWindow(dialog) != 0 {
        if GetMessageW(&mut msg, null_mut(), 0, 0) <= 0 {
            break;
        }
        // Tab 换焦点、方向键与默认按钮走对话框消息；Esc 由下面的 WM_KEYDOWN 处理
        if IsDialogMessageW(dialog, &mut msg) != 0 {
            continue;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    EnableWindow(hwnd, 1);
    SetForegroundWindow(hwnd);
}

/// 标题与建议逐条对应 `docs/windows-ui-plan.md` §4.7 的错误映射表。
fn error_title(error: &PakError) -> &'static str {
    match error {
        PakError::BadMagic(_) => "这不是有效的 PAK 文件",
        PakError::CatalogOob { .. }
        | PakError::CatalogBadSize { .. }
        | PakError::TreeTooDeep(_)
        | PakError::TreeCycle(_) => "包目录表损坏",
        PakError::NameTooLong(_) => "包内文件名过长",
        PakError::NameNonAscii(_) => "包内文件名含非 ASCII 字符",
        PakError::BadName(_) => "包内文件名非法",
        PakError::NameCaseConflict(..) => "目录中存在仅大小写不同的文件",
        PakError::Missing(_) | PakError::NotExists(_) => "包内找不到这个条目",
        _ => "无法打开这个 PAK",
    }
}

fn error_reason(error: &PakError) -> &'static str {
    match error {
        PakError::Truncated => "文件不足 16 字节，读不到 PAK 头部。原文件未被修改。",
        PakError::BadMagic(_) => "文件开头的魔数与 PAK 标识不符。原文件未被修改。",
        PakError::CatalogOob { .. }
        | PakError::CatalogBadSize { .. }
        | PakError::TreeTooDeep(_)
        | PakError::TreeCycle(_) => "包目录表无法解析，文件可能已损坏或不完整。原文件未被修改。",
        PakError::NameTooLong(_) | PakError::NameNonAscii(_) | PakError::BadName(_) => {
            "包内有不符合命名规则的条目。原文件未被修改。"
        }
        _ => "读取失败，原文件未被修改。",
    }
}

fn error_suggestion(error: &PakError) -> &'static str {
    match error {
        PakError::Truncated => "确认文件完整、未被截断",
        PakError::BadMagic(_) => "确认选择的是 PAK，而非压缩包或备份",
        PakError::CatalogOob { .. }
        | PakError::CatalogBadSize { .. }
        | PakError::TreeTooDeep(_)
        | PakError::TreeCycle(_) => "从备份恢复；不要用工具继续写入",
        PakError::NameTooLong(_) => "缩短名称后重试",
        PakError::NameNonAscii(_) => "改为 ASCII 名称后重试",
        PakError::NameCaseConflict(..) => "改名后重试；Windows 解包会互相覆盖",
        PakError::Missing(_) => "检查包内路径，或先在列表中定位",
        PakError::NotExists(_) => "先解包 → 放入文件 → 重新打包",
        _ => "重试一次；仍失败请复制技术详情",
    }
}

struct ErrorState {
    title: HWND,
    reason: HWND,
    suggestion: HWND,
    details: HWND,
    toggle: HWND,
    copy: HWND,
    close: HWND,
    expanded: bool,
    base_h: i32,
    details_h: i32,
}

unsafe fn create_static(parent: HWND, text: *const u16, instance: *mut c_void) -> HWND {
    CreateWindowExW(
        0,
        wide("STATIC").as_ptr(),
        text,
        WS_CHILD | WS_VISIBLE,
        0,
        0,
        0,
        0,
        parent,
        null_mut(),
        instance,
        null(),
    )
}

unsafe extern "system" fn error_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CTLCOLORSTATIC => static_color(wparam),
        WM_SIZE => {
            layout_error(hwnd);
            0
        }
        WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
            DestroyWindow(hwnd);
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            let code = ((wparam >> 16) & 0xffff) as u32;
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ErrorState;
            if !pointer.is_null() && code == BN_CLICKED {
                let state = &mut *pointer;
                if id == DETAIL_BUTTON_ID as i32 {
                    state.expanded = !state.expanded;
                    ShowWindow(
                        state.details,
                        if state.expanded { SW_SHOW } else { SW_HIDE },
                    );
                    let label = if state.expanded {
                        "收起详情"
                    } else {
                        "技术详情"
                    };
                    let text = wide(label);
                    SetWindowTextW(state.toggle, text.as_ptr());
                    let mut rect: RECT = zeroed();
                    GetWindowRect(hwnd, &mut rect);
                    let height = state.base_h + if state.expanded { state.details_h } else { 0 };
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    layout_error(hwnd);
                    return 0;
                }
                if id == COPY_BUTTON_ID as i32 {
                    let length = GetWindowTextLengthW(state.details);
                    let mut buffer = vec![0u16; length as usize + 1];
                    GetWindowTextW(state.details, buffer.as_mut_ptr(), buffer.len() as i32);
                    copy_text(hwnd, &String::from_utf16_lossy(&buffer[..length as usize]));
                    return 0;
                }
                if id == CLOSE_BUTTON_ID as i32 {
                    DestroyWindow(hwnd);
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_NCDESTROY => {
            let pointer = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut ErrorState;
            if !pointer.is_null() {
                drop(Box::from_raw(pointer));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn layout_error(hwnd: HWND) {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ErrorState;
    if pointer.is_null() {
        return;
    }
    let state = &*pointer;
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let dpi = dpi_of(hwnd);
    let margin = scale(16, dpi);
    let width = client.right - margin * 2;
    let button_w = scale(120, dpi);
    let button_h = scale(30, dpi);
    let footer_y = client.bottom - margin - button_h;

    place(state.title, margin, margin, width, scale(40, dpi));
    place(state.reason, margin, scale(64, dpi), width, scale(40, dpi));
    place(state.suggestion, margin, scale(104, dpi), width, scale(40, dpi));
    place(
        state.details,
        margin,
        scale(152, dpi),
        width,
        footer_y - scale(152, dpi) - margin,
    );
    place(state.toggle, margin, footer_y, button_w, button_h);
    place(
        state.copy,
        margin + button_w + scale(8, dpi),
        footer_y,
        button_w,
        button_h,
    );
    place(
        state.close,
        client.right - margin - button_w,
        footer_y,
        button_w,
        button_h,
    );
}

/// 错误对话框的「复制详情」（§4.7 次操作）。
unsafe fn copy_text(owner: HWND, text: &str) {
    let data = wide(text);
    if OpenClipboard(owner) == 0 {
        return;
    }
    EmptyClipboard();
    let bytes = data.len() * 2;
    let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
    if !handle.is_null() {
        let dest = GlobalLock(handle);
        if !dest.is_null() {
            std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), dest.cast::<u8>(), bytes);
            GlobalUnlock(handle);
            if SetClipboardData(CF_UNICODETEXT as u32, handle).is_null() {
                GlobalFree(handle);
            }
        }
    }
    CloseClipboard();
}
