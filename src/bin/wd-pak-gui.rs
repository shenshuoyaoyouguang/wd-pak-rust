#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! GUI spike：原生窗口 + 五区布局 + 打开真实 PAK + 虚拟列表 + 选中详情。
//! 用于验证中文输入（S3）、大列表（S2）与辅助技术可读性（S4），
//! 并作为 G0/G1 外壳的第一个可运行切片。

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::path::PathBuf;
    use std::ptr::{null, null_mut};

    use wd_pak::browse::{default_visible, Column, Query, ViewMode};
    use wd_pak::error::PakError;
    use wd_pak::gui::layout::{
        layout, scale, LayoutInput, Rect, ShellRects, MIN_HEIGHT, MIN_WIDTH,
    };
    use wd_pak::session::{PakSession, SessionEntry};
    use windows_sys::Win32::Foundation::{
        GetLastError, GlobalFree, FILETIME, HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, EndPaint, FillRect, GetStockObject, GetSysColor, GetSysColorBrush,
        InvalidateRect, SetBkColor, SetTextColor, UpdateWindow, COLOR_BTNFACE, COLOR_WINDOW,
        COLOR_WINDOWTEXT, HDC, PAINTSTRUCT,
    };
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
    use windows_sys::Win32::System::Time::FileTimeToSystemTime;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows_sys::Win32::UI::Controls::{
        InitCommonControlsEx, EM_SETCUEBANNER, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX,
        LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVM_GETNEXTITEM,
        LVM_INSERTCOLUMNW, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMCOUNT, LVNI_SELECTED,
        LVN_GETDISPINFOW, LVN_ITEMCHANGED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
        LVS_OWNERDATA, LVS_REPORT, NMHDR, NMLVDISPINFOW,
    };
    use windows_sys::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
    use windows_sys::Win32::UI::HiDpi::{
        GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::Ime::{
        ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, GCS_COMPSTR,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetMessageW, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        IsWindow, KillTimer, LoadCursorW, MessageBoxW, PostQuitMessage, RegisterClassW,
        SendMessageW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos,
        SetWindowTextW, ShowWindow, TranslateMessage, BN_CLICKED, CS_HREDRAW, CS_VREDRAW,
        CW_USEDEFAULT, EN_CHANGE, ES_AUTOHSCROLL, ES_MULTILINE, ES_READONLY, GWLP_USERDATA,
        IDC_ARROW, MB_ICONERROR, MB_OK, MINMAXINFO, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE,
        SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED,
        WM_ERASEBKGND, WM_GETMINMAXINFO, WM_NCDESTROY, WM_NOTIFY, WM_PAINT, WM_SETFONT, WM_SIZE,
        WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE,
        WS_EX_DLGMODALFRAME, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
        WS_VSCROLL,
    };

    const OPEN_BUTTON_ID: usize = 1001;
    const DETAIL_BUTTON_ID: usize = 2001;
    const CLOSE_BUTTON_ID: usize = 2002;
    const COPY_BUTTON_ID: usize = 2003;
    const FILTER_TIMER_ID: usize = 1;
    const ERROR_CLASS: &str = "WdPakErrorDialog";

    struct Ui {
        edit: HWND,
        list: HWND,
        detail: HWND,
        status: HWND,
        open_button: HWND,
        columns: Vec<Column>,
        visible: Vec<usize>,
        session: Option<PakSession>,
        path: Option<PathBuf>,
        last_dir: Option<PathBuf>,
        text: Vec<u16>,
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn dpi_of(hwnd: HWND) -> u32 {
        let dpi = GetDpiForWindow(hwnd);
        if dpi == 0 {
            96
        } else {
            dpi
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_ERASEBKGND => 1,
            WM_CTLCOLORSTATIC => static_color(wparam),
            WM_SIZE => {
                layout_controls(hwnd);
                InvalidateRect(hwnd, null(), 1);
                0
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam as *mut MINMAXINFO);
                let dpi = dpi_of(hwnd);
                info.ptMinTrackSize = POINT {
                    x: scale(MIN_WIDTH, dpi),
                    y: scale(MIN_HEIGHT, dpi),
                };
                0
            }
            WM_DPICHANGED => {
                let suggested = &*(lparam as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                layout_controls(hwnd);
                InvalidateRect(hwnd, null(), 1);
                0
            }
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                if id == OPEN_BUTTON_ID as i32 && code == BN_CLICKED {
                    open_pak(hwnd);
                    return 0;
                }
                if code == EN_CHANGE {
                    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
                    if !pointer.is_null() && !is_composing((&*pointer).edit) {
                        // 规格 §3.3：停顿约 200 ms 再刷新，避免每次按键都过滤
                        SetTimer(hwnd, FILTER_TIMER_ID, 200, None);
                    }
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_NOTIFY => {
                let ui_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
                if !ui_ptr.is_null() {
                    let header = &*(lparam as *const NMHDR);
                    if header.code == LVN_GETDISPINFOW {
                        let info = &mut *(lparam as *mut NMLVDISPINFOW);
                        if info.item.mask & LVIF_TEXT != 0 && info.item.iItem >= 0 {
                            let ui = &mut *ui_ptr;
                            let value = {
                                let session = ui.session.as_ref();
                                let entry = ui
                                    .visible
                                    .get(info.item.iItem as usize)
                                    .and_then(|&index| session?.entries().get(index));
                                let column = ui
                                    .columns
                                    .get(info.item.iSubItem as usize)
                                    .copied()
                                    .unwrap_or(Column::Name);
                                entry
                                    .map(|entry| entry_text(entry, column))
                                    .unwrap_or_default()
                            };
                            ui.text.clear();
                            ui.text.extend(value.encode_utf16());
                            ui.text.push(0);
                            info.item.pszText = ui.text.as_mut_ptr();
                            info.item.cchTextMax = ui.text.len() as i32;
                        }
                        return 0;
                    }
                    if header.code == LVN_ITEMCHANGED {
                        refresh_texts(hwnd);
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_NCDESTROY => {
                let pointer = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Ui;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_TIMER => {
                if wparam == FILTER_TIMER_ID {
                    KillTimer(hwnd, FILTER_TIMER_ID);
                    apply_filter(hwnd);
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn static_color(wparam: WPARAM) -> LRESULT {
        let hdc = wparam as HDC;
        SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
        SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
        GetSysColorBrush(COLOR_WINDOW) as isize
    }

    unsafe fn is_composing(edit: HWND) -> bool {
        let context = ImmGetContext(edit);
        if context.is_null() {
            return false;
        }
        let length = ImmGetCompositionStringW(context, GCS_COMPSTR, null_mut(), 0);
        let _ = ImmReleaseContext(edit, context);
        length > 0
    }

    unsafe fn apply_filter(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &mut *pointer;
        let length = GetWindowTextLengthW(ui.edit).max(0);
        let mut buffer = vec![0u16; length as usize + 1];
        if length > 0 {
            GetWindowTextW(ui.edit, buffer.as_mut_ptr(), buffer.len() as i32);
        }
        let keyword = String::from_utf16_lossy(&buffer[..length as usize]);
        let query = Query {
            keyword,
            ..Default::default()
        };
        let visible = if let Some(session) = &ui.session {
            query.apply(session.entries())
        } else {
            Vec::new()
        };
        ui.visible = visible;
        SendMessageW(ui.list, LVM_SETITEMCOUNT, ui.visible.len(), 0);
        refresh_texts(hwnd);
        InvalidateRect(hwnd, null(), 1);
    }

    unsafe fn shell_layout(hwnd: HWND) -> (RECT, u32, ShellRects) {
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        let dpi = dpi_of(hwnd);
        let rects = layout(LayoutInput {
            width: (client.right * 96) / dpi as i32,
            height: (client.bottom * 96) / dpi as i32,
        });
        (client, dpi, rects)
    }

    unsafe fn layout_controls(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &*pointer;
        let (client, dpi, rects) = shell_layout(hwnd);

        SetWindowPos(
            ui.open_button,
            null_mut(),
            scale(8, dpi),
            scale(9, dpi),
            scale(110, dpi),
            scale(30, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let edit_w = scale(320, dpi);
        let edit_h = scale(26, dpi);
        SetWindowPos(
            ui.edit,
            null_mut(),
            client.right - edit_w - scale(8, dpi),
            scale(11, dpi),
            edit_w,
            edit_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let content = rects.content.scaled(dpi);
        SetWindowPos(
            ui.list,
            null_mut(),
            content.x,
            content.y,
            content.w,
            content.h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let detail = rects.detail.scaled(dpi);
        SetWindowPos(
            ui.detail,
            null_mut(),
            detail.x,
            detail.y,
            detail.w,
            detail.h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let status = rects.status_bar.scaled(dpi);
        SetWindowPos(
            ui.status,
            null_mut(),
            status.x,
            status.y,
            status.w,
            status.h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }

    unsafe fn refresh_texts(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &*pointer;
        let detail = wide(&detail_text(ui));
        SetWindowTextW(ui.detail, detail.as_ptr());
        let status = wide(&status_text(ui));
        SetWindowTextW(ui.status, status.as_ptr());
    }

    unsafe fn open_pak(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        let initial = if pointer.is_null() {
            None
        } else {
            (&*pointer)
                .last_dir
                .as_ref()
                .map(|path| wide(&path.to_string_lossy()))
        };

        let filter: Vec<u16> = "PAK 文件\0*.pak\0\0".encode_utf16().collect();
        let title = wide("打开 PAK");
        let mut file = [0u16; 1024];
        let mut ofn: OPENFILENAMEW = zeroed();
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
            return;
        }
        let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
        open_path(hwnd, PathBuf::from(String::from_utf16_lossy(&file[..len])));
    }

    unsafe fn open_path(hwnd: HWND, path: PathBuf) {
        match PakSession::open(&path) {
            Ok(session) => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
                if pointer.is_null() {
                    return;
                }
                {
                    let ui = &mut *pointer;
                    ui.session = Some(session);
                    ui.last_dir = path.parent().map(|parent| parent.to_path_buf());
                    ui.path = Some(path);
                    let _ = SetWindowTextW(ui.edit, wide("").as_ptr());
                }
                apply_filter(hwnd);

                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
                if pointer.is_null() {
                    return;
                }
                let ui = &*pointer;
                let name = ui
                    .path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let window_title = wide(&format!("wd-pak — {name}"));
                SetWindowTextW(hwnd, window_title.as_ptr());
                InvalidateRect(hwnd, null(), 1);
                SetFocus(ui.list);
            }
            Err(error) => show_error(hwnd, &error),
        }
    }

    /// 标题与建议逐条对应 `docs/windows-ui-plan.md` §4.7 的错误映射表；
    /// 不同错误不得合并成同一句话（硬约束 2）。
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

    unsafe fn show_error(hwnd: HWND, error: &PakError) {
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
            WS_EX_DLGMODALFRAME,
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

        let font = GetStockObject(windows_sys::Win32::Graphics::Gdi::DEFAULT_GUI_FONT);
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
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        EnableWindow(hwnd, 1);
        SetForegroundWindow(hwnd);
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
                        let height =
                            state.base_h + if state.expanded { state.details_h } else { 0 };
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

    unsafe fn place(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
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

    fn entry_text(entry: &SessionEntry, column: Column) -> String {
        match column {
            Column::Name => {
                if entry.is_dir() {
                    format!("{}/", entry.path)
                } else {
                    entry.path.clone()
                }
            }
            Column::Type => {
                if entry.is_dir() {
                    "目录".into()
                } else {
                    "文件".into()
                }
            }
            Column::Org if entry.is_dir() => "—".into(),
            Column::Org => entry.org.to_string(),
            Column::Size if entry.is_dir() => "—".into(),
            Column::Size => entry.size.to_string(),
            Column::Compression if entry.is_dir() => "—".into(),
            Column::Compression if entry.is_raw() => "原样".into(),
            Column::Compression => "LZSS".into(),
            Column::Attrib => format!("0x{:08x}", entry.attrib),
            Column::Offset => format!("0x{:08x}", entry.offset),
            Column::Mtime if entry.is_dir() => "—".into(),
            Column::Mtime if entry.mtime == 0 => "未设置".into(),
            Column::Mtime => mtime_local(entry.mtime).unwrap_or_else(|| entry.mtime.to_string()),
            Column::Ratio if entry.is_dir() || entry.org == 0 => "—".into(),
            Column::Ratio => format!("{:.3}", entry.size as f64 / entry.org as f64),
        }
    }

    /// Unix 秒 → 本地时间（规格 §2.4 要求 mtime 以本地时间呈现）。
    fn mtime_local(mtime: u32) -> Option<String> {
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
        let mut system: SYSTEMTIME = unsafe { zeroed() };
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

    fn mtime_text(mtime: u32) -> String {
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

    unsafe fn selected_entry(ui: &Ui) -> Option<&SessionEntry> {
        let selected = SendMessageW(
            ui.list,
            LVM_GETNEXTITEM,
            (-1isize) as usize,
            LVNI_SELECTED as isize,
        );
        if selected < 0 {
            return None;
        }
        let index = *ui.visible.get(selected as usize)?;
        ui.session.as_ref()?.entries().get(index)
    }

    fn detail_text(ui: &Ui) -> String {
        let Some(session) = &ui.session else {
            return "未打开包\n\n点击“打开 PAK”".into();
        };
        if let Some(entry) = unsafe { selected_entry(ui) } {
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

    fn short_middle(value: &str, limit: usize) -> String {
        let chars: Vec<char> = value.chars().collect();
        if chars.len() <= limit || limit < 8 {
            return value.into();
        }
        let head = limit / 2 - 1;
        let tail = limit - head - 1;
        format!(
            "{}…{}",
            chars[..head].iter().collect::<String>(),
            chars[chars.len() - tail..].iter().collect::<String>()
        )
    }

    fn status_text(ui: &Ui) -> String {
        let Some(path) = &ui.path else {
            return "未打开包".into();
        };
        let Some(session) = &ui.session else {
            return path.display().to_string();
        };
        let stats = session.stats();
        let selected = unsafe { selected_entry(ui) }
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| "未选中".into());
        format!(
            "{} | {} 个文件 / {} 个目录 | 存储 {} / 原始 {}（存储比 {:.3}） | {}",
            short_middle(&path.display().to_string(), 60),
            stats.files,
            stats.dirs,
            stats.total_stored,
            stats.total_org,
            stats.ratio,
            selected
        )
    }

    unsafe fn paint(hwnd: HWND) {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);

        let (_client, dpi, rects) = shell_layout(hwnd);
        zone(hdc, rects.command_bar, dpi, COLOR_BTNFACE);
        zone(hdc, rects.nav, dpi, COLOR_BTNFACE);
        zone(hdc, rects.tabs, dpi, COLOR_WINDOW);
        zone(hdc, rects.content, dpi, COLOR_WINDOW);
        zone(hdc, rects.detail, dpi, COLOR_WINDOW);
        zone(hdc, rects.task_panel, dpi, COLOR_BTNFACE);
        zone(hdc, rects.status_bar, dpi, COLOR_BTNFACE);

        EndPaint(hwnd, &ps);
    }

    unsafe fn zone(hdc: HDC, rect: Rect, dpi: u32, color: i32) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        let px = rect.scaled(dpi);
        let bounds = RECT {
            left: px.x,
            top: px.y,
            right: px.right(),
            bottom: px.bottom(),
        };
        FillRect(hdc, &bounds, GetSysColorBrush(color));
    }

    unsafe fn create_ui(hwnd: HWND) -> Option<Box<Ui>> {
        let instance = GetModuleHandleW(null());
        let open_button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("打开 PAK").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            0,
            0,
            0,
            0,
            hwnd,
            OPEN_BUTTON_ID as *mut c_void,
            instance,
            null(),
        );
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null(),
        );
        let list = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("SysListView32").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | LVS_REPORT | LVS_OWNERDATA,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null(),
        );
        let detail = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null(),
        );
        let status = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null(),
        );
        if open_button.is_null()
            || edit.is_null()
            || list.is_null()
            || detail.is_null()
            || status.is_null()
        {
            fatal("创建控件失败");
            return None;
        }

        let list_name = wide("包内路径列表");
        SetWindowTextW(list, list_name.as_ptr());

        let font = GetStockObject(windows_sys::Win32::Graphics::Gdi::DEFAULT_GUI_FONT);
        SendMessageW(edit, WM_SETFONT, font as usize, 1);
        SendMessageW(list, WM_SETFONT, font as usize, 1);
        SendMessageW(detail, WM_SETFONT, font as usize, 1);
        SendMessageW(status, WM_SETFONT, font as usize, 1);

        let cue = wide("搜索包内路径/文件名（不区分大小写）");
        SendMessageW(edit, EM_SETCUEBANNER, 1, cue.as_ptr() as isize);

        let dpi = dpi_of(hwnd);
        let columns: Vec<Column> = Column::ALL
            .into_iter()
            .filter(|column| default_visible(*column, ViewMode::List))
            .collect();
        for (index, column) in columns.iter().enumerate() {
            let mut definition: LVCOLUMNW = zeroed();
            definition.mask = LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM;
            definition.cx = scale(column.default_width(), dpi);
            definition.iSubItem = index as i32;
            let label = wide(column.title());
            definition.pszText = label.as_ptr() as *mut u16;
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                index,
                &definition as *const _ as isize,
            );
        }
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            0,
            (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
        );
        SendMessageW(list, LVM_SETITEMCOUNT, 0, 0);

        Some(Box::new(Ui {
            edit,
            list,
            detail,
            status,
            open_button,
            columns,
            visible: Vec::new(),
            session: None,
            path: None,
            last_dir: None,
            text: Vec::new(),
        }))
    }

    /// 启动期致命失败：带 Win32 错误码，便于用户复制反馈（§4.7 技术详情）。
    unsafe fn fatal(message: &str) {
        let code = GetLastError();
        let text = wide(&format!("{message}\n\nWin32 错误码: {code}"));
        let caption = wide("wd-pak");
        MessageBoxW(
            null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }

    pub fn run() {
        unsafe {
            let controls = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            let _ = InitCommonControlsEx(&controls);
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

            let instance = GetModuleHandleW(null());
            let class_name = wide("WdPakGuiSpike");

            let mut wc: WNDCLASSW = zeroed();
            wc.style = CS_HREDRAW | CS_VREDRAW;
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = instance;
            wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
            wc.lpszClassName = class_name.as_ptr();

            if RegisterClassW(&wc) == 0 {
                fatal("注册窗口类失败");
                return;
            }

            let title = wide("wd-pak");
            // §5.1：默认 1280×800 是 DIP，按系统 DPI 换算成物理像素
            let system_dpi = GetDpiForSystem();
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                scale(1280, system_dpi),
                scale(800, system_dpi),
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                fatal("创建窗口失败");
                return;
            }

            let Some(ui) = create_ui(hwnd) else {
                return;
            };
            let edit = ui.edit;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ui) as isize);
            layout_controls(hwnd);
            refresh_texts(hwnd);

            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
            SetFocus(edit);

            if let Some(path) = std::env::args_os().nth(1) {
                open_path(hwnd, PathBuf::from(path));
            }

            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("wd-pak-gui 目前仅支持 Windows");
}
