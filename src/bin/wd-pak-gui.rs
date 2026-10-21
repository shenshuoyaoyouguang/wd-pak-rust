#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! GUI spike：原生窗口 + 五区布局 + 打开真实 PAK + 虚拟列表 + 选中详情。
//! 用于验证中文输入（S3）、大列表（S2）与辅助技术可读性（S4），
//! 并作为 G0/G1 外壳的第一个可运行切片。

#[cfg(windows)]
mod win {
    use std::collections::HashSet;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Instant;

    use wd_pak::browse::{default_visible, sort_indices, Column, Query, SortDir, SortSpec, ViewMode};
    use wd_pak::commands_write::{cmd_pack, cmd_replace};
    use wd_pak::config::Config;
    use wd_pak::diagnostics::{APP_VERSION, ErrorLog};
    use wd_pak::error::PakError;
    use wd_pak::gui::error_text::{error_reason, error_suggestion, error_title};
    use wd_pak::gui::export::{
        default_export_name, looks_binary, preview_for, preview_hex, preview_truncate,
        unique_target, ExportConflict, ExportPlan, PreviewKind, HEX_PREVIEW_LIMIT, PREVIEW_LIMIT,
    };
    use wd_pak::gui::pack::{
        clamp_jobs, default_jobs, manifest_mtime_issues, pack_precheck, scan_dir, PackIssue,
        PackPrecheckResult, PackScan,
    };
    use wd_pak::gui::replace::{backup_path, replace_precheck, ReplaceIssueKind};
    use wd_pak::gui::unpack::{
        unpack_precheck_full, UnpackIssue, UnpackIssueKind, UnpackPrecheckResult,
    };
    use wd_pak::pak::PakReader;
    use wd_pak::gui::layout::{
        layout, scale, LayoutInput, Rect, ShellRects, MIN_HEIGHT, MIN_WIDTH,
    };
    use wd_pak::gui::locate::HighlightState;
    use wd_pak::gui::persist::{self, SessionSnapshot, TabSnapshot};
    use wd_pak::gui::text::{detail_text_multi, entry_text, status_text_multi, tree_entry_text};
    use wd_pak::loader::{LoadResult, Loader};
    use wd_pak::manifest::{FileStat, Manifest, MANIFEST_NAME};
    use wd_pak::session::{PakSession, SessionEntry};
    use wd_pak::workspace::{OpenRequest, OpenSource, Workspace};
    use windows_sys::Win32::Foundation::{
        GetLastError, GlobalFree, FILETIME, HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, CreateFontW, DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject,
        GetSysColor, GetSysColorBrush, InvalidateRect, SelectObject, SetBkColor, SetBkMode,
        SetTextColor, UpdateWindow, COLOR_BTNFACE, COLOR_HIGHLIGHT, COLOR_WINDOW,
        COLOR_WINDOWTEXT, DEFAULT_GUI_FONT, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
        DT_SINGLELINE, DT_VCENTER, HDC, HFONT, PAINTSTRUCT, TRANSPARENT,
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
        GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT,
        OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows_sys::Win32::UI::Controls::{
        InitCommonControlsEx, EM_LIMITTEXT, EM_SETCUEBANNER, ICC_LISTVIEW_CLASSES,
        ICC_PROGRESS_CLASS,
        INITCOMMONCONTROLSEX, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT,
        LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETECOLUMN, LVM_ENSUREVISIBLE,
        LVM_GETCOLUMNWIDTH, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_SETCOLUMNWIDTH,
        LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMCOUNT, LVM_SETITEMSTATE, LVNI_SELECTED,
        LVN_COLUMNCLICK, LVN_GETDISPINFOW, LVN_ITEMCHANGED, LVN_KEYDOWN, LVS_EX_DOUBLEBUFFER,
        LVS_EX_FULLROWSELECT, LVS_OWNERDATA, LVS_REPORT, NMHDR, NMLISTVIEW, NMLVCUSTOMDRAW,
        NMLVDISPINFOW, NMLVKEYDOWN, NMITEMACTIVATE, NM_CUSTOMDRAW, NM_DBLCLK, NM_RETURN,
        CDDS_ITEMPREPAINT,
        CDDS_PREPAINT, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW, PBM_SETMARQUEE, PBM_SETPOS,
        PBM_SETRANGE32, PBS_MARQUEE, BST_CHECKED,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FileTimeToLocalFileTime, GetDiskFreeSpaceExW,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteW, BROWSEINFOW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, ILFree,
        SHBrowseForFolderW, SHGetPathFromIDListW,
    };
    use windows_sys::Win32::UI::HiDpi::{
        GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::Ime::{
        ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, GCS_COMPSTR,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetMessageW, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        IsWindow, IsZoomed, KillTimer, LoadCursorW, MessageBoxW, PostQuitMessage, RegisterClassW,
        SendMessageW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos,
        SetWindowTextW, ShowWindow, TranslateMessage, ACCEL, BN_CLICKED, BS_AUTOCHECKBOX,
        BS_AUTORADIOBUTTON, BS_DEFPUSHBUTTON,
        BM_GETCHECK, BM_SETCHECK, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBS_DROPDOWNLIST,
        CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, EN_CHANGE, ES_AUTOHSCROLL,
        ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, FCONTROL, FSHIFT, FVIRTKEY, GWLP_USERDATA,
        IDCANCEL, IDC_ARROW, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING,
        MB_YESNO, MB_OK, MB_DEFBUTTON2, MINMAXINFO, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE,
        SW_MAXIMIZE,
        SW_SHOW, SW_SHOWNORMAL, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED,
        WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_NCDESTROY,
        WM_NOTIFY,
        WM_PAINT, WM_SETFONT, WM_SIZE, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD,
        WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_GROUP, WS_HSCROLL,
        WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
        WNDPROC,
    };

    const OPEN_BUTTON_ID: usize = 1001;
    const EXPORT_BUTTON_ID: usize = 1002;
    const DETAIL_BUTTON_ID: usize = 2001;
    const CLOSE_BUTTON_ID: usize = 2002;
    const COPY_BUTTON_ID: usize = 2003;
    const CANCEL_BUTTON_ID: usize = 2004;
    const EXPORT_DIR_ID: usize = 3001;
    const EXPORT_NAME_ID: usize = 3002;
    const EXPORT_BROWSE_ID: usize = 3003;
    const EXPORT_STRUCTURE_ID: usize = 3004;
    const EXPORT_OPEN_ID: usize = 3005;
    const EXPORT_OK_ID: usize = 3006;
    const EXPORT_CANCEL_ID: usize = 3007;
    const EXPORT_CONFLICT_ID: usize = 3008;
    const PREVIEW_TEXT_ID: usize = 3101;
    const PREVIEW_HEX_ID: usize = 3102;
    const PREVIEW_EXPORT_ID: usize = 3103;
    const PREVIEW_CLOSE_ID: usize = 3104;
    const CONFLICT_REPLACE_ID: usize = 3201;
    const CONFLICT_SKIP_ID: usize = 3202;
    const CONFLICT_RENAME_ID: usize = 3203;
    const CONFLICT_ALL_ID: usize = 3204;
    const CONFLICT_CANCEL_ID: usize = 3205;
    const REPLACE_FILE_ID: usize = 4001;
    const REPLACE_BROWSE_ID: usize = 4002;
    const REPLACE_COMPRESS_ID: usize = 4003;
    const REPLACE_RAW_ID: usize = 4004;
    const REPLACE_BACKUP_ID: usize = 4005;
    const REPLACE_OK_ID: usize = 4006;
    const REPLACE_CANCEL_ID: usize = 4007;
    const PACK_SOURCE_ID: usize = 4101;
    const PACK_SOURCE_BROWSE_ID: usize = 4102;
    const PACK_OUTPUT_ID: usize = 4103;
    const PACK_OUTPUT_BROWSE_ID: usize = 4104;
    const PACK_COMPRESS_ID: usize = 4105;
    const PACK_JOBS_ID: usize = 4106;
    const PACK_CHECK_ID: usize = 4107;
    const PACK_START_ID: usize = 4108;
    const PACK_CLOSE_ID: usize = 4109;
    const UNPACK_DIR_ID: usize = 4201;
    const UNPACK_BROWSE_ID: usize = 4202;
    const UNPACK_OPEN_ID: usize = 4203;
    const UNPACK_CHECK_ID: usize = 4205;
    const UNPACK_START_ID: usize = 4206;
    const UNPACK_CLOSE_ID: usize = 4207;
    const FILTER_TIMER_ID: usize = 1;
    const HIGHLIGHT_TIMER_ID: usize = 3;
    const EXPORT_PROGRESS_TIMER_ID: usize = 4;
    const WRITE_TIMER_ID: usize = 5;
    const ERROR_CLASS: &str = "WdPakErrorDialog";
    const EXPORT_CLASS: &str = "WdPakExportDialog";
    const PREVIEW_CLASS: &str = "WdPakPreviewDialog";
    const CONFLICT_CLASS: &str = "WdPakConflictDialog";
    const EXPORT_PROGRESS_CLASS: &str = "WdPakExportProgress";
    const REPLACE_CLASS: &str = "WdPakReplaceDialog";
    const PACK_CLASS: &str = "WdPakPackDialog";
    const UNPACK_CLASS: &str = "WdPakUnpackDialog";
    const WRITE_PROGRESS_CLASS: &str = "WdPakWriteProgress";
    /// 预览读取上限：超过该大小的条目整包解压代价过高，直接提示改为导出。
    const PREVIEW_READ_GUARD: u64 = 64 * 1024 * 1024;

    type HMENU = *mut c_void;

    #[link(name = "user32")]
    extern "system" {
        fn CreateMenu() -> HMENU;
        fn CreatePopupMenu() -> HMENU;
        fn AppendMenuW(hMenu: HMENU, uFlags: u32, uIDNewItem: usize, lpNewItem: *const u16) -> i32;
        fn SetMenu(hWnd: HWND, hMenu: HMENU) -> i32;
        fn DrawMenuBar(hWnd: HWND) -> i32;
        fn GetCursorPos(lpPoint: *mut POINT) -> i32;
        fn TrackPopupMenu(hMenu: HMENU, uFlags: u32, x: i32, y: i32, nReserved: i32, hWnd: HWND, lprcRect: *const RECT) -> i32;
        fn GetMenu(hWnd: HWND) -> HMENU;
        fn GetSubMenu(hMenu: HMENU, nPos: i32) -> HMENU;
        fn GetMenuItemCount(hMenu: HMENU) -> i32;
        fn DeleteMenu(hMenu: HMENU, uPosition: u32, uFlags: u32) -> i32;
        fn EnableMenuItem(hMenu: HMENU, uIDEnableItem: u32, uEnable: u32) -> i32;
        fn CheckMenuItem(hMenu: HMENU, uIDCheckItem: u32, uCheck: u32) -> i32;
        fn DestroyMenu(hMenu: HMENU) -> i32;
        fn CreateAcceleratorTableW(paccel: *const ACCEL, caccel: i32) -> *mut c_void;
        fn DestroyAcceleratorTable(haccel: *mut c_void) -> i32;
        fn TranslateAcceleratorW(hwnd: HWND, haccel: *mut c_void, msg: *const MSG) -> i32;
        fn IsDialogMessageW(hwnd: HWND, msg: *const MSG) -> i32;
    }

    const MF_STRING: u32 = 0x0000;
    const MF_POPUP: u32 = 0x0010;
    const MF_SEPARATOR: u32 = 0x0800;
    const MF_ENABLED: u32 = 0x0000;
    const MF_GRAYED: u32 = 0x0001;
    const MF_CHECKED: u32 = 0x0008;
    const MF_BYCOMMAND: u32 = 0x0000;
    const MF_BYPOSITION: u32 = 0x0400;

    const MENU_OPEN: usize = 9001;
    const MENU_CLOSE: usize = 9002;
    const MENU_EXIT: usize = 9003;
    const MENU_ABOUT: usize = 9004;
    const MENU_COPY_PATH: usize = 9005;
    const MENU_FIND: usize = 9006;
    const MENU_REFRESH: usize = 9007;
    const MENU_CLEAR: usize = 9008;
    const MENU_EXPORT: usize = 9009;
    const MENU_PREVIEW: usize = 9010;
    const MENU_REPLACE: usize = 9011;
    const MENU_PACK: usize = 9012;
    const MENU_UNPACK: usize = 9013;
    const MENU_RECENT_BASE: usize = 9100;
    const MENU_RECENT_MAX: usize = 20;
    const MENU_RECENT_CLEAR: usize = 9120;
    const MENU_NEXT_TAB: usize = 9200;
    const MENU_TAB_BASE: usize = 9201;
    const MENU_VIEW_TREE: usize = 9300;
    const MENU_VIEW_LIST: usize = 9301;
    const MENU_COLUMN_BASE: usize = 9310;
    const MENU_COLUMNS_RESET: usize = 9320;
    const MENU_LOCATE: usize = 9330;
    const TAB_MIN_DIP: i32 = 96;
    const TAB_MAX_DIP: i32 = 200;
    const TAB_CLOSE_DIP: i32 = 22;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const TPM_LEFTALIGN: u32 = 0x0000;
    const TPM_RETURNCMD: u32 = 0x0100;
    const SS_CENTER: u32 = 0x0000_0001;
    const SS_NOPREFIX: u32 = 0x0000_0080;
    const PROGRESS_CLASS: &str = "msctls_progress32";
    const PROGRESS_CLASS_NAME: &str = "WdPakLoadProgress";
    const PROGRESS_TIMER_ID: usize = 2;
    const HIT_TEXT_W: i32 = 72;

    /// 非活动标签的会话与最后选中项；活动标签的会话放在 `Ui::session`。
    struct TabRuntime {
        session: Option<PakSession>,
        selected: Option<String>,
    }

    struct Ui {
        edit: HWND,
        list: HWND,
        detail: HWND,
        status: HWND,
        empty: HWND,
        hits: HWND,
        open_button: HWND,
        export_button: HWND,
        columns: Vec<Column>,
        column_mode: ViewMode,
        column_tab: Option<usize>,
        visible: Vec<usize>,
        depths: Vec<usize>,
        highlight: Option<HighlightState>,
        session: Option<PakSession>,
        path: Option<PathBuf>,
        last_dir: Option<PathBuf>,
        filter: String,
        detail_hidden: bool,
        font: HFONT,
        text: Vec<u16>,
        workspace: Workspace,
        runtime: Vec<TabRuntime>,
        config: Config,
        config_path: PathBuf,
        session_path: PathBuf,
        recent_menu: HMENU,
        config_read_only: bool,
        error_log: ErrorLog,
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 给控件设置文本；Win32 在调用内复制字符串，临时缓冲可直接借用。
    unsafe fn set_text(hwnd: HWND, text: &str) {
        SetWindowTextW(hwnd, wide(text).as_ptr());
    }

    /// 追加菜单项（标签临时转 UTF-16，AppendMenuW 调用内复制）。
    unsafe fn append_menu(menu: HMENU, flags: u32, id: usize, label: &str) {
        AppendMenuW(menu, flags, id, wide(label).as_ptr());
    }

    unsafe fn append_separator(menu: HMENU) {
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
    }

    /// 读取控件文本；空控件返回空串。
    unsafe fn read_text(control: HWND) -> String {
        let length = GetWindowTextLengthW(control);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }

    /// 控件全部创建成功才继续；任一失败由调用方销毁对话框回退。
    fn all_present(controls: &[HWND]) -> bool {
        controls.iter().all(|control| !control.is_null())
    }

    /// 套用系统默认 UI 字体（各向导/对话框统一字号）。
    unsafe fn apply_default_font(controls: &[HWND]) {
        let font = GetStockObject(DEFAULT_GUI_FONT);
        for control in controls {
            SendMessageW(*control, WM_SETFONT, font as usize, 1);
        }
    }

    /// 创建子控件；`id` 为 0 时不挂控件 id。
    unsafe fn create_control(
        parent: HWND,
        instance: *mut c_void,
        class: &str,
        text: *const u16,
        style: u32,
        ex_style: u32,
        id: usize,
    ) -> HWND {
        let class = wide(class);
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            text,
            style,
            0,
            0,
            0,
            0,
            parent,
            id as *mut c_void,
            instance,
            null(),
        )
    }

    /// 静态文本。
    unsafe fn create_label(parent: HWND, instance: *mut c_void, text: &str) -> HWND {
        let text = wide(text);
        create_control(
            parent,
            instance,
            "STATIC",
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
        )
    }

    /// 命令按钮；`style` 只补充按钮特有样式（BS_*、WS_GROUP 等）。
    unsafe fn create_button(
        parent: HWND,
        instance: *mut c_void,
        text: &str,
        style: u32,
        id: usize,
    ) -> HWND {
        let text = wide(text);
        create_control(
            parent,
            instance,
            "BUTTON",
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
            0,
            id,
        )
    }

    /// 单行输入框（带边框与横向滚动）；`text` 为 None 时不设初始文本。
    unsafe fn create_text_field(
        parent: HWND,
        instance: *mut c_void,
        text: Option<&str>,
        id: usize,
    ) -> HWND {
        let text = text.map(wide);
        create_control(
            parent,
            instance,
            "EDIT",
            text.as_ref().map_or(null(), |value| value.as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            WS_EX_CLIENTEDGE,
            id,
        )
    }

    /// 只读多行文本区（预检问题列表）。
    unsafe fn create_text_area(parent: HWND, instance: *mut c_void) -> HWND {
        create_control(
            parent,
            instance,
            "EDIT",
            null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32
                | WS_VSCROLL,
            WS_EX_CLIENTEDGE,
            0,
        )
    }

    /// 进度条；`style` 追加 PBS_MARQUEE 等。
    unsafe fn create_progress(parent: HWND, instance: *mut c_void, style: u32) -> HWND {
        create_control(
            parent,
            instance,
            PROGRESS_CLASS,
            null(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
        )
    }

    /// 注册对话框窗口类，返回宽字符类名与模块句柄供 CreateWindowExW 使用。
    unsafe fn register_dialog_class(
        name: &str,
        proc: WNDPROC,
        background: i32,
    ) -> (Vec<u16>, *mut c_void) {
        let instance = GetModuleHandleW(null());
        let class_name = wide(name);
        let mut wc: WNDCLASSW = zeroed();
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = proc;
        wc.hInstance = instance;
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        wc.hbrBackground = (background + 1) as usize as *mut c_void;
        wc.lpszClassName = class_name.as_ptr();
        let _ = RegisterClassW(&wc);
        (class_name, instance)
    }

    /// 在 owner 居中创建模态对话框；宽高已是物理像素。
    unsafe fn create_centered_dialog(
        owner: HWND,
        class_name: &[u16],
        instance: *mut c_void,
        title: &str,
        ex_style: u32,
        width: i32,
        height: i32,
    ) -> HWND {
        let mut owner_rect: RECT = zeroed();
        GetWindowRect(owner, &mut owner_rect);
        let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - width) / 2;
        let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - height) / 2;
        let title = wide(title);
        CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            x,
            y,
            width,
            height,
            owner,
            null_mut(),
            instance,
            null(),
        )
    }

    /// 对话框模态消息循环；销毁后恢复 owner 的交互并带回前台。
    unsafe fn run_modal(dialog: HWND, owner: HWND, focus: HWND) {
        EnableWindow(owner, 0);
        ShowWindow(dialog, SW_SHOW);
        SetFocus(focus);
        let mut msg: MSG = zeroed();
        while IsWindow(dialog) != 0 {
            if GetMessageW(&mut msg, null_mut(), 0, 0) <= 0 {
                break;
            }
            if IsDialogMessageW(dialog, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        EnableWindow(owner, 1);
        SetForegroundWindow(owner);
    }

    /// 客户区像素矩形与窗口 DPI。
    unsafe fn client_metrics(hwnd: HWND) -> (RECT, u32) {
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        (client, dpi_of(hwnd))
    }

    unsafe fn dpi_of(hwnd: HWND) -> u32 {
        let dpi = GetDpiForWindow(hwnd);
        if dpi == 0 {
            96
        } else {
            dpi
        }
    }

    unsafe fn create_ui_font(dpi: u32) -> HFONT {
        let face = wide("Segoe UI");
        CreateFontW(
            -scale(12, dpi),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            face.as_ptr(),
        )
    }

    unsafe fn rebuild_font(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let font = create_ui_font(dpi_of(hwnd));
        if font.is_null() {
            return;
        }
        let ui = &mut *pointer;
        for control in [
            ui.open_button,
            ui.export_button,
            ui.edit,
            ui.list,
            ui.detail,
            ui.status,
            ui.empty,
            ui.hits,
        ] {
            SendMessageW(control, WM_SETFONT, font as usize, 1);
        }
        if !ui.font.is_null() {
            DeleteObject(ui.font);
        }
        ui.font = font;
        apply_column_widths(hwnd);
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
            WM_CTLCOLORSTATIC => static_color(hwnd, wparam, lparam),
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
                rebuild_font(hwnd);
                layout_controls(hwnd);
                InvalidateRect(hwnd, null(), 1);
                0
            }
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                if code == BN_CLICKED {
                    match id as usize {
                        OPEN_BUTTON_ID => {
                            open_pak(hwnd);
                            return 0;
                        }
                        EXPORT_BUTTON_ID => {
                            export_selected(hwnd);
                            return 0;
                        }
                        _ => {}
                    }
                }
                // 菜单命令 code=0；加速键命令 code=1（BN_CLICKED 也是 0，故按 id 区分）。
                if lparam == 0 && (code == 0 || code == 1) {
                    match id as usize {
                        MENU_OPEN => { open_pak(hwnd); return 0; }
                        MENU_CLOSE => { close_pak(hwnd); return 0; }
                        MENU_FIND => {
                            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
                            if !pointer.is_null() {
                                SetFocus((*pointer).edit);
                            }
                            return 0;
                        }
                        MENU_REFRESH => { reload_pak(hwnd); return 0; }
                        MENU_EXPORT => { export_selected(hwnd); return 0; }
                        MENU_PREVIEW => { preview_selected(hwnd); return 0; }
                        MENU_REPLACE => { replace_selected(hwnd); return 0; }
                        MENU_PACK => { open_pack_dialog(hwnd); return 0; }
                        MENU_UNPACK => { open_unpack_dialog(hwnd); return 0; }
                        MENU_CLEAR => { handle_escape(hwnd); return 0; }
                        MENU_COPY_PATH => {
                            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
                            if !pointer.is_null() {
                                let paths = selected_paths(&*pointer);
                                if !paths.is_empty() {
                                    copy_text(hwnd, &paths.join("\r\n"));
                                }
                            }
                            return 0;
                        }
                        MENU_NEXT_TAB => { cycle_tab(hwnd); return 0; }
                        MENU_VIEW_TREE => { set_view_mode(hwnd, ViewMode::Tree); return 0; }
                        MENU_VIEW_LIST => { set_view_mode(hwnd, ViewMode::List); return 0; }
                        MENU_COLUMNS_RESET => { reset_columns(hwnd); return 0; }
                        MENU_LOCATE => { locate_first(hwnd); return 0; }
                        command
                            if (MENU_COLUMN_BASE
                                ..MENU_COLUMN_BASE + Column::ALL.len())
                                .contains(&command) =>
                        {
                            toggle_column(hwnd, command - MENU_COLUMN_BASE);
                            return 0;
                        }
                        MENU_RECENT_CLEAR => { clear_recent(hwnd); return 0; }
                        command if (MENU_RECENT_BASE..MENU_RECENT_BASE + MENU_RECENT_MAX)
                            .contains(&command) =>
                        {
                            open_recent(hwnd, command - MENU_RECENT_BASE);
                            return 0;
                        }
                        command if (MENU_TAB_BASE..MENU_TAB_BASE + 9).contains(&command) => {
                            activate_tab(hwnd, command - MENU_TAB_BASE);
                            return 0;
                        }
                        MENU_EXIT => { save_state(hwnd); DestroyWindow(hwnd); return 0; }
                        MENU_ABOUT => {
                            let text = wide(&format!("wd-pak {APP_VERSION}\n问道 PAK 工具"));
                            let caption = wide("关于");
                            MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_OK);
                            return 0;
                        }
                        _ => {}
                    }
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
                                let depth = ui
                                    .depths
                                    .get(info.item.iItem as usize)
                                    .copied()
                                    .unwrap_or(0);
                                entry
                                    .map(|entry| {
                                        if ui.column_mode == ViewMode::Tree {
                                            tree_entry_text(entry, depth, column, mtime_local)
                                        } else {
                                            entry_text(entry, column, mtime_local)
                                        }
                                    })
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
                        let ui = &mut *ui_ptr;
                        if ui.detail_hidden && selected_entry(ui).is_some() {
                            ui.detail_hidden = false;
                            layout_controls(hwnd);
                        }
                        refresh_texts(hwnd);
                        update_menu_state(hwnd);
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                    if header.code == LVN_COLUMNCLICK {
                        let info = &*(lparam as *const NMLISTVIEW);
                        let mut changed = false;
                        {
                            let ui = &mut *ui_ptr;
                            if let Some(column) =
                                ui.columns.get(info.iSubItem as usize).copied()
                            {
                                if column.sortable() {
                                    if let Some(index) = ui.workspace.active_index() {
                                        if let Some(tab) = ui.workspace.tab_mut(index) {
                                            let current = tab.view.sort;
                                            tab.view.sort = if current.key == column {
                                                SortSpec {
                                                    key: column,
                                                    dir: if current.dir == SortDir::Asc {
                                                        SortDir::Desc
                                                    } else {
                                                        SortDir::Asc
                                                    },
                                                }
                                            } else {
                                                SortSpec {
                                                    key: column,
                                                    dir: SortDir::Asc,
                                                }
                                            };
                                            changed = true;
                                        }
                                    }
                                }
                            }
                        }
                        if changed {
                            sync_columns(hwnd, true, true);
                            apply_filter(hwnd);
                        }
                        return 0;
                    }
                    if header.code == NM_DBLCLK {
                        let info = &*(lparam as *const NMITEMACTIVATE);
                        let activation = if info.iItem >= 0 {
                            let ui = &*ui_ptr;
                            ui.visible
                                .get(info.iItem as usize)
                                .and_then(|&index| ui.session.as_ref()?.entries().get(index))
                                .map(|entry| (entry.path.clone(), entry.is_dir()))
                        } else {
                            None
                        };
                        if let Some((path, is_dir)) = activation {
                            if is_dir {
                                if active_view_mode(&*ui_ptr) == ViewMode::Tree {
                                    if let Some(index) = (*ui_ptr).workspace.active_index() {
                                        if let Some(tab) = (*ui_ptr).workspace.tab_mut(index) {
                                            tab.view.tree.toggle(&path);
                                        }
                                    }
                                    apply_filter(hwnd);
                                }
                            } else {
                                preview_path(hwnd, hwnd, &path);
                                SetFocus((*ui_ptr).list);
                            }
                        }
                        return 0;
                    }
                    if header.code == NM_RETURN {
                        preview_selected(hwnd);
                        return 0;
                    }
                    if header.code == LVN_KEYDOWN {
                        let info = &*(lparam as *const NMLVKEYDOWN);
                        if info.wVKey == 0x20 || info.wVKey == 0x0D {
                            preview_selected(hwnd);
                            return 0;
                        }
                        return 0;
                    }
                    if header.code == NM_CUSTOMDRAW {
                        let draw = &mut *(lparam as *mut NMLVCUSTOMDRAW);
                        if draw.nmcd.dwDrawStage == CDDS_PREPAINT {
                            return CDRF_NOTIFYITEMDRAW as isize;
                        }
                        if draw.nmcd.dwDrawStage == CDDS_ITEMPREPAINT {
                            let ui = &*ui_ptr;
                            if let Some(highlight) = ui.highlight.as_ref() {
                                let matched = highlight.is_active()
                                    && ui
                                        .visible
                                        .get(draw.nmcd.dwItemSpec)
                                        .and_then(|&index| ui.session.as_ref()?.entries().get(index))
                                        .map(|entry| entry.path == highlight.path)
                                        .unwrap_or(false);
                                if matched {
                                    draw.clrTextBk = 0x00D0F0FF;
                                }
                            }
                            return CDRF_DODEFAULT as isize;
                        }
                        return CDRF_DODEFAULT as isize;
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_NCDESTROY => {
                let pointer = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Ui;
                if !pointer.is_null() {
                    let ui = Box::from_raw(pointer);
                    if !ui.font.is_null() {
                        DeleteObject(ui.font);
                    }
                    drop(ui);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CONTEXTMENU => {
                let mut pt: POINT = std::mem::zeroed();
                GetCursorPos(&mut pt);
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
                if !pointer.is_null() {
                    let ui = &*pointer;
                    let paths = selected_paths(ui);
                    if !paths.is_empty() {
                        let files = selected_file_paths(ui);
                        let can_preview = single_selected_file(ui).is_some();
                        let menu = CreatePopupMenu();
                        if !menu.is_null() {
                            let export_flags =
                                MF_STRING | if files.is_empty() { MF_GRAYED } else { 0 };
                            let preview_flags =
                                MF_STRING | if can_preview { 0 } else { MF_GRAYED };
                            append_menu(menu, export_flags, MENU_EXPORT, "导出所选…");
                            append_menu(menu, preview_flags, MENU_PREVIEW, "预览");
                            append_menu(menu, preview_flags, MENU_REPLACE, "替换…");
                            append_separator(menu);
                            let label = if paths.len() > 1 {
                                format!("复制 {} 条包内路径", paths.len())
                            } else {
                                "复制包内路径".to_string()
                            };
                            append_menu(menu, MF_STRING, MENU_COPY_PATH, &label);
                            let cmd = TrackPopupMenu(
                                menu,
                                TPM_LEFTALIGN | TPM_RETURNCMD,
                                pt.x,
                                pt.y,
                                0,
                                hwnd,
                                std::ptr::null(),
                            );
                            DestroyMenu(menu);
                            match cmd as usize {
                                MENU_EXPORT => export_selected(hwnd),
                                MENU_PREVIEW => preview_selected(hwnd),
                                MENU_REPLACE => replace_selected(hwnd),
                                MENU_COPY_PATH => copy_text(hwnd, &paths.join("\r\n")),
                                _ => {}
                            }
                        }
                    }
                }
                0
            }
            WM_TIMER => {
                if wparam == FILTER_TIMER_ID {
                    KillTimer(hwnd, FILTER_TIMER_ID);
                    apply_filter(hwnd);
                } else if wparam == HIGHLIGHT_TIMER_ID {
                    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
                    if !pointer.is_null() {
                        let ui = &mut *pointer;
                        let active = ui
                            .highlight
                            .as_mut()
                            .map(|highlight| {
                                highlight.tick(100);
                                highlight.is_active()
                            })
                            .unwrap_or(false);
                        if !active {
                            ui.highlight = None;
                            KillTimer(hwnd, HIGHLIGHT_TIMER_ID);
                        }
                        InvalidateRect(ui.list, null(), 1);
                    }
                }
                0
            }
            WM_LBUTTONDOWN => {
                click_tab(hwnd, lparam, false);
                0
            }
            WM_MBUTTONDOWN => {
                click_tab(hwnd, lparam, true);
                0
            }
            WM_CLOSE => {
                save_state(hwnd);
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                let menu = GetMenu(hwnd);
                if !menu.is_null() {
                    SetMenu(hwnd, null_mut());
                    DestroyMenu(menu);
                }
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn static_color(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let hdc = wparam as HDC;
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        let background = if !pointer.is_null()
            && ((*pointer).status == lparam as HWND || (*pointer).hits == lparam as HWND)
        {
            COLOR_BTNFACE
        } else {
            COLOR_WINDOW
        };
        SetBkColor(hdc, GetSysColor(background));
        SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
        GetSysColorBrush(background) as isize
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

    unsafe fn selected_positions(ui: &Ui) -> Vec<usize> {
        let mut positions = Vec::new();
        let mut position = SendMessageW(
            ui.list,
            LVM_GETNEXTITEM,
            (-1isize) as usize,
            LVNI_SELECTED as isize,
        );
        while position >= 0 {
            positions.push(position as usize);
            position = SendMessageW(
                ui.list,
                LVM_GETNEXTITEM,
                position as usize,
                LVNI_SELECTED as isize,
            );
        }
        positions
    }

    unsafe fn selected_entries(ui: &Ui) -> Vec<&SessionEntry> {
        let Some(session) = ui.session.as_ref() else {
            return Vec::new();
        };
        selected_positions(ui)
            .into_iter()
            .filter_map(|position| ui.visible.get(position).copied())
            .filter_map(|index| session.entries().get(index))
            .collect()
    }

    unsafe fn selected_paths(ui: &Ui) -> Vec<String> {
        selected_entries(ui)
            .into_iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// 选中项中的文件条目；目录不参与导出。
    unsafe fn selected_file_paths(ui: &Ui) -> Vec<String> {
        selected_entries(ui)
            .into_iter()
            .filter(|entry| !entry.is_dir())
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// 把焦点还给主列表（模态对话框关闭后）。
    unsafe fn focus_list(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if !pointer.is_null() {
            SetFocus((*pointer).list);
        }
    }

    /// 仅当恰好选中一个文件条目时返回它（预览入口）。
    unsafe fn single_selected_file(ui: &Ui) -> Option<&SessionEntry> {
        let selected = selected_entries(ui);
        if selected.len() == 1 && !selected[0].is_dir() {
            Some(selected[0])
        } else {
            None
        }
    }

    unsafe fn clear_list_selection(ui: &Ui) {
        let mut item: LVITEMW = zeroed();
        item.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
        item.state = 0;
        SendMessageW(
            ui.list,
            LVM_SETITEMSTATE,
            (-1isize) as usize,
            &item as *const _ as isize,
        );
    }

    unsafe fn restore_selection_paths(hwnd: HWND, paths: &[String]) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &*pointer;
        let Some(session) = ui.session.as_ref() else {
            return;
        };
        // 行号会随筛选/排序变化，始终按路径重建选择，避免遗留旧位置的选中态。
        clear_list_selection(ui);
        if paths.is_empty() {
            return;
        }
        let mut focus = None;
        for path in paths {
            let Some(position) = ui
                .visible
                .iter()
                .position(|&index| session.entries()[index].path == *path)
            else {
                continue;
            };
            let mut item: LVITEMW = zeroed();
            item.stateMask = LVIS_SELECTED;
            item.state = LVIS_SELECTED;
            SendMessageW(
                ui.list,
                LVM_SETITEMSTATE,
                position,
                &item as *const _ as isize,
            );
            focus = Some(position);
        }
        if let Some(position) = focus {
            let mut item: LVITEMW = zeroed();
            item.stateMask = LVIS_FOCUSED;
            item.state = LVIS_FOCUSED;
            SendMessageW(
                ui.list,
                LVM_SETITEMSTATE,
                position,
                &item as *const _ as isize,
            );
            SendMessageW(ui.list, LVM_ENSUREVISIBLE, position, 0);
        }
    }

    fn active_view_mode(ui: &Ui) -> ViewMode {
        ui.workspace
            .active_index()
            .and_then(|index| ui.workspace.tab(index))
            .map(|tab| tab.view.mode)
            .unwrap_or(ui.column_mode)
    }

    /// 把当前 ListView 列宽与隐藏列写回活动标签和配置（DIP）。
    unsafe fn capture_columns(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &mut *pointer;
        let Some(index) = ui.workspace.active_index() else {
            return;
        };
        if ui.column_tab != Some(index) {
            return;
        }
        let dpi = dpi_of(hwnd);
        let hidden: Vec<String> = ui
            .workspace
            .tab(index)
            .map(|tab| {
                Column::ALL
                    .iter()
                    .filter(|column| {
                        **column != Column::Name && tab.view.columns.is_hidden(**column)
                    })
                    .map(|column| column.key().to_string())
                    .collect()
            })
            .unwrap_or_default();
        for (position, column) in ui.columns.iter().enumerate() {
            let width = SendMessageW(ui.list, LVM_GETCOLUMNWIDTH, position, 0) as i32;
            let dip = if dpi == 0 {
                width
            } else {
                (width as f64 * 96.0 / dpi as f64).round() as i32
            };
            if dip > 0 {
                ui.config.column_widths.insert(column.key().to_string(), dip);
                if let Some(tab) = ui.workspace.tab_mut(index) {
                    tab.view.columns.set_width(*column, dip);
                }
            }
        }
        ui.config.hidden_columns = Some(hidden);
    }

    /// 新标签继承全局列宽与隐藏列。
    fn apply_saved_columns(ui: &mut Ui, index: usize) {
        let widths: Vec<(Column, i32)> = Column::ALL
            .iter()
            .filter_map(|column| {
                ui.config
                    .column_widths
                    .get(column.key())
                    .map(|width| (*column, *width))
            })
            .collect();
        let hidden = ui.config.hidden_columns.clone();
        let Some(tab) = ui.workspace.tab_mut(index) else {
            return;
        };
        for (column, width) in widths {
            if width > 0 {
                tab.view.columns.set_width(column, width);
            }
        }
        if let Some(hidden) = hidden {
            for column in Column::ALL {
                tab.view.columns.hidden.remove(&column);
                if column != Column::Name && hidden.iter().any(|key| key == column.key()) {
                    tab.view.columns.hidden.insert(column);
                }
            }
        }
    }

    /// 在列表指定位置插入一列（宽度已是物理像素）。
    unsafe fn insert_column(list: HWND, position: usize, title: &str, width: i32) {
        let label = wide(title);
        let mut definition: LVCOLUMNW = zeroed();
        definition.mask = LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM;
        definition.cx = width;
        definition.iSubItem = position as i32;
        definition.pszText = label.as_ptr() as *mut u16;
        SendMessageW(
            list,
            LVM_INSERTCOLUMNW,
            position,
            &definition as *const _ as isize,
        );
    }

    /// 按当前标签的视图状态重建列；列宽取标签状态，名称列吸收剩余宽度。
    unsafe fn sync_columns(hwnd: HWND, force: bool, capture: bool) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let active = (*pointer).workspace.active_index();
        let mode = active_view_mode(&*pointer);
        if !force
            && (*pointer).column_tab == active
            && (*pointer).column_mode == mode
            && !(*pointer).columns.is_empty()
        {
            return;
        }
        if capture && (*pointer).column_tab == active && active.is_some() {
            capture_columns(hwnd);
        }
        let ui = &mut *pointer;
        let visible: Vec<Column> = Column::ALL
            .into_iter()
            .filter(|column| {
                if *column == Column::Name {
                    return true;
                }
                match active.and_then(|index| ui.workspace.tab(index)) {
                    Some(tab) => !tab.view.columns.is_hidden(*column),
                    None => default_visible(*column, ViewMode::List),
                }
            })
            .collect();
        let sort = active
            .and_then(|index| ui.workspace.tab(index))
            .map(|tab| tab.view.sort)
            .unwrap_or(SortSpec {
                key: Column::Name,
                dir: SortDir::Asc,
            });
        let dpi = dpi_of(hwnd);
        let old_count = ui.columns.len();
        for _ in 0..old_count {
            SendMessageW(ui.list, LVM_DELETECOLUMN, 0, 0);
        }
        ui.columns.clear();
        let view = active
            .and_then(|index| ui.workspace.tab(index))
            .map(|tab| tab.view.columns.clone());
        for (position, column) in visible.iter().enumerate() {
            let arrow = if *column == sort.key {
                if sort.dir == SortDir::Asc {
                    " ▲"
                } else {
                    " ▼"
                }
            } else {
                ""
            };
            let label = format!("{}{arrow}", column.title());
            let width = scale(
                view.as_ref()
                    .map(|view| view.width(*column))
                    .unwrap_or(column.default_width()),
                dpi,
            );
            insert_column(ui.list, position, &label, width);
            ui.columns.push(*column);
        }
        ui.column_mode = mode;
        ui.column_tab = active;
        apply_column_widths(hwnd);
    }

    unsafe fn apply_column_widths(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() || (*pointer).columns.is_empty() {
            return;
        }
        let ui = &*pointer;
        let (_, dpi, rects) = shell_layout(hwnd);
        let view = ui
            .workspace
            .active_index()
            .and_then(|index| ui.workspace.tab(index))
            .map(|tab| tab.view.columns.clone());
        let mut fixed = 0;
        for (position, column) in ui.columns.iter().enumerate() {
            if *column == Column::Name {
                continue;
            }
            let dip = view
                .as_ref()
                .map(|view| view.width(*column))
                .unwrap_or(column.default_width());
            fixed += dip;
            SendMessageW(ui.list, LVM_SETCOLUMNWIDTH, position, scale(dip, dpi) as isize);
        }
        if let Some(position) = ui.columns.iter().position(|column| *column == Column::Name) {
            let width = (rects.content.w - fixed - 2).max(160);
            SendMessageW(ui.list, LVM_SETCOLUMNWIDTH, position, scale(width, dpi) as isize);
        }
    }

    unsafe fn apply_filter(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let (keyword, saved) = {
            let ui = &*pointer;
            (read_text(ui.edit), selected_paths(ui))
        };

        let (visible, depths) = {
            let ui = &mut *pointer;
            if let Some(session) = ui.session.as_ref() {
                let active = ui.workspace.active_index();
                let view = active
                    .and_then(|index| ui.workspace.tab_mut(index))
                    .map(|tab| {
                        tab.view.query.keyword = keyword.clone();
                        tab.view.clone()
                    });
                match view {
                    Some(view) if view.mode == ViewMode::Tree => {
                        let rows = view.tree.project(session.entries(), view.sort, &view.query);
                        (
                            rows.iter().map(|row| row.index).collect(),
                            rows.iter().map(|row| row.depth).collect(),
                        )
                    }
                    Some(view) => {
                        let mut indices = Query::apply(&view.query, session.entries());
                        sort_indices(session.entries(), &mut indices, view.sort);
                        let len = indices.len();
                        (indices, vec![0; len])
                    }
                    None => (Vec::new(), Vec::new()),
                }
            } else {
                (Vec::new(), Vec::new())
            }
        };

        {
            let ui = &mut *pointer;
            ui.filter = keyword;
            ui.visible = visible;
            ui.depths = depths;
        }
        sync_columns(hwnd, false, false);
        {
            let ui = &mut *pointer;
            SendMessageW(ui.list, LVM_SETITEMCOUNT, ui.visible.len(), 0);
        }
        restore_selection_paths(hwnd, &saved);
        update_auxiliary(hwnd);
        refresh_texts(hwnd);
        InvalidateRect(hwnd, null(), 1);
    }

    unsafe fn set_view_mode(hwnd: HWND, mode: ViewMode) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        if let Some(index) = (*pointer).workspace.active_index() {
            if let Some(tab) = (*pointer).workspace.tab_mut(index) {
                tab.view.mode = mode;
            }
        }
        sync_columns(hwnd, true, true);
        apply_filter(hwnd);
        update_menu_state(hwnd);
    }

    unsafe fn toggle_column(hwnd: HWND, ordinal: usize) {
        let Some(column) = Column::ALL.get(ordinal).copied() else {
            return;
        };
        if column == Column::Name {
            return;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        if let Some(index) = (*pointer).workspace.active_index() {
            if let Some(tab) = (*pointer).workspace.tab_mut(index) {
                tab.view.columns.toggle_hidden(column);
            }
        }
        sync_columns(hwnd, true, true);
        apply_filter(hwnd);
        update_menu_state(hwnd);
    }

    unsafe fn reset_columns(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        if let Some(index) = (*pointer).workspace.active_index() {
            if let Some(tab) = (*pointer).workspace.tab_mut(index) {
                tab.view.columns.reset();
            }
        }
        sync_columns(hwnd, true, false);
        apply_filter(hwnd);
        update_menu_state(hwnd);
    }

    /// 定位到当前首个可见行并高亮 2 秒（G2-06）。
    unsafe fn locate_first(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &mut *pointer;
        let Some(&index) = ui.visible.first() else {
            return;
        };
        let Some(path) = ui
            .session
            .as_ref()
            .and_then(|session| session.entries().get(index))
            .map(|entry| entry.path.clone())
        else {
            return;
        };
        clear_list_selection(ui);
        let mut item: LVITEMW = zeroed();
        item.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
        item.state = LVIS_SELECTED | LVIS_FOCUSED;
        SendMessageW(ui.list, LVM_SETITEMSTATE, 0, &item as *const _ as isize);
        SendMessageW(ui.list, LVM_ENSUREVISIBLE, 0, 0);
        ui.highlight = Some(HighlightState::new(&path, 2000));
        SetTimer(hwnd, HIGHLIGHT_TIMER_ID, 100, None);
        InvalidateRect(ui.list, null(), 1);
        SetFocus(ui.list);
    }

    /// 标签条命中矩形（DIP 转物理像素）；宽度随标签数收缩，超出部分裁掉。
    fn tab_layout(ui: &Ui, tabs: Rect, dpi: u32) -> Vec<(Rect, Rect)> {
        let count = ui.workspace.tabs().len();
        if count == 0 || tabs.w <= 0 {
            return Vec::new();
        }
        let width = (tabs.w / count as i32).clamp(TAB_MIN_DIP, TAB_MAX_DIP);
        let mut result = Vec::with_capacity(count);
        for index in 0..count {
            let x = tabs.x + index as i32 * width;
            if x >= tabs.right() {
                break;
            }
            let w = width.min(tabs.right() - x);
            let close = Rect::new(x + w - TAB_CLOSE_DIP, tabs.y, TAB_CLOSE_DIP, tabs.h).scaled(dpi);
            result.push((Rect::new(x, tabs.y, w, tabs.h).scaled(dpi), close));
        }
        result
    }

    unsafe fn update_window_title(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let path = (*pointer).path.as_ref();
        let title = match path.and_then(|path| path.file_name()) {
            Some(name) => format!("wd-pak — {}", name.to_string_lossy()),
            None => "wd-pak".into(),
        };
        set_text(hwnd, &title);
    }

    /// 把活动标签的会话、筛选与选中项收回运行时槽位。
    unsafe fn store_active(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let Some(index) = (*pointer).workspace.active_index() else {
            return;
        };
        capture_columns(hwnd);
        let ui = &mut *pointer;
        let selected = selected_entry(ui).map(|entry| entry.path.clone());
        if let Some(slot) = ui.runtime.get_mut(index) {
            slot.session = ui.session.take();
            slot.selected = selected;
        }
        if let Some(tab) = ui.workspace.tab_mut(index) {
            tab.view.query.keyword = ui.filter.clone();
        }
    }

    /// 从运行时槽位装载当前活动标签；调用前 `workspace` 的 active 必须已指向目标。
    unsafe fn load_active_view(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let Some(index) = (*pointer).workspace.active_index() else {
            clear_view(hwnd);
            return;
        };
        let (path, filter, session, selected) = {
            let ui = &mut *pointer;
            let path = ui.workspace.tab(index).map(|tab| tab.path().to_path_buf());
            let filter = ui
                .workspace
                .tab(index)
                .map(|tab| tab.view.query.keyword.clone())
                .unwrap_or_default();
            let (session, selected) = match ui.runtime.get_mut(index) {
                Some(slot) => (slot.session.take(), slot.selected.take()),
                None => (None, None),
            };
            (path, filter, session, selected)
        };
        {
            let ui = &mut *pointer;
            ui.session = session;
            ui.path = path;
            ui.filter = filter.clone();
            ui.detail_hidden = false;
            set_text(ui.edit, &filter);
        }
        apply_filter(hwnd);
        if let Some(selected) = selected {
            restore_selection(hwnd, &selected);
        }
        update_window_title(hwnd);
        update_menu_state(hwnd);
        refresh_texts(hwnd);
        InvalidateRect(hwnd, null(), 1);
        SetFocus((*pointer).list);
    }

    unsafe fn activate_tab(hwnd: HWND, index: usize) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() || index >= (*pointer).workspace.tabs().len() {
            return;
        }
        if (*pointer).workspace.active_index() == Some(index) {
            SetFocus((*pointer).list);
            return;
        }
        store_active(hwnd);
        if !(*pointer).workspace.activate(index) {
            return;
        }
        load_active_view(hwnd);
        save_session_state(hwnd);
    }

    unsafe fn cycle_tab(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let count = (*pointer).workspace.tabs().len();
        if count < 2 {
            return;
        }
        let active = (*pointer).workspace.active_index().unwrap_or(0);
        activate_tab(hwnd, (active + 1) % count);
    }

    /// 中键或标签上的关闭区：关闭指定标签，活动标签关闭后接管相邻标签。
    unsafe fn close_tab(hwnd: HWND, index: usize) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() || index >= (*pointer).workspace.tabs().len() {
            return;
        }
        if (*pointer).workspace.tabs()[index].write_busy {
            return;
        }
        let was_active = (*pointer).workspace.active_index() == Some(index);
        if was_active {
            store_active(hwnd);
        }
        {
            let ui = &mut *pointer;
            ui.workspace.close(index);
            ui.runtime.remove(index);
        }
        if was_active {
            if (*pointer).workspace.active_index().is_some() {
                load_active_view(hwnd);
            } else {
                clear_view(hwnd);
            }
        } else {
            update_window_title(hwnd);
            update_menu_state(hwnd);
            InvalidateRect(hwnd, null(), 1);
        }
        save_session_state(hwnd);
    }

    unsafe fn click_tab(hwnd: HWND, lparam: LPARAM, middle: bool) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let x = (lparam & 0xffff) as i32;
        let y = ((lparam >> 16) & 0xffff) as i32;
        let (_client, dpi, rects) = shell_layout(hwnd);
        let tabs = rects.tabs.scaled(dpi);
        if x < tabs.x || x >= tabs.right() || y < tabs.y || y >= tabs.bottom() {
            return;
        }
        for (index, (rect, close)) in tab_layout(&*pointer, rects.tabs, dpi).iter().enumerate() {
            if x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom() {
                if middle || (x >= close.x && x < close.right()) {
                    close_tab(hwnd, index);
                } else {
                    activate_tab(hwnd, index);
                }
                return;
            }
        }
    }

    unsafe fn restore_selection(hwnd: HWND, inner: &str) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &*pointer;
        let Some(session) = ui.session.as_ref() else {
            return;
        };
        let Some(position) = ui
            .visible
            .iter()
            .position(|&index| session.entries()[index].path == inner)
        else {
            return;
        };
        let mut item: LVITEMW = zeroed();
        item.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
        item.state = LVIS_SELECTED | LVIS_FOCUSED;
        SendMessageW(
            ui.list,
            LVM_SETITEMSTATE,
            position,
            &item as *const _ as isize,
        );
        SendMessageW(ui.list, LVM_ENSUREVISIBLE, position, 0);
    }

    /// 清空浏览区（无活动标签）；保留标签栏与菜单。
    unsafe fn clear_view(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &mut *pointer;
        ui.session = None;
        ui.path = None;
        ui.visible.clear();
        ui.depths.clear();
        ui.filter.clear();
        ui.detail_hidden = false;
        ui.highlight = None;
        SendMessageW(ui.list, LVM_SETITEMCOUNT, 0, 0);
        set_text(ui.edit, "");
        set_text(hwnd, "wd-pak");
        update_auxiliary(hwnd);
        refresh_texts(hwnd);
        update_menu_state(hwnd);
        InvalidateRect(hwnd, null(), 1);
    }

    unsafe fn session_snapshot(hwnd: HWND) -> SessionSnapshot {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return SessionSnapshot::default();
        }
        let ui = &*pointer;
        let active = ui.workspace.active_index();
        let tabs = ui
            .workspace
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let is_active = active == Some(index);
                let selected = if is_active {
                    selected_entry(ui).map(|entry| entry.path.clone())
                } else {
                    ui.runtime
                        .get(index)
                        .and_then(|slot| slot.selected.clone())
                };
                let filter = if is_active {
                    ui.filter.clone()
                } else {
                    tab.view.query.keyword.clone()
                };
                TabSnapshot {
                    path: tab.path().to_string_lossy().into_owned(),
                    filter,
                    selected,
                }
            })
            .collect();
        SessionSnapshot {
            version: persist::SESSION_VERSION,
            active: active.unwrap_or(0),
            tabs,
        }
    }

    unsafe fn save_session_state(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &*pointer;
        let snapshot = session_snapshot(hwnd);
        let _ = persist::save_session(&ui.session_path, &snapshot);
    }

    /// 退出前保存窗口几何、最近打开与会话快照。
    unsafe fn save_state(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        capture_columns(hwnd);
        let snapshot = session_snapshot(hwnd);
        let ui = &mut *pointer;
        let dpi = dpi_of(hwnd);
        let mut rect: RECT = zeroed();
        GetWindowRect(hwnd, &mut rect);
        let zoomed = IsZoomed(hwnd) != 0;
        ui.config.window.maximized = zoomed;
        if !zoomed {
            ui.config.window.x = (rect.left * 96) / dpi as i32;
            ui.config.window.y = (rect.top * 96) / dpi as i32;
            ui.config.window.width = ((rect.right - rect.left) * 96) / dpi as i32;
            ui.config.window.height = ((rect.bottom - rect.top) * 96) / dpi as i32;
        }
        persist_config(ui);
        let _ = persist::save_session(&ui.session_path, &snapshot);
    }

    fn now_seconds() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    }

    fn persist_config(ui: &Ui) {
        if ui.config_read_only {
            return;
        }
        persist_config(ui);
    }

    unsafe fn record_recent(hwnd: HWND, path: &std::path::Path) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        {
            let ui = &mut *pointer;
            ui.config
                .touch_recent(&path.to_string_lossy(), now_seconds());
            persist_config(ui);
        }
        rebuild_recent_menu(hwnd);
    }

    unsafe fn rebuild_recent_menu(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &mut *pointer;
        let menu = ui.recent_menu;
        if menu.is_null() {
            return;
        }
        while GetMenuItemCount(menu) > 0 {
            DeleteMenu(menu, 0, MF_BYPOSITION);
        }
        let entries: Vec<(String, String)> = ui
            .config
            .recent
            .iter()
            .take(MENU_RECENT_MAX)
            .map(|entry| {
                let name = std::path::Path::new(&entry.path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| entry.path.clone());
                (entry.path.clone(), name)
            })
            .collect();
        if entries.is_empty() {
            append_menu(menu, MF_STRING | MF_GRAYED, MENU_RECENT_BASE, "（空）");
        } else {
            for (index, (path, name)) in entries.iter().enumerate() {
                let mut label = format!("{}  —  {}", name, path);
                if label.contains('&') {
                    label = label.replace('&', "&&");
                }
                append_menu(menu, MF_STRING, MENU_RECENT_BASE + index, &label);
            }
            append_separator(menu);
            append_menu(menu, MF_STRING, MENU_RECENT_CLEAR, "清空最近打开…");
        }
        DrawMenuBar(hwnd);
    }

    unsafe fn open_recent(hwnd: HWND, index: usize) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let path = {
            let ui = &*pointer;
            ui.config
                .recent
                .get(index)
                .map(|entry| PathBuf::from(&entry.path))
        };
        if let Some(path) = path {
            open_path(hwnd, path, OpenSource::Recent);
        }
    }

    unsafe fn clear_recent(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() || (*pointer).config.recent.is_empty() {
            return;
        }
        let question = wide("确定清空最近打开列表？");
        let caption = wide("清除最近打开");
        if MessageBoxW(
            hwnd,
            question.as_ptr(),
            caption.as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        ) == 6
        {
            let ui = &mut *pointer;
            ui.config.clear_recent();
            persist_config(ui);
            rebuild_recent_menu(hwnd);
        }
    }

    /// 命中数、空状态与筛选后的状态栏（规格 §2.8 / §3.3）。
    unsafe fn update_auxiliary(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let ui = &*pointer;
        let has_session = ui.session.is_some();
        let filtering = !ui.filter.is_empty();
        let hits = if has_session && filtering {
            format!("{} 项", ui.visible.len())
        } else {
            String::new()
        };
        set_text(ui.hits, &hits);
        ShowWindow(ui.hits, if hits.is_empty() { SW_HIDE } else { SW_SHOW });

        let empty = if !has_session {
            "尚未打开包\n\n点击「打开 PAK」或按 Ctrl+O"
        } else if ui.visible.is_empty() {
            if filtering {
                "没有匹配的条目\n\n按 Esc 清除搜索条件"
            } else {
                "包内没有文件条目"
            }
        } else {
            ""
        };
        set_text(ui.empty, empty);
        ShowWindow(ui.empty, if empty.is_empty() { SW_HIDE } else { SW_SHOW });
    }

    /// Esc 的上下文顺序：先清搜索，再关闭 Compact 详情浮层（规格 §6.5）。
    unsafe fn handle_escape(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        if !(&(*pointer).filter).is_empty() {
            let edit = (*pointer).edit;
            let list = (*pointer).list;
            set_text(edit, "");
            apply_filter(hwnd);
            SetFocus(list);
            return;
        }
        let (_client, _dpi, rects) = shell_layout(hwnd);
        if rects.detail_overlay.is_some() && !(*pointer).detail_hidden {
            (*pointer).detail_hidden = true;
            layout_controls(hwnd);
            InvalidateRect(hwnd, null(), 0);
        }
    }

    unsafe fn shell_layout(hwnd: HWND) -> (RECT, u32, ShellRects) {
        let (client, dpi) = client_metrics(hwnd);
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
            scale(8, dpi),
            scale(110, dpi),
            scale(32, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowPos(
            ui.export_button,
            null_mut(),
            scale(126, dpi),
            scale(8, dpi),
            scale(110, dpi),
            scale(32, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let edit_w = scale(320, dpi);
        let edit_h = scale(32, dpi);
        let edit_x = client.right - edit_w - scale(8, dpi);
        SetWindowPos(
            ui.edit,
            null_mut(),
            edit_x,
            scale(8, dpi),
            edit_w,
            edit_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let hits_w = scale(HIT_TEXT_W, dpi);
        SetWindowPos(
            ui.hits,
            null_mut(),
            (edit_x - hits_w - scale(8, dpi)).max(scale(244, dpi)),
            scale(8, dpi),
            hits_w,
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
        SetWindowPos(
            ui.empty,
            null_mut(),
            content.x,
            content.y,
            content.w,
            content.h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        apply_column_widths(hwnd);

        // Compact 断点没有独立详情列，使用浮层矩形，否则详情面板会变成 0 宽。
        let detail = rects.detail_overlay.unwrap_or(rects.detail).scaled(dpi);
        let show_detail = rects.detail_overlay.is_none() || !ui.detail_hidden;
        ShowWindow(ui.detail, if show_detail { SW_SHOW } else { SW_HIDE });
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
        set_text(ui.detail, &detail_text(ui));
        set_text(ui.status, &status_text(ui));
        update_auxiliary(hwnd);
    }

    unsafe fn open_pak(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        let initial = if pointer.is_null() {
            None
        } else {
            (*pointer).last_dir.clone()
        };
        let Some(path) = pick_file_dialog(
            hwnd,
            "打开 PAK",
            "PAK 文件\0*.pak\0\0",
            initial.as_deref(),
        ) else {
            return;
        };
        open_path(hwnd, path, OpenSource::CommandBar);
    }

    struct ProgressState {
        loader: Option<Loader>,
        label: HWND,
        bar: HWND,
        cancel: HWND,
        result: *mut Option<LoadResult>,
    }

    unsafe fn open_path(hwnd: HWND, path: PathBuf, source: OpenSource) {
        let Some(request) = OpenRequest::new(&path, source) else {
            return;
        };
        let path = request.path;
        let existing = {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
            if pointer.is_null() {
                return;
            }
            let key = wd_pak::platform::path_key(&path);
            (*pointer)
                .workspace
                .tabs()
                .iter()
                .position(|tab| wd_pak::platform::path_key(tab.path()) == key)
        };
        if let Some(index) = existing {
            activate_tab(hwnd, index);
            return;
        }
        let Some(result) = load_with_progress(hwnd, &path) else {
            return;
        };
        match result {
            LoadResult::Ready(session) => install_session(hwnd, path, *session),
            LoadResult::Failed(error) => show_error(hwnd, &error),
            LoadResult::Cancelled => {}
        }
    }

    /// F5：重新读取当前包，保留筛选条件与选中项（规格 §2.4）。
    unsafe fn reload_pak(hwnd: HWND) {
        let (path, selected) = {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
            if pointer.is_null() {
                return;
            }
            let ui = &*pointer;
            (
                ui.path.clone(),
                selected_entry(ui).map(|entry| entry.path.clone()),
            )
        };
        let Some(path) = path else {
            return;
        };
        let Some(result) = load_with_progress(hwnd, &path) else {
            return;
        };
        match result {
            LoadResult::Ready(session) => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
                if pointer.is_null() {
                    return;
                }
                (*pointer).session = Some(*session);
                apply_filter(hwnd);
                if let Some(selected) = selected {
                    restore_selection(hwnd, &selected);
                }
                update_window_title(hwnd);
                update_menu_state(hwnd);
                refresh_texts(hwnd);
                save_session_state(hwnd);
            }
            LoadResult::Failed(error) => show_error(hwnd, &error),
            LoadResult::Cancelled => {}
        }
    }

    /// 新包加载成功：新建标签并成为活动标签；同路径已在标签中则直接激活。
    unsafe fn install_session(hwnd: HWND, path: PathBuf, session: PakSession) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let key = wd_pak::platform::path_key(&path);
        if let Some(index) = (*pointer)
            .workspace
            .tabs()
            .iter()
            .position(|tab| wd_pak::platform::path_key(tab.path()) == key)
        {
            activate_tab(hwnd, index);
            return;
        }
        store_active(hwnd);
        let index = {
            let ui = &mut *pointer;
            ui.last_dir = path.parent().map(|parent| parent.to_path_buf());
            let index = ui.workspace.open(path);
            ui.runtime.push(TabRuntime {
                session: None,
                selected: None,
            });
            ui.workspace.activate(index);
            apply_saved_columns(ui, index);
            let path = ui
                .workspace
                .tab(index)
                .map(|tab| tab.path().to_path_buf());
            ui.session = Some(session);
            ui.path = path;
            ui.filter.clear();
            ui.detail_hidden = false;
            ui.highlight = None;
            set_text(ui.edit, "");
            index
        };
        apply_filter(hwnd);
        let path = (*pointer)
            .workspace
            .tab(index)
            .map(|tab| tab.path().to_path_buf());
        if let Some(path) = path {
            record_recent(hwnd, &path);
        }
        update_window_title(hwnd);
        update_menu_state(hwnd);
        refresh_texts(hwnd);
        save_session_state(hwnd);
        InvalidateRect(hwnd, null(), 1);
        SetFocus((*pointer).list);
    }

    /// Ctrl+W：关闭当前标签；只剩一个标签时回到空状态。
    unsafe fn close_pak(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        if let Some(index) = (*pointer).workspace.active_index() {
            close_tab(hwnd, index);
        }
    }

    /// 启动恢复：逐条重开上次的标签，跳过失效包；全部失效时提示降级。
    unsafe fn restore_session(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
        if pointer.is_null() {
            return;
        }
        let (session_path, restore) = {
            let ui = &*pointer;
            (ui.session_path.clone(), ui.config.ui.restore_session)
        };
        if !restore {
            return;
        }
        let Some(snapshot) = persist::load_session(&session_path) else {
            return;
        };
        if snapshot.tabs.is_empty() {
            return;
        }
        let mut loaded = 0usize;
        for tab in &snapshot.tabs {
            let path = PathBuf::from(&tab.path);
            let Ok(session) = PakSession::open(&path) else {
                continue;
            };
            let ui = &mut *pointer;
            let before = ui.workspace.tabs().len();
            let index = ui.workspace.open(path);
            if index < before {
                continue;
            }
            apply_saved_columns(ui, index);
            ui.runtime.push(TabRuntime {
                session: Some(session),
                selected: tab.selected.clone(),
            });
            if let Some(tab_state) = ui.workspace.tab_mut(index) {
                tab_state.view.query.keyword = tab.filter.clone();
            }
            loaded += 1;
        }
        if loaded == 0 {
            show_message(
                hwnd,
                "上次打开的包已不可用，已进入空工作区。",
                MB_OK | MB_ICONWARNING,
            );
            return;
        }
        let target = snapshot.active.min(loaded - 1);
        (*pointer).workspace.activate(target);
        load_active_view(hwnd);
        save_session_state(hwnd);
    }

    /// 同步读取包（进度窗口无法创建时的回退路径）。
    fn load_direct(path: &Path) -> LoadResult {
        match PakSession::open(path) {
            Ok(session) => LoadResult::Ready(Box::new(session)),
            Err(error) => LoadResult::Failed(error),
        }
    }

    unsafe fn load_with_progress(hwnd: HWND, path: &std::path::Path) -> Option<LoadResult> {
        let mut result: Option<LoadResult> = None;
        let (class_name, instance) =
            register_dialog_class(PROGRESS_CLASS_NAME, Some(progress_wndproc), COLOR_BTNFACE);

        let dpi = dpi_of(hwnd);
        let width = scale(420, dpi);
        let height = scale(170, dpi);
        let dialog = create_centered_dialog(
            hwnd,
            &class_name,
            instance,
            "打开 PAK",
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            width,
            height,
        );
        if dialog.is_null() {
            return Some(load_direct(path));
        }

        let label = create_label(dialog, instance, "正在读取包…");
        let bar = create_progress(dialog, instance, PBS_MARQUEE);
        let cancel = create_button(dialog, instance, "取消", 0, CANCEL_BUTTON_ID);
        if !all_present(&[label, bar, cancel]) {
            DestroyWindow(dialog);
            return Some(load_direct(path));
        }

        apply_default_font(&[label, cancel]);

        let state = Box::new(ProgressState {
            loader: Some(Loader::start(path.to_path_buf())),
            label,
            bar,
            cancel,
            result: &mut result,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_progress(dialog);
        SendMessageW(bar, PBM_SETMARQUEE, 1, 30);
        SetTimer(dialog, PROGRESS_TIMER_ID, 50, None);

        run_modal(dialog, hwnd, cancel);
        result
    }

    unsafe extern "system" fn progress_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_progress(hwnd);
                0
            }
            WM_TIMER => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ProgressState;
                if !pointer.is_null() && wparam == PROGRESS_TIMER_ID {
                    let state = &mut *pointer;
                    let done = state.loader.as_mut().and_then(|loader| loader.poll());
                    if let Some(result) = done {
                        *state.result = Some(result);
                        KillTimer(hwnd, PROGRESS_TIMER_ID);
                        DestroyWindow(hwnd);
                    } else if let Some(loader) = state.loader.as_ref() {
                        let count = loader.progress().unwrap_or(0);
                        let text = format!("正在读取包…（已读取 {count} 个条目）");
                        set_text(state.label, &text);
                    }
                }
                0
            }
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ProgressState;
                if !pointer.is_null() && code == BN_CLICKED && id == CANCEL_BUTTON_ID as i32 {
                    cancel_loading(&mut *pointer);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ProgressState;
                if !pointer.is_null() {
                    cancel_loading(&mut *pointer);
                }
                0
            }
            WM_DESTROY => {
                KillTimer(hwnd, PROGRESS_TIMER_ID);
                0
            }
            WM_NCDESTROY => {
                let pointer = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut ProgressState;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn cancel_loading(state: &mut ProgressState) {
        if let Some(loader) = state.loader.as_ref() {
            loader.cancel();
        }
        EnableWindow(state.cancel, 0);
        set_text(state.label, "正在取消…");
    }

    unsafe fn layout_progress(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ProgressState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(16, dpi);
        let width = client.right - margin * 2;
        let button_w = scale(100, dpi);
        let button_h = scale(32, dpi);
        place(state.label, margin, scale(14, dpi), width, scale(24, dpi));
        place(state.bar, margin, scale(48, dpi), width, scale(18, dpi));
        place(
            state.cancel,
            client.right - margin - button_w,
            client.bottom - margin - button_h,
            button_w,
            button_h,
        );
    }

    unsafe fn update_menu_state(hwnd: HWND) {
        let menu = GetMenu(hwnd);
        if menu.is_null() {
            return;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        let (has_session, has_selection, has_files, can_preview, can_locate, mode, hidden) =
            if pointer.is_null() {
                (false, false, false, false, false, ViewMode::List, Vec::new())
            } else {
                let ui = &*pointer;
                let hidden: Vec<Column> = ui
                    .workspace
                    .active_index()
                    .and_then(|index| ui.workspace.tab(index))
                    .map(|tab| {
                        Column::ALL
                            .iter()
                            .copied()
                            .filter(|column| tab.view.columns.is_hidden(*column))
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    ui.session.is_some(),
                    !selected_positions(ui).is_empty(),
                    !selected_file_paths(ui).is_empty(),
                    single_selected_file(ui).is_some(),
                    !ui.visible.is_empty(),
                    active_view_mode(ui),
                    hidden,
                )
            };
        let enable = |flag: bool| MF_BYCOMMAND | if flag { MF_ENABLED } else { MF_GRAYED };

        let file_menu = GetSubMenu(menu, 0);
        if !file_menu.is_null() {
            EnableMenuItem(file_menu, MENU_CLOSE as u32, enable(has_session));
            EnableMenuItem(file_menu, MENU_REFRESH as u32, enable(has_session));
            EnableMenuItem(file_menu, MENU_EXPORT as u32, enable(has_files));
            EnableMenuItem(file_menu, MENU_PREVIEW as u32, enable(can_preview));
            EnableMenuItem(file_menu, MENU_REPLACE as u32, enable(can_preview));
            EnableMenuItem(file_menu, MENU_PACK as u32, enable(true));
            EnableMenuItem(file_menu, MENU_UNPACK as u32, enable(has_session));
        }
        let edit_menu = GetSubMenu(menu, 1);
        if !edit_menu.is_null() {
            EnableMenuItem(edit_menu, MENU_FIND as u32, enable(has_session));
            EnableMenuItem(edit_menu, MENU_COPY_PATH as u32, enable(has_selection));
        }
        let view_menu = GetSubMenu(menu, 2);
        if !view_menu.is_null() {
            let check = |flag: bool| MF_BYCOMMAND | if flag { MF_CHECKED } else { 0 };
            CheckMenuItem(
                view_menu,
                MENU_VIEW_TREE as u32,
                check(has_session && mode == ViewMode::Tree),
            );
            CheckMenuItem(
                view_menu,
                MENU_VIEW_LIST as u32,
                check(has_session && mode == ViewMode::List),
            );
            EnableMenuItem(view_menu, MENU_VIEW_TREE as u32, enable(has_session));
            EnableMenuItem(view_menu, MENU_VIEW_LIST as u32, enable(has_session));
            EnableMenuItem(view_menu, MENU_COLUMNS_RESET as u32, enable(has_session));
            EnableMenuItem(
                view_menu,
                MENU_LOCATE as u32,
                enable(has_session && can_locate),
            );
            let columns_menu = GetSubMenu(view_menu, 3);
            if !columns_menu.is_null() {
                for (ordinal, column) in Column::ALL.iter().enumerate() {
                    if *column == Column::Name {
                        continue;
                    }
                    let id = (MENU_COLUMN_BASE + ordinal) as u32;
                    CheckMenuItem(columns_menu, id, check(!hidden.contains(column)));
                    EnableMenuItem(columns_menu, id, enable(has_session));
                }
            }
        }
        if !pointer.is_null() {
            EnableWindow((*pointer).export_button, has_files as i32);
        }
        DrawMenuBar(hwnd);
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
        if let Some(ui) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui).as_mut() {
            ui.error_log.record(now_seconds(), &title, &details);
        }

        let (class_name, instance) =
            register_dialog_class(ERROR_CLASS, Some(error_wndproc), COLOR_WINDOW);

        let dpi = dpi_of(hwnd);
        let width = scale(560, dpi);
        let height = scale(280, dpi);
        let dialog = create_centered_dialog(
            hwnd,
            &class_name,
            instance,
            "wd-pak",
            WS_EX_DLGMODALFRAME,
            width,
            height,
        );
        if dialog.is_null() {
            show_message(
                hwnd,
                &format!("{title}\n\n{reason}\n\n{details}"),
                MB_OK | MB_ICONERROR,
            );
            return;
        }

        let details_text = wide(&details);
        let title_control = create_label(dialog, instance, title);
        let reason_control = create_label(dialog, instance, reason);
        let suggestion_control = create_label(dialog, instance, &suggestion);
        let details_control = create_control(
            dialog,
            instance,
            "EDIT",
            details_text.as_ptr(),
            WS_CHILD | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32,
            0,
            0,
        );
        let toggle = create_button(dialog, instance, "技术详情", 0, DETAIL_BUTTON_ID);
        let copy = create_button(dialog, instance, "复制详情", 0, COPY_BUTTON_ID);
        let close = create_button(
            dialog,
            instance,
            "关闭",
            BS_DEFPUSHBUTTON as u32,
            CLOSE_BUTTON_ID,
        );
        if !all_present(&[
            title_control,
            reason_control,
            suggestion_control,
            details_control,
            toggle,
            copy,
            close,
        ]) {
            DestroyWindow(dialog);
            return;
        }

        apply_default_font(&[
            title_control,
            reason_control,
            suggestion_control,
            details_control,
            toggle,
            copy,
            close,
        ]);

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

        run_modal(dialog, hwnd, close);
    }

    unsafe extern "system" fn error_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_CTLCOLORSTATIC => static_color(hwnd, wparam, lparam),
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
                        set_text(state.toggle, label);
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
                        copy_text(hwnd, &read_text(state.details));
                        return 0;
                    }
                    if id == CLOSE_BUTTON_ID as i32 || id == IDCANCEL as i32 {
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
        let (client, dpi) = client_metrics(hwnd);
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

    unsafe fn is_checked(control: HWND) -> bool {
        SendMessageW(control, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
    }

    unsafe fn set_checked(control: HWND, checked: bool) {
        SendMessageW(
            control,
            BM_SETCHECK,
            if checked { BST_CHECKED as usize } else { 0 },
            0,
        );
    }

    unsafe fn dialog_color(wparam: WPARAM) -> LRESULT {
        let hdc = wparam as HDC;
        SetBkColor(hdc, GetSysColor(COLOR_BTNFACE));
        SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
        GetSysColorBrush(COLOR_BTNFACE) as isize
    }

    unsafe fn show_message(owner: HWND, text: &str, flags: u32) {
        let text = wide(text);
        let caption = wide("wd-pak");
        MessageBoxW(owner, text.as_ptr(), caption.as_ptr(), flags);
    }

    /// EDIT 控件需要 CRLF 才能正确换行。
    fn to_crlf(text: &str) -> String {
        let mut out = String::with_capacity(text.len() + text.matches('\n').count());
        for ch in text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                out.push('\r');
            }
            out.push(ch);
        }
        out
    }

    unsafe fn preview_selected(main: HWND) {
        let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let path = single_selected_file(&*pointer).map(|entry| entry.path.clone());
        if let Some(path) = path {
            preview_path(main, main, &path);
            focus_list(main);
        }
    }

    struct PreviewState {
        main: HWND,
        info: HWND,
        text: HWND,
        text_button: HWND,
        hex_button: HWND,
        export_button: HWND,
        close: HWND,
        data: Vec<u8>,
        path: String,
        kind: PreviewKind,
        font: HFONT,
    }

    /// 只读预览（G3-03）：文本/十六进制、1 MB 截断提示、导出入口。
    unsafe fn preview_path(owner: HWND, main: HWND, path: &str) {
        let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let origin = {
            let ui = &*pointer;
            let Some(session) = ui.session.as_ref() else {
                return;
            };
            match session.entries().iter().find(|entry| entry.path == path) {
                Some(entry) => entry.org as u64,
                None => return,
            }
        };
        if origin > PREVIEW_READ_GUARD {
            show_message(
                owner,
                &format!(
                    "文件较大（{origin} 字节），预览已跳过。\n请使用「导出所选」查看完整内容。"
                ),
                MB_OK | MB_ICONWARNING,
            );
            return;
        }
        let data = {
            let ui = &*pointer;
            let Some(session) = ui.session.as_ref() else {
                return;
            };
            match session.read(path) {
                Ok(data) => data,
                Err(error) => {
                    show_error(main, &error);
                    return;
                }
            }
        };

        let (class_name, instance) =
            register_dialog_class(PREVIEW_CLASS, Some(preview_wndproc), COLOR_BTNFACE);

        let dpi = dpi_of(main);
        let width = scale(820, dpi);
        let height = scale(600, dpi);
        let dialog = create_centered_dialog(
            owner,
            &class_name,
            instance,
            &format!("预览 - {path}"),
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            width,
            height,
        );
        if dialog.is_null() {
            return;
        }

        let info = create_label(dialog, instance, "");
        let text_control = create_control(
            dialog,
            instance,
            "EDIT",
            null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | WS_HSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32
                | ES_AUTOHSCROLL as u32,
            WS_EX_CLIENTEDGE,
            0,
        );
        let text_button = create_button(
            dialog,
            instance,
            "文本",
            WS_GROUP | BS_AUTORADIOBUTTON as u32,
            PREVIEW_TEXT_ID,
        );
        let hex_button = create_button(
            dialog,
            instance,
            "十六进制",
            BS_AUTORADIOBUTTON as u32,
            PREVIEW_HEX_ID,
        );
        let export_button = create_button(dialog, instance, "导出…", 0, PREVIEW_EXPORT_ID);
        let close = create_button(
            dialog,
            instance,
            "关闭",
            BS_DEFPUSHBUTTON as u32,
            PREVIEW_CLOSE_ID,
        );
        if !all_present(&[
            info,
            text_control,
            text_button,
            hex_button,
            export_button,
            close,
        ]) {
            DestroyWindow(dialog);
            return;
        }

        let face = wide("Consolas");
        let mono = CreateFontW(
            -scale(10, dpi),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            face.as_ptr(),
        );
        SendMessageW(text_control, WM_SETFONT, mono as usize, 1);
        apply_default_font(&[info, text_button, hex_button, export_button, close]);
        SendMessageW(text_control, EM_LIMITTEXT, 0, 0);

        let initial = preview_for(&data, PREVIEW_LIMIT);
        let state = Box::new(PreviewState {
            main,
            info,
            text: text_control,
            text_button,
            hex_button,
            export_button,
            close,
            data,
            path: path.to_string(),
            kind: PreviewKind::Text,
            font: mono,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_preview(dialog);
        refresh_preview(dialog, initial.kind);

        run_modal(dialog, owner, text_control);
    }

    unsafe fn refresh_preview(hwnd: HWND, kind: PreviewKind) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PreviewState;
        if pointer.is_null() {
            return;
        }
        let (preview, info) = {
            let state = &*pointer;
            let preview = if kind == PreviewKind::Hex {
                preview_hex(&state.data, HEX_PREVIEW_LIMIT)
            } else {
                preview_truncate(&state.data, PREVIEW_LIMIT)
            };
            let mut info = format!("{}  ·  {} 字节", state.path, preview.original_size);
            if preview.truncated {
                info.push_str(&format!(
                    "  ·  预览已截断（显示前 {} 字节），导出内容不受影响",
                    preview.limit
                ));
            }
            if kind == PreviewKind::Hex && looks_binary(&state.data) {
                info.push_str("  ·  二进制内容");
            }
            (preview, info)
        };
        set_text((*pointer).text, &to_crlf(&preview.content));
        set_text((*pointer).info, &info);
        set_checked((*pointer).text_button, kind == PreviewKind::Text);
        set_checked((*pointer).hex_button, kind == PreviewKind::Hex);
        (*pointer).kind = kind;
    }

    unsafe fn layout_preview(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const PreviewState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(16, dpi);
        let button_h = scale(30, dpi);
        let width = client.right - margin * 2;
        let footer_y = client.bottom - margin - button_h;
        place(state.info, margin, scale(12, dpi), width, scale(36, dpi));
        place(
            state.text,
            margin,
            scale(54, dpi),
            width,
            footer_y - scale(54, dpi) - scale(8, dpi),
        );
        place(state.text_button, margin, footer_y, scale(84, dpi), button_h);
        place(
            state.hex_button,
            margin + scale(92, dpi),
            footer_y,
            scale(100, dpi),
            button_h,
        );
        let close_w = scale(100, dpi);
        let export_w = scale(110, dpi);
        place(
            state.close,
            client.right - margin - close_w,
            footer_y,
            close_w,
            button_h,
        );
        place(
            state.export_button,
            client.right - margin - close_w - scale(8, dpi) - export_w,
            footer_y,
            export_w,
            button_h,
        );
    }

    unsafe extern "system" fn preview_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_preview(hwnd);
                0
            }
            WM_CTLCOLORSTATIC => dialog_color(wparam),
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                if code == BN_CLICKED {
                    match id as usize {
                        PREVIEW_TEXT_ID => {
                            refresh_preview(hwnd, PreviewKind::Text);
                            return 0;
                        }
                        PREVIEW_HEX_ID => {
                            refresh_preview(hwnd, PreviewKind::Hex);
                            return 0;
                        }
                        PREVIEW_EXPORT_ID => {
                            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const PreviewState;
                            if !pointer.is_null() {
                                let path = (*pointer).path.clone();
                                let main = (*pointer).main;
                                open_export_dialog(hwnd, main, vec![path]);
                            }
                            return 0;
                        }
                        PREVIEW_CLOSE_ID => {
                            DestroyWindow(hwnd);
                            return 0;
                        }
                        _ => {}
                    }
                }
                if id == IDCANCEL as i32 {
                    DestroyWindow(hwnd);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                let pointer = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut PreviewState;
                if !pointer.is_null() {
                    let state = Box::from_raw(pointer);
                    if !state.font.is_null() {
                        DeleteObject(state.font);
                    }
                    drop(state);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    struct ExportDialogState {
        main: HWND,
        dir_label: HWND,
        dir: HWND,
        browse: HWND,
        name_label: HWND,
        name: HWND,
        hint: HWND,
        structure: HWND,
        open_folder: HWND,
        conflict_label: HWND,
        conflict: HWND,
        ok: HWND,
        cancel: HWND,
        entries: Vec<String>,
        single: bool,
    }

    #[derive(Clone)]
    struct ExportItem {
        inner: String,
        target: PathBuf,
        skip: bool,
        replace: bool,
    }

    #[derive(Default)]
    struct ExportOutcome {
        written: usize,
        skipped: usize,
        bytes: u64,
        failures: Vec<(String, String)>,
        cancelled: bool,
        fatal: Option<String>,
    }

    unsafe fn export_selected(main: HWND) {
        let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() {
            return;
        }
        let entries = selected_file_paths(&*pointer);
        if entries.is_empty() {
            return;
        }
        open_export_dialog(main, main, entries);
        focus_list(main);
    }

    unsafe fn open_export_dialog(owner: HWND, main: HWND, entries: Vec<String>) {
        let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() || entries.is_empty() {
            return;
        }
        let (default_dir, single, default_name) = {
            let ui = &*pointer;
            let dir = ui
                .config
                .last_export_dir
                .clone()
                .map(PathBuf::from)
                .filter(|path| path.is_dir())
                .or_else(|| ui.last_dir.clone())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let single = entries.len() == 1;
            let name = entries
                .first()
                .map(|inner| default_export_name(inner))
                .unwrap_or_default();
            (dir, single, name)
        };

        let (class_name, instance) =
            register_dialog_class(EXPORT_CLASS, Some(export_wndproc), COLOR_BTNFACE);

        let dpi = dpi_of(owner);
        let width = scale(640, dpi);
        let height = scale(380, dpi);
        let dialog = create_centered_dialog(
            owner,
            &class_name,
            instance,
            "导出所选",
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            width,
            height,
        );
        if dialog.is_null() {
            return;
        }

        let dir_label = create_label(dialog, instance, "导出到：");
        let dir_text = default_dir.to_string_lossy().into_owned();
        let dir = create_text_field(dialog, instance, Some(&dir_text), EXPORT_DIR_ID);
        let browse = create_button(dialog, instance, "浏览…", 0, EXPORT_BROWSE_ID);
        let name_label = create_label(dialog, instance, "文件名：");
        let name_text = if single {
            default_name.clone()
        } else {
            format!("（多选：共 {} 项）", entries.len())
        };
        let name = create_text_field(dialog, instance, Some(&name_text), EXPORT_NAME_ID);
        let hint = create_label(
            dialog,
            instance,
            if single {
                "导出的字节与 cat 一致；目录条目不会被导出。"
            } else {
                "按包内路径导出；目录条目不会被导出。"
            },
        );
        let structure = create_button(
            dialog,
            instance,
            "保留包内目录结构",
            BS_AUTOCHECKBOX as u32,
            EXPORT_STRUCTURE_ID,
        );
        let open_folder = create_button(
            dialog,
            instance,
            "导出完成后打开文件夹",
            BS_AUTOCHECKBOX as u32,
            EXPORT_OPEN_ID,
        );
        let conflict_label = create_label(dialog, instance, "同名文件：");
        let conflict = create_control(
            dialog,
            instance,
            "COMBOBOX",
            null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST as u32,
            0,
            EXPORT_CONFLICT_ID,
        );
        let ok = create_button(
            dialog,
            instance,
            "导出",
            BS_DEFPUSHBUTTON as u32,
            EXPORT_OK_ID,
        );
        let cancel = create_button(dialog, instance, "取消", 0, EXPORT_CANCEL_ID);
        if !all_present(&[
            dir_label,
            dir,
            browse,
            name_label,
            name,
            hint,
            structure,
            open_folder,
            conflict_label,
            conflict,
            ok,
            cancel,
        ]) {
            DestroyWindow(dialog);
            return;
        }

        if !single {
            EnableWindow(name, 0);
        }
        set_checked(structure, true);
        apply_default_font(&[
            dir_label,
            dir,
            browse,
            name_label,
            name,
            hint,
            structure,
            open_folder,
            conflict_label,
            conflict,
            ok,
            cancel,
        ]);
        for option in ["询问", "替换", "跳过", "自动改名"] {
            SendMessageW(conflict, CB_ADDSTRING, 0, wide(option).as_ptr() as isize);
        }
        SendMessageW(conflict, CB_SETCURSEL, 0, 0);

        let state = Box::new(ExportDialogState {
            main,
            dir_label,
            dir,
            browse,
            name_label,
            name,
            hint,
            structure,
            open_folder,
            conflict_label,
            conflict,
            ok,
            cancel,
            entries,
            single,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_export(dialog);

        run_modal(dialog, owner, dir);
    }

    unsafe fn layout_export(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ExportDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(16, dpi);
        let label_w = scale(80, dpi);
        let field_x = margin + label_w;
        let field_w = client.right - field_x - margin;
        let button_h = scale(30, dpi);
        let browse_w = scale(88, dpi);

        place(state.dir_label, margin, scale(20, dpi), label_w, button_h);
        place(
            state.dir,
            field_x,
            scale(20, dpi),
            field_w - browse_w - scale(8, dpi),
            button_h,
        );
        place(
            state.browse,
            client.right - margin - browse_w,
            scale(20, dpi),
            browse_w,
            button_h,
        );
        place(state.name_label, margin, scale(62, dpi), label_w, button_h);
        place(state.name, field_x, scale(62, dpi), field_w, button_h);
        place(
            state.hint,
            field_x,
            scale(98, dpi),
            field_w,
            scale(22, dpi),
        );
        place(
            state.structure,
            field_x,
            scale(128, dpi),
            scale(220, dpi),
            scale(24, dpi),
        );
        place(
            state.open_folder,
            field_x,
            scale(158, dpi),
            scale(240, dpi),
            scale(24, dpi),
        );
        place(
            state.conflict_label,
            margin,
            scale(200, dpi),
            label_w + scale(24, dpi),
            button_h,
        );
        place(
            state.conflict,
            field_x + scale(24, dpi),
            scale(200, dpi),
            scale(160, dpi),
            scale(120, dpi),
        );

        let footer_y = client.bottom - margin - button_h;
        let cancel_w = scale(100, dpi);
        let ok_w = scale(110, dpi);
        place(
            state.cancel,
            client.right - margin - cancel_w,
            footer_y,
            cancel_w,
            button_h,
        );
        place(
            state.ok,
            client.right - margin - cancel_w - scale(8, dpi) - ok_w,
            footer_y,
            ok_w,
            button_h,
        );
    }

    unsafe extern "system" fn export_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_export(hwnd);
                0
            }
            WM_CTLCOLORSTATIC => dialog_color(wparam),
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                if code == BN_CLICKED {
                    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ExportDialogState;
                    if !pointer.is_null() {
                        match id as usize {
                            EXPORT_BROWSE_ID => {
                                let dir = (*pointer).dir;
                                if let Some(path) = browse_for_folder(hwnd, "选择导出目录") {
                                    set_text(dir, &path.to_string_lossy());
                                }
                                return 0;
                            }
                            EXPORT_OK_ID => {
                                handle_export_confirm(hwnd);
                                return 0;
                            }
                            EXPORT_CANCEL_ID => {
                                DestroyWindow(hwnd);
                                return 0;
                            }
                            _ => {}
                        }
                    }
                }
                if id == IDCANCEL as i32 {
                    DestroyWindow(hwnd);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                let pointer = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut ExportDialogState;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn browse_for_folder(owner: HWND, caption: &str) -> Option<PathBuf> {
        let mut display = [0u16; 1024];
        let title = wide(caption);
        let mut info: BROWSEINFOW = zeroed();
        info.hwndOwner = owner;
        info.pszDisplayName = display.as_mut_ptr();
        info.lpszTitle = title.as_ptr();
        info.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
        let id_list = SHBrowseForFolderW(&info);
        if id_list.is_null() {
            return None;
        }
        let mut path = [0u16; 1024];
        let ok = SHGetPathFromIDListW(id_list, path.as_mut_ptr()) != 0;
        ILFree(id_list);
        if !ok {
            return None;
        }
        let length = path.iter().position(|&value| value == 0).unwrap_or(path.len());
        Some(PathBuf::from(String::from_utf16_lossy(&path[..length])))
    }

    unsafe fn handle_export_confirm(dialog: HWND) {
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut ExportDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let dir_text = read_text(state.dir).trim().to_string();
        if dir_text.is_empty() {
            show_message(dialog, "请选择导出目录。", MB_OK | MB_ICONWARNING);
            SetFocus(state.dir);
            return;
        }
        let raw_dir = PathBuf::from(&dir_text);
        let target_dir = std::path::absolute(&raw_dir).unwrap_or(raw_dir);
        if let Err(error) = std::fs::create_dir_all(&target_dir) {
            show_message(
                dialog,
                &format!("无法创建导出目录：\n{error}"),
                MB_OK | MB_ICONERROR,
            );
            return;
        }
        let probe = target_dir.join(".wd-pak-write-check.tmp");
        if let Err(error) = std::fs::write(&probe, b"") {
            show_message(
                dialog,
                &format!("目标目录不可写：\n{error}"),
                MB_OK | MB_ICONERROR,
            );
            return;
        }
        let _ = std::fs::remove_file(&probe);

        let plan = ExportPlan::new(state.entries.clone(), target_dir.clone(), is_checked(state.structure));
        let mut items = Vec::new();
        if state.single {
            let file_name = read_text(state.name).trim().to_string();
            if file_name.is_empty() {
                show_message(dialog, "请输入导出文件名。", MB_OK | MB_ICONWARNING);
                SetFocus(state.name);
                return;
            }
            if file_name.chars().any(|value| "\\/:*?\"<>|".contains(value))
                || file_name.ends_with('.')
                || file_name.ends_with(' ')
            {
                show_message(
                    dialog,
                    "文件名不能包含 \\ / : * ? \" < > |，也不能以空格或点结尾。",
                    MB_OK | MB_ICONWARNING,
                );
                SetFocus(state.name);
                return;
            }
            items.push(ExportItem {
                inner: state.entries[0].clone(),
                target: target_dir.join(&file_name),
                skip: false,
                replace: false,
            });
        } else {
            for inner in &state.entries {
                items.push(ExportItem {
                    inner: inner.clone(),
                    target: plan.target_path(inner),
                    skip: false,
                    replace: false,
                });
            }
        }

        let (pak_path, sizes) = {
            let ui = &*(GetWindowLongPtrW(state.main, GWLP_USERDATA) as *const Ui);
            let Some(session) = ui.session.as_ref() else {
                show_message(dialog, "当前标签已关闭，无法导出。", MB_OK | MB_ICONWARNING);
                return;
            };
            let sizes: std::collections::HashMap<String, u64> = session
                .entries()
                .iter()
                .map(|entry| (entry.path.clone(), entry.org as u64))
                .collect();
            (ui.path.clone(), sizes)
        };
        let Some(pak_path) = pak_path else {
            show_message(dialog, "当前标签没有可用的包路径。", MB_OK | MB_ICONWARNING);
            return;
        };

        let open_folder = is_checked(state.open_folder);
        let choice = SendMessageW(state.conflict, CB_GETCURSEL, 0, 0);
        let preset = match choice {
            1 => Some(ExportConflict::Replace),
            2 => Some(ExportConflict::Skip),
            3 => Some(ExportConflict::Rename),
            _ => None,
        };
        let mut forced = preset;
        let mut taken: HashSet<String> = HashSet::new();
        for item in items.iter_mut() {
            let key = wd_pak::gui::export::plan_key(&item.target);
            let duplicate = taken.contains(&key);
            if !item.target.exists() && !duplicate {
                taken.insert(key);
                continue;
            }
            let decision = if let Some(forced) = forced {
                forced
            } else {
                let (decision, apply_all) =
                    show_conflict_dialog(dialog, &item.target.to_string_lossy());
                if decision == ExportConflict::Pending {
                    return;
                }
                if apply_all {
                    forced = Some(decision);
                }
                decision
            };
            match decision {
                ExportConflict::Replace => {
                    item.replace = true;
                    taken.insert(key);
                }
                ExportConflict::Skip => {
                    item.skip = true;
                }
                ExportConflict::Rename => {
                    item.target = unique_target(&item.target, &mut taken);
                }
                ExportConflict::Pending => return,
            }
        }

        if items.iter().all(|item| item.skip) {
            show_message(
                dialog,
                "所有条目都已跳过，没有写入文件。",
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        }
        let needed: u64 = items
            .iter()
            .filter(|item| !item.skip && !item.replace)
            .map(|item| sizes.get(&item.inner).copied().unwrap_or(0))
            .sum();
        if needed > 0 {
            let dir_wide = wide(&target_dir.to_string_lossy());
            let mut free = 0u64;
            let mut total = 0u64;
            if GetDiskFreeSpaceExW(dir_wide.as_ptr(), &mut free, &mut total, null_mut()) != 0
                && free < needed
            {
                show_message(
                    dialog,
                    &format!(
                        "目标磁盘可用空间不足：\n需要 {needed} 字节，可用 {free} 字节。"
                    ),
                    MB_OK | MB_ICONWARNING,
                );
                return;
            }
        }

        {
            let ui = &mut *(GetWindowLongPtrW(state.main, GWLP_USERDATA) as *mut Ui);
            ui.config.last_export_dir = Some(target_dir.to_string_lossy().into_owned());
            persist_config(ui);
        }

        let outcome = run_export(dialog, &pak_path, items.clone());
        let (summary, flags) = export_summary(&items, &outcome);
        show_message(dialog, &summary, flags);
        if open_folder && outcome.written > 0 && !outcome.cancelled {
            reveal_folder(&target_dir);
        }
        DestroyWindow(dialog);
    }

    fn export_summary(items: &[ExportItem], outcome: &ExportOutcome) -> (String, u32) {
        if let Some(fatal) = &outcome.fatal {
            return (format!("导出失败：{fatal}"), MB_OK | MB_ICONERROR);
        }
        let single = items.len() == 1 && !items[0].skip;
        if single && outcome.failures.is_empty() {
            if outcome.cancelled {
                return ("导出已取消，未写入文件。".into(), MB_OK | MB_ICONINFORMATION);
            }
            let target = items[0].target.to_string_lossy();
            return (
                format!("已导出 {} 字节 -> {target}", outcome.bytes),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        if single && outcome.written == 0 {
            if let Some((_, reason)) = outcome.failures.first() {
                return (format!("导出失败：{reason}"), MB_OK | MB_ICONERROR);
            }
        }
        let mut message = String::new();
        if outcome.cancelled {
            message.push_str("导出已取消。\n");
        }
        message.push_str(&format!(
            "已导出 {} 项（{} 字节）",
            outcome.written, outcome.bytes
        ));
        if outcome.skipped > 0 {
            message.push_str(&format!("，跳过 {} 项", outcome.skipped));
        }
        if !outcome.failures.is_empty() {
            message.push_str(&format!("，失败 {} 项", outcome.failures.len()));
            message.push_str("\n\n失败明细：");
            for (inner, reason) in outcome.failures.iter().take(5) {
                message.push_str(&format!("\n{inner}: {reason}"));
            }
            if outcome.failures.len() > 5 {
                message.push_str(&format!("\n…另有 {} 项", outcome.failures.len() - 5));
            }
        }
        let flags = if outcome.cancelled || !outcome.failures.is_empty() {
            MB_OK | MB_ICONWARNING
        } else {
            MB_OK | MB_ICONINFORMATION
        };
        (message, flags)
    }

    unsafe fn reveal_folder(path: &Path) {
        let operation = wide("open");
        let target = wide(&path.to_string_lossy());
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        );
    }

    struct ConflictState {
        decision: ExportConflict,
        apply_all: bool,
        all: HWND,
        label: HWND,
        path: HWND,
        replace: HWND,
        skip: HWND,
        rename: HWND,
        cancel: HWND,
        result: *mut (ExportConflict, bool),
    }

    unsafe fn show_conflict_dialog(owner: HWND, target: &str) -> (ExportConflict, bool) {
        let mut result = (ExportConflict::Pending, false);
        let (class_name, instance) =
            register_dialog_class(CONFLICT_CLASS, Some(conflict_wndproc), COLOR_BTNFACE);

        let dpi = dpi_of(owner);
        let width = scale(660, dpi);
        let height = scale(250, dpi);
        let dialog = create_centered_dialog(
            owner,
            &class_name,
            instance,
            "同名文件",
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            width,
            height,
        );
        if dialog.is_null() {
            return result;
        }

        let label = create_label(dialog, instance, "目标文件已存在：");
        let path = create_label(dialog, instance, target);
        let all = create_button(
            dialog,
            instance,
            "对后续冲突使用相同选择",
            BS_AUTOCHECKBOX as u32,
            CONFLICT_ALL_ID,
        );
        let replace = create_button(dialog, instance, "替换", 0, CONFLICT_REPLACE_ID);
        let skip = create_button(
            dialog,
            instance,
            "跳过",
            BS_DEFPUSHBUTTON as u32,
            CONFLICT_SKIP_ID,
        );
        let rename = create_button(dialog, instance, "自动改名", 0, CONFLICT_RENAME_ID);
        let cancel = create_button(dialog, instance, "取消导出", 0, CONFLICT_CANCEL_ID);
        if !all_present(&[label, path, all, replace, skip, rename, cancel]) {
            DestroyWindow(dialog);
            return result;
        }
        apply_default_font(&[label, path, all, replace, skip, rename, cancel]);

        let state = Box::new(ConflictState {
            decision: ExportConflict::Pending,
            apply_all: false,
            all,
            label,
            path,
            replace,
            skip,
            rename,
            cancel,
            result: &mut result,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_conflict(dialog);

        run_modal(dialog, owner, skip);
        result
    }

    unsafe fn layout_conflict(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ConflictState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(16, dpi);
        let button_h = scale(30, dpi);
        let button_w = scale(112, dpi);
        let width = client.right - margin * 2;
        place(state.label, margin, scale(16, dpi), width, scale(24, dpi));
        place(state.path, margin, scale(48, dpi), width, scale(52, dpi));
        place(state.all, margin, scale(108, dpi), scale(260, dpi), scale(24, dpi));
        let footer_y = client.bottom - margin - button_h;
        place(state.replace, margin, footer_y, button_w, button_h);
        place(
            state.skip,
            margin + button_w + scale(8, dpi),
            footer_y,
            button_w,
            button_h,
        );
        place(
            state.rename,
            margin + (button_w + scale(8, dpi)) * 2,
            footer_y,
            button_w,
            button_h,
        );
        place(
            state.cancel,
            client.right - margin - scale(110, dpi),
            footer_y,
            scale(110, dpi),
            button_h,
        );
    }

    unsafe extern "system" fn conflict_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_conflict(hwnd);
                0
            }
            WM_CTLCOLORSTATIC => dialog_color(wparam),
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                if code == BN_CLICKED {
                    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ConflictState;
                    if !pointer.is_null() {
                        let decision = match id as usize {
                            CONFLICT_REPLACE_ID => Some(ExportConflict::Replace),
                            CONFLICT_SKIP_ID => Some(ExportConflict::Skip),
                            CONFLICT_RENAME_ID => Some(ExportConflict::Rename),
                            _ => None,
                        };
                        if let Some(decision) = decision {
                            let state = &mut *pointer;
                            state.decision = decision;
                            state.apply_all = is_checked((*pointer).all);
                            *state.result = (decision, state.apply_all);
                            DestroyWindow(hwnd);
                            return 0;
                        }
                        if id as usize == CONFLICT_CANCEL_ID {
                            DestroyWindow(hwnd);
                            return 0;
                        }
                    }
                }
                if id == IDCANCEL as i32 {
                    DestroyWindow(hwnd);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                let pointer = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut ConflictState;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    struct ExportProgressState {
        label: HWND,
        bar: HWND,
        cancel: HWND,
        handle: Option<JoinHandle<ExportOutcome>>,
        progress: Arc<AtomicUsize>,
        cancel_flag: Arc<AtomicBool>,
        total: usize,
        outcome: *mut Option<ExportOutcome>,
        cancel_requested: bool,
    }

    fn export_temp_path(target: &Path, index: usize) -> PathBuf {
        let name = target
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "export".into());
        target.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), index))
    }

    unsafe fn run_export(parent: HWND, pak_path: &Path, items: Vec<ExportItem>) -> ExportOutcome {
        let mut outcome: Option<ExportOutcome> = None;
        let (class_name, instance) = register_dialog_class(
            EXPORT_PROGRESS_CLASS,
            Some(export_progress_wndproc),
            COLOR_BTNFACE,
        );

        let dpi = dpi_of(parent);
        let width = scale(460, dpi);
        let height = scale(170, dpi);
        let dialog = create_centered_dialog(
            parent,
            &class_name,
            instance,
            "正在导出",
            WS_EX_DLGMODALFRAME,
            width,
            height,
        );
        if dialog.is_null() {
            return ExportOutcome {
                fatal: Some("无法创建导出进度窗口。".into()),
                ..Default::default()
            };
        }

        let label = create_label(dialog, instance, "正在准备导出…");
        let bar = create_progress(dialog, instance, 0);
        let cancel = create_button(dialog, instance, "取消", 0, CANCEL_BUTTON_ID);
        if !all_present(&[label, bar, cancel]) {
            DestroyWindow(dialog);
            return ExportOutcome {
                fatal: Some("无法创建导出进度控件。".into()),
                ..Default::default()
            };
        }
        apply_default_font(&[label, cancel]);

        let total = items.len();
        let progress = Arc::new(AtomicUsize::new(0));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let handle = {
            let progress = progress.clone();
            let cancel_flag = cancel_flag.clone();
            let pak_path = pak_path.to_path_buf();
            std::thread::spawn(move || {
                let mut outcome = ExportOutcome::default();
                let session = match PakSession::open(&pak_path) {
                    Ok(session) => session,
                    Err(error) => {
                        outcome.fatal = Some(error.to_string());
                        return outcome;
                    }
                };
                for (index, item) in items.iter().enumerate() {
                    if cancel_flag.load(Ordering::Relaxed) {
                        outcome.cancelled = true;
                        break;
                    }
                    if item.skip {
                        outcome.skipped += 1;
                        progress.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let data = match session.read(&item.inner) {
                        Ok(data) => data,
                        Err(error) => {
                            outcome.failures.push((item.inner.clone(), error.to_string()));
                            progress.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    };
                    if let Some(parent) = item.target.parent() {
                        if let Err(error) = std::fs::create_dir_all(parent) {
                            outcome
                                .failures
                                .push((item.inner.clone(), format!("创建目录失败：{error}")));
                            progress.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    }
                    let temp = export_temp_path(&item.target, index);
                    if let Err(error) = std::fs::write(&temp, &data) {
                        outcome
                            .failures
                            .push((item.inner.clone(), format!("写入失败：{error}")));
                        let _ = std::fs::remove_file(&temp);
                        progress.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if cancel_flag.load(Ordering::Relaxed) {
                        let _ = std::fs::remove_file(&temp);
                        outcome.cancelled = true;
                        break;
                    }
                    if item.replace {
                        let _ = std::fs::remove_file(&item.target);
                    }
                    match std::fs::rename(&temp, &item.target) {
                        Ok(()) => {
                            outcome.written += 1;
                            outcome.bytes += data.len() as u64;
                        }
                        Err(error) => {
                            outcome
                                .failures
                                .push((item.inner.clone(), format!("替换目标失败：{error}")));
                            let _ = std::fs::remove_file(&temp);
                        }
                    }
                    progress.fetch_add(1, Ordering::Relaxed);
                }
                outcome
            })
        };

        let state = Box::new(ExportProgressState {
            label,
            bar,
            cancel,
            handle: Some(handle),
            progress,
            cancel_flag,
            total,
            outcome: &mut outcome,
            cancel_requested: false,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_export_progress(dialog);
        SendMessageW(bar, PBM_SETRANGE32, 0, total as isize);
        SendMessageW(bar, PBM_SETPOS, 0, 0);
        SetTimer(dialog, EXPORT_PROGRESS_TIMER_ID, 80, None);

        run_modal(dialog, parent, cancel);
        outcome.unwrap_or_else(|| ExportOutcome {
            fatal: Some("导出被中断。".into()),
            ..Default::default()
        })
    }

    unsafe fn layout_export_progress(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ExportProgressState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(16, dpi);
        let width = client.right - margin * 2;
        let button_h = scale(32, dpi);
        place(state.label, margin, scale(14, dpi), width, scale(24, dpi));
        place(state.bar, margin, scale(48, dpi), width, scale(18, dpi));
        place(
            state.cancel,
            client.right - margin - scale(100, dpi),
            client.bottom - margin - button_h,
            scale(100, dpi),
            button_h,
        );
    }

    unsafe fn request_export_cancel(state: &mut ExportProgressState) {
        state.cancel_flag.store(true, Ordering::Relaxed);
        state.cancel_requested = true;
        EnableWindow(state.cancel, 0);
        set_text(state.label, "正在取消…");
    }

    unsafe extern "system" fn export_progress_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_export_progress(hwnd);
                0
            }
            WM_TIMER => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ExportProgressState;
                if !pointer.is_null()
                    && wparam == EXPORT_PROGRESS_TIMER_ID
                {
                    let state = &mut *pointer;
                    let processed = state.progress.load(Ordering::Relaxed).min(state.total);
                    SendMessageW(state.bar, PBM_SETPOS, processed, 0);
                    if !state.cancel_requested {
                        let text = format!("正在导出…（{processed} / {}）", state.total);
                        set_text(state.label, &text);
                    }
                    let done = state
                        .handle
                        .as_ref()
                        .map(|handle| handle.is_finished())
                        .unwrap_or(true);
                    if done {
                        if let Some(handle) = state.handle.take() {
                            let result = handle.join().unwrap_or_else(|_| ExportOutcome {
                                fatal: Some("导出线程异常终止。".into()),
                                ..Default::default()
                            });
                            *state.outcome = Some(result);
                        }
                        KillTimer(hwnd, EXPORT_PROGRESS_TIMER_ID);
                        DestroyWindow(hwnd);
                    }
                }
                0
            }
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                let pointer =
                    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ExportProgressState;
                if !pointer.is_null()
                    && code == BN_CLICKED
                    && id == CANCEL_BUTTON_ID as i32
                {
                    request_export_cancel(&mut *pointer);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ExportProgressState;
                if !pointer.is_null() {
                    request_export_cancel(&mut *pointer);
                }
                0
            }
            WM_DESTROY => {
                KillTimer(hwnd, EXPORT_PROGRESS_TIMER_ID);
                0
            }
            WM_NCDESTROY => {
                let pointer =
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut ExportProgressState;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    // -----------------------------------------------------------------------
    // G4 写入链路：替换 / 打包 / 解包
    // -----------------------------------------------------------------------

    unsafe fn pick_file_dialog(
        owner: HWND,
        title: &str,
        filter: &str,
        initial_dir: Option<&Path>,
    ) -> Option<PathBuf> {
        let filter_wide: Vec<u16> = filter.encode_utf16().chain(std::iter::once(0)).collect();
        let title_wide = wide(title);
        let initial = initial_dir.map(|path| wide(&path.to_string_lossy()));
        let mut file = [0u16; 1024];
        let mut ofn: OPENFILENAMEW = zeroed();
        ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filter_wide.as_ptr();
        ofn.lpstrFile = file.as_mut_ptr();
        ofn.nMaxFile = file.len() as u32;
        ofn.lpstrTitle = title_wide.as_ptr();
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

    unsafe fn pick_save_dialog(
        owner: HWND,
        title: &str,
        filter: &str,
        initial_dir: Option<&Path>,
        default_name: &str,
    ) -> Option<PathBuf> {
        let filter_wide: Vec<u16> = filter.encode_utf16().chain(std::iter::once(0)).collect();
        let title_wide = wide(title);
        let initial = initial_dir.map(|path| wide(&path.to_string_lossy()));
        let mut file = [0u16; 1024];
        for (slot, value) in file
            .iter_mut()
            .zip(default_name.encode_utf16().chain(std::iter::once(0)))
        {
            *slot = value;
        }
        let mut ofn: OPENFILENAMEW = zeroed();
        ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filter_wide.as_ptr();
        ofn.lpstrFile = file.as_mut_ptr();
        ofn.nMaxFile = file.len() as u32;
        ofn.lpstrTitle = title_wide.as_ptr();
        if let Some(initial) = &initial {
            ofn.lpstrInitialDir = initial.as_ptr();
        }
        ofn.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
        if GetSaveFileNameW(&mut ofn) == 0 {
            return None;
        }
        let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
        Some(PathBuf::from(String::from_utf16_lossy(&file[..len])))
    }

    fn copy_with_mtime(from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::copy(from, to)?;
        let modified = std::fs::metadata(from)?.modified()?;
        let file = std::fs::OpenOptions::new().write(true).open(to)?;
        file.set_times(std::fs::FileTimes::new().set_modified(modified))?;
        Ok(())
    }

    fn strip_hash(text: &str) -> String {
        text.lines()
            .map(|line| line.strip_prefix("# ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[derive(Debug, Clone, Default)]
    struct WriteOutcome {
        ok: bool,
        cancelled: bool,
        message: String,
    }

    impl WriteOutcome {
        fn success(message: impl Into<String>) -> Self {
            Self {
                ok: true,
                cancelled: false,
                message: message.into(),
            }
        }

        fn failure(message: impl Into<String>) -> Self {
            Self {
                ok: false,
                cancelled: false,
                message: message.into(),
            }
        }
    }

    struct WriteProgressState {
        label: HWND,
        stage_label: HWND,
        bar: HWND,
        cancel: HWND,
        handle: Option<JoinHandle<WriteOutcome>>,
        progress: Arc<AtomicUsize>,
        stage: Arc<AtomicUsize>,
        cancel_flag: Arc<AtomicBool>,
        total: usize,
        steps: Vec<String>,
        verb: String,
        cancel_enabled: bool,
        cancel_requested: bool,
        outcome: *mut Option<WriteOutcome>,
    }

    unsafe fn layout_write_progress(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WriteProgressState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(16, dpi);
        let width = client.right - margin * 2;
        let button_h = scale(30, dpi);
        place(state.label, margin, scale(14, dpi), width, scale(24, dpi));
        place(state.stage_label, margin, scale(40, dpi), width, scale(24, dpi));
        place(state.bar, margin, scale(72, dpi), width, scale(18, dpi));
        place(
            state.cancel,
            client.right - margin - scale(120, dpi),
            client.bottom - margin - button_h,
            scale(120, dpi),
            button_h,
        );
    }

    unsafe fn request_write_cancel(state: &mut WriteProgressState) {
        if !state.cancel_enabled || state.cancel_requested {
            return;
        }
        state.cancel_flag.store(true, Ordering::Relaxed);
        state.cancel_requested = true;
        EnableWindow(state.cancel, 0);
        set_text(state.label, "正在取消…");
    }

    unsafe extern "system" fn write_progress_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_write_progress(hwnd);
                0
            }
            WM_TIMER => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WriteProgressState;
                if !pointer.is_null() && wparam == WRITE_TIMER_ID {
                    let state = &mut *pointer;
                    let processed = state.progress.load(Ordering::Relaxed);
                    if state.total > 0 {
                        let shown = processed.min(state.total);
                        SendMessageW(state.bar, PBM_SETPOS, shown, 0);
                        if !state.cancel_requested {
                            let text = format!(
                                "正在{}…（{} / {}）",
                                state.verb, shown, state.total
                            );
                            set_text(state.label, &text);
                        }
                    } else if !state.cancel_requested {
                        let text = format!("正在{}…", state.verb);
                        set_text(state.label, &text);
                    }
                    if !state.steps.is_empty() {
                        let index = state
                            .stage
                            .load(Ordering::Relaxed)
                            .min(state.steps.len().saturating_sub(1));
                        let text = format!(
                            "步骤 {}/{}：{}",
                            index + 1,
                            state.steps.len(),
                            state.steps[index]
                        );
                        set_text(state.stage_label, &text);
                    }
                    let done = state
                        .handle
                        .as_ref()
                        .map(|handle| handle.is_finished())
                        .unwrap_or(true);
                    if done {
                        if let Some(handle) = state.handle.take() {
                            let result = handle.join().unwrap_or_else(|_| {
                                WriteOutcome::failure("写入线程异常终止。")
                            });
                            *state.outcome = Some(result);
                        }
                        KillTimer(hwnd, WRITE_TIMER_ID);
                        DestroyWindow(hwnd);
                    }
                }
                0
            }
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WriteProgressState;
                if !pointer.is_null() && code == BN_CLICKED && id == CANCEL_BUTTON_ID as i32 {
                    request_write_cancel(&mut *pointer);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WriteProgressState;
                if !pointer.is_null() {
                    request_write_cancel(&mut *pointer);
                }
                0
            }
            WM_DESTROY => {
                KillTimer(hwnd, WRITE_TIMER_ID);
                0
            }
            WM_NCDESTROY => {
                let pointer =
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut WriteProgressState;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn run_write_progress<F>(
        parent: HWND,
        verb: &str,
        total: usize,
        steps: Vec<String>,
        cancel_enabled: bool,
        work: F,
    ) -> WriteOutcome
    where
        F: FnOnce(Arc<AtomicUsize>, Arc<AtomicBool>, Arc<AtomicUsize>) -> WriteOutcome
            + Send
            + 'static,
    {
        let mut outcome: Option<WriteOutcome> = None;
        let (class_name, instance) = register_dialog_class(
            WRITE_PROGRESS_CLASS,
            Some(write_progress_wndproc),
            COLOR_BTNFACE,
        );

        let dpi = dpi_of(parent);
        let width = scale(500, dpi);
        let height = scale(230, dpi);
        let dialog = create_centered_dialog(
            parent,
            &class_name,
            instance,
            &format!("正在{verb}"),
            WS_EX_DLGMODALFRAME,
            width,
            height,
        );
        if dialog.is_null() {
            return WriteOutcome::failure("无法创建写入进度窗口。");
        }
        let label = create_label(dialog, instance, "正在准备…");
        let stage_label = create_label(dialog, instance, "");
        let bar = create_progress(dialog, instance, if total == 0 { PBS_MARQUEE } else { 0 });
        let cancel_text = if cancel_enabled {
            "取消"
        } else {
            "写入阶段不可取消"
        };
        let cancel = create_button(dialog, instance, cancel_text, 0, CANCEL_BUTTON_ID);
        if !all_present(&[label, stage_label, bar, cancel]) {
            DestroyWindow(dialog);
            return WriteOutcome::failure("无法创建写入进度控件。");
        }
        if !cancel_enabled {
            EnableWindow(cancel, 0);
        }
        apply_default_font(&[label, stage_label, cancel]);

        let progress = Arc::new(AtomicUsize::new(0));
        let stage = Arc::new(AtomicUsize::new(0));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let handle = {
            let progress = progress.clone();
            let stage = stage.clone();
            let cancel_flag = cancel_flag.clone();
            std::thread::spawn(move || work(progress, cancel_flag, stage))
        };

        let state = Box::new(WriteProgressState {
            label,
            stage_label,
            bar,
            cancel,
            handle: Some(handle),
            progress,
            stage,
            cancel_flag,
            total,
            steps,
            verb: verb.to_string(),
            cancel_enabled,
            cancel_requested: false,
            outcome: &mut outcome,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_write_progress(dialog);
        if total > 0 {
            SendMessageW(bar, PBM_SETRANGE32, 0, total as isize);
            SendMessageW(bar, PBM_SETPOS, 0, 0);
        } else {
            SendMessageW(bar, PBM_SETMARQUEE, 1, 30);
        }
        SetTimer(dialog, WRITE_TIMER_ID, 80, None);

        run_modal(dialog, parent, cancel);
        outcome.unwrap_or_else(|| WriteOutcome::failure("写入被中断。"))
    }

    // -----------------------------------------------------------------------
    // G4-01/02 替换向导
    // -----------------------------------------------------------------------

    struct ReplaceDialogState {
        main: HWND,
        pak_path: PathBuf,
        inner: String,
        old_org: u64,
        path_label: HWND,
        info_label: HWND,
        file_label: HWND,
        file_edit: HWND,
        browse: HWND,
        delta: HWND,
        compress: HWND,
        raw: HWND,
        hint: HWND,
        backup: HWND,
        backup_label: HWND,
        risk: HWND,
        ok: HWND,
        cancel: HWND,
        success: *mut bool,
    }

    unsafe fn layout_replace(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ReplaceDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(18, dpi);
        let width = client.right - margin * 2;
        let edit_h = scale(26, dpi);
        let button_w = scale(96, dpi);
        let button_h = scale(32, dpi);
        let mut y = scale(14, dpi);
        place(state.path_label, margin, y, width, scale(22, dpi));
        y += scale(26, dpi);
        place(state.info_label, margin, y, width, scale(22, dpi));
        y += scale(38, dpi);
        place(state.file_label, margin, y, width, scale(20, dpi));
        y += scale(22, dpi);
        place(
            state.file_edit,
            margin,
            y,
            width - button_w - scale(8, dpi),
            edit_h,
        );
        place(
            state.browse,
            client.right - margin - button_w,
            y,
            button_w,
            edit_h,
        );
        y += scale(32, dpi);
        place(state.delta, margin, y, width, scale(22, dpi));
        y += scale(34, dpi);
        place(state.compress, margin, y, scale(210, dpi), scale(24, dpi));
        place(
            state.raw,
            margin + scale(220, dpi),
            y,
            scale(210, dpi),
            scale(24, dpi),
        );
        y += scale(26, dpi);
        place(state.hint, margin, y, width, scale(22, dpi));
        y += scale(32, dpi);
        place(state.backup, margin, y, width, scale(24, dpi));
        y += scale(24, dpi);
        place(state.backup_label, margin, y, width, scale(22, dpi));
        y += scale(30, dpi);
        place(state.risk, margin, y, width, scale(56, dpi));
        let bottom = client.bottom - margin - button_h;
        place(
            state.ok,
            client.right - margin - button_w,
            bottom,
            button_w,
            button_h,
        );
        place(
            state.cancel,
            client.right - margin - button_w * 2 - scale(10, dpi),
            bottom,
            button_w,
            button_h,
        );
    }

    unsafe fn update_replace_state(dialog: HWND) {
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *const ReplaceDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let text = read_text(state.file_edit).trim().to_string();
        let mut delta = "请选择新文件。".to_string();
        let mut risk = String::new();
        let mut valid = false;
        if !text.is_empty() {
            let path = PathBuf::from(&text);
            match std::fs::metadata(&path) {
                Ok(meta) if meta.is_file() => {
                    let size = meta.len();
                    let diff = size as i128 - state.old_org as i128;
                    delta = format!(
                        "大小变化：{}{} 字节（当前 {} → 新 {}）",
                        if diff >= 0 { "+" } else { "" },
                        diff,
                        state.old_org,
                        size
                    );
                    if size > u64::from(u32::MAX) {
                        risk.push_str("新文件超过 4GB 上限，无法替换。\n");
                    } else {
                        valid = true;
                    }
                }
                _ => delta = "新文件不存在或不是普通文件。".into(),
            }
        }
        let bak = backup_path(&state.pak_path);
        if bak.exists() {
            risk.push_str(&format!("同名备份将被覆盖：{}\n", bak.display()));
        }
        if state.old_org > u64::from(u32::MAX) {
            risk.push_str("当前条目超过 4GB 上限。\n");
            valid = false;
        }
        if risk.is_empty() {
            risk.push_str("写入完成后不可撤销，只能依赖 .bak 人工回退。");
        }
        set_text(state.delta, &delta);
        set_text(state.risk, &risk);
        EnableWindow(state.ok, valid as i32);
    }

    unsafe fn handle_replace_confirm(dialog: HWND) {
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut ReplaceDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &mut *pointer;
        let text = read_text(state.file_edit).trim().to_string();
        if text.is_empty() {
            show_message(dialog, "请选择新文件。", MB_OK | MB_ICONWARNING);
            SetFocus(state.file_edit);
            return;
        }
        let newfile = PathBuf::from(&text);
        let meta = match std::fs::metadata(&newfile) {
            Ok(meta) if meta.is_file() => meta,
            _ => {
                show_message(dialog, "新文件不存在或不是普通文件。", MB_OK | MB_ICONWARNING);
                SetFocus(state.file_edit);
                return;
            }
        };
        let writable = std::fs::OpenOptions::new()
            .write(true)
            .open(&state.pak_path)
            .is_ok();
        let entry_exists = {
            let main_pointer = GetWindowLongPtrW(state.main, GWLP_USERDATA) as *const Ui;
            if main_pointer.is_null() {
                false
            } else {
                (*main_pointer)
                    .session
                    .as_ref()
                    .map(|session| {
                        session
                            .entries()
                            .iter()
                            .any(|entry| entry.path == state.inner && !entry.is_dir())
                    })
                    .unwrap_or(false)
            }
        };
        if !entry_exists {
            let text = "替换只能修改已有条目；新增文件需先解包、放入文件、再打包。\n\n是否打开解包向导？";
            let answer = MessageBoxW(
                dialog,
                wide(text).as_ptr(),
                wide("包内不存在").as_ptr(),
                MB_YESNO | MB_ICONQUESTION,
            );
            if answer == 6 {
                open_unpack_dialog(state.main);
            }
            return;
        }
        let precheck = replace_precheck(
            true,
            state.old_org,
            meta.len(),
            writable,
            u64::from(u32::MAX),
        );
        if !precheck.can_start() {
            let mut text = String::new();
            for issue in &precheck.issues {
                let line = match issue.kind {
                    ReplaceIssueKind::TargetMissing => "包内条目不存在。",
                    ReplaceIssueKind::TargetNotWritable => {
                        "目标文件被其他程序占用或不可写。"
                    }
                    ReplaceIssueKind::NewFileTooLarge | ReplaceIssueKind::SizeLimit => {
                        "新文件超过 4GB 上限。"
                    }
                    ReplaceIssueKind::NameInvalid => "包内路径非法。",
                };
                text.push_str(line);
                text.push('\n');
            }
            show_message(dialog, text.trim_end(), MB_OK | MB_ICONERROR);
            return;
        }
        let compress = is_checked(state.compress);
        let backup = is_checked(state.backup);
        let pak_path = state.pak_path.clone();
        let inner = state.inner.clone();
        let newfile = newfile.clone();
        let bak = backup_path(&pak_path);
        let outcome = run_write_progress(
            dialog,
            "替换",
            0,
            vec!["备份".into(), "写入".into(), "完成".into()],
            false,
            move |_progress, _cancel, stage| {
                let mut log = String::new();
                if backup {
                    stage.store(0, Ordering::Relaxed);
                    if let Err(error) = copy_with_mtime(&pak_path, &bak) {
                        return WriteOutcome::failure(format!(
                            "备份失败：{error}\n原包未修改。"
                        ));
                    }
                    log.push_str(&format!("已备份: {}\n", bak.display()));
                }
                stage.store(1, Ordering::Relaxed);
                let mut err = Vec::new();
                match cmd_replace(&pak_path, &inner, &newfile, compress, false, &mut err) {
                    Ok(()) => {
                        stage.store(2, Ordering::Relaxed);
                        let cli = strip_hash(&String::from_utf8_lossy(&err));
                        log.push_str(cli.trim_end());
                        WriteOutcome::success(log.trim_end().to_string())
                    }
                    Err(error) => {
                        let suffix = if backup {
                            format!("\n备份已保留：{}", bak.display())
                        } else {
                            String::new()
                        };
                        WriteOutcome::failure(format!(
                            "{error}\n原包保持不变。{suffix}"
                        ))
                    }
                }
            },
        );
        if outcome.ok {
            *state.success = true;
            show_message(dialog, &outcome.message, MB_OK | MB_ICONINFORMATION);
            DestroyWindow(dialog);
        } else {
            show_message(
                dialog,
                &format!("替换失败：{}", outcome.message),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    unsafe extern "system" fn replace_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_replace(hwnd);
                0
            }
            WM_CTLCOLORSTATIC => dialog_color(wparam),
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                let pointer =
                    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ReplaceDialogState;
                if pointer.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                if id == REPLACE_BROWSE_ID as i32 && code == BN_CLICKED {
                    let state = &mut *pointer;
                    let initial = PathBuf::from(read_text(state.file_edit))
                        .parent()
                        .map(|parent| parent.to_path_buf())
                        .or_else(|| state.pak_path.parent().map(|p| p.to_path_buf()));
                    if let Some(path) = pick_file_dialog(
                        hwnd,
                        "选择新文件",
                        "所有文件\0*.*\0",
                        initial.as_deref(),
                    ) {
                        set_text(state.file_edit, &path.to_string_lossy());
                        update_replace_state(hwnd);
                    }
                    return 0;
                }
                if id == REPLACE_OK_ID as i32 && code == BN_CLICKED {
                    handle_replace_confirm(hwnd);
                    return 0;
                }
                if id == REPLACE_CANCEL_ID as i32
                    || (id == IDCANCEL as i32 && code == BN_CLICKED)
                {
                    DestroyWindow(hwnd);
                    return 0;
                }
                if code == EN_CHANGE {
                    update_replace_state(hwnd);
                    return 0;
                }
                if code == BN_CLICKED
                    && (id == REPLACE_COMPRESS_ID as i32
                        || id == REPLACE_RAW_ID as i32
                        || id == REPLACE_BACKUP_ID as i32)
                {
                    update_replace_state(hwnd);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                let pointer =
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut ReplaceDialogState;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn replace_selected(main: HWND) {
        let (entry, pak_path) = {
            let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const Ui;
            if pointer.is_null() {
                return;
            }
            let Some(entry) = single_selected_file(&*pointer) else {
                return;
            };
            let Some(path) = (*pointer).path.clone() else {
                return;
            };
            (entry.clone(), path)
        };
        let mut success = false;
        open_replace_dialog(main, &pak_path, &entry, &mut success);
        if success {
            reload_pak(main);
        }
        focus_list(main);
    }

    unsafe fn open_replace_dialog(
        main: HWND,
        pak_path: &Path,
        entry: &SessionEntry,
        success: &mut bool,
    ) {
        let (class_name, instance) =
            register_dialog_class(REPLACE_CLASS, Some(replace_wndproc), COLOR_BTNFACE);

        let dpi = dpi_of(main);
        let width = scale(700, dpi);
        let height = scale(450, dpi);
        let dialog = create_centered_dialog(
            main,
            &class_name,
            instance,
            "替换条目",
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            width,
            height,
        );
        if dialog.is_null() {
            return;
        }
        let path_label = create_label(dialog, instance, &format!("包内路径：{}", entry.path));
        let compression = if entry.is_raw() { "RAW（原样）" } else { "LZSS" };
        let info_label = create_label(
            dialog,
            instance,
            &format!(
                "当前条目：原始 {} 字节 / 存储 {} 字节 / {} / mtime {}",
                entry.org, entry.size, compression, entry.mtime
            ),
        );
        let file_label = create_label(dialog, instance, "新文件：");
        let file_edit = create_text_field(dialog, instance, Some(""), REPLACE_FILE_ID);
        let browse = create_button(dialog, instance, "浏览…", 0, REPLACE_BROWSE_ID);
        let delta = create_label(dialog, instance, "请选择新文件。");
        let compress = create_button(
            dialog,
            instance,
            "压缩（LZSS，默认）",
            WS_GROUP | BS_AUTORADIOBUTTON as u32,
            REPLACE_COMPRESS_ID,
        );
        let raw = create_button(
            dialog,
            instance,
            "不压缩（原样，RAW）",
            BS_AUTORADIOBUTTON as u32,
            REPLACE_RAW_ID,
        );
        let hint = create_label(
            dialog,
            instance,
            "高熵大文件用原样可避免替换时内存占用升高。",
        );
        let backup = create_button(
            dialog,
            instance,
            "替换前创建备份",
            BS_AUTOCHECKBOX as u32,
            REPLACE_BACKUP_ID,
        );
        let backup_label = create_label(dialog, instance, "");
        let risk = create_label(dialog, instance, "");
        let ok = create_button(
            dialog,
            instance,
            "替换",
            BS_DEFPUSHBUTTON as u32,
            REPLACE_OK_ID,
        );
        let cancel = create_button(dialog, instance, "取消", 0, REPLACE_CANCEL_ID);
        if !all_present(&[
            path_label,
            info_label,
            file_label,
            file_edit,
            browse,
            delta,
            compress,
            raw,
            hint,
            backup,
            backup_label,
            risk,
            ok,
            cancel,
        ]) {
            DestroyWindow(dialog);
            return;
        }
        apply_default_font(&[
            path_label,
            info_label,
            file_label,
            file_edit,
            browse,
            delta,
            compress,
            raw,
            hint,
            backup,
            backup_label,
            risk,
            ok,
            cancel,
        ]);
        set_checked(compress, true);
        set_checked(backup, true);
        let bak = backup_path(pak_path);
        let bak_text = if bak.exists() {
            format!("备份路径：{}（同名备份将被覆盖）", bak.display())
        } else {
            format!("备份路径：{}", bak.display())
        };
        set_text(backup_label, &bak_text);
        EnableWindow(ok, 0);

        let state = Box::new(ReplaceDialogState {
            main,
            pak_path: pak_path.to_path_buf(),
            inner: entry.path.clone(),
            old_org: entry.org as u64,
            path_label,
            info_label,
            file_label,
            file_edit,
            browse,
            delta,
            compress,
            raw,
            hint,
            backup,
            backup_label,
            risk,
            ok,
            cancel,
            success,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_replace(dialog);
        run_modal(dialog, main, file_edit);
    }

    // -----------------------------------------------------------------------
    // G4-03/04/05 打包向导
    // -----------------------------------------------------------------------

    struct PackDialogState {
        main: HWND,
        source_label: HWND,
        source_edit: HWND,
        source_browse: HWND,
        output_label: HWND,
        output_edit: HWND,
        output_browse: HWND,
        compress: HWND,
        jobs_label: HWND,
        jobs_edit: HWND,
        note: HWND,
        stats: HWND,
        issues: HWND,
        risk: HWND,
        check: HWND,
        start: HWND,
        close: HWND,
        checked: bool,
        success_output: *mut Option<PathBuf>,
    }

    fn dir_is_writable(dir: &Path) -> bool {
        let probe = dir.join(format!(".wd-pak-write-check-{}.tmp", std::process::id()));
        match std::fs::write(&probe, b"") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
    }

    fn pack_target_problem(output: &Path) -> Option<String> {
        if output.as_os_str().is_empty() {
            return Some("请填写输出文件。".into());
        }
        if output.is_dir() {
            return Some("输出路径是一个目录。".into());
        }
        let parent = match output.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        if !parent.is_dir() {
            return Some(format!("输出目录不存在：{}", parent.display()));
        }
        if !dir_is_writable(parent) {
            return Some(format!("输出目录不可写：{}", parent.display()));
        }
        if output.exists()
            && std::fs::OpenOptions::new().write(true).open(output).is_err()
        {
            return Some("输出文件被其他程序占用或不可写。".into());
        }
        None
    }

    /// 预检问题单行文本：有包内路径时加分隔前缀。
    fn issue_line(path: &str, detail: &str) -> String {
        if path.is_empty() {
            format!("- {detail}")
        } else {
            format!("- {path}：{detail}")
        }
    }

    fn pack_issue_text(issues: &[PackIssue]) -> String {
        let mut lines: Vec<String> = issues
            .iter()
            .take(40)
            .map(|issue| issue_line(&issue.path, &issue.detail))
            .collect();
        if issues.len() > 40 {
            lines.push(format!("…另有 {} 项", issues.len() - 40));
        }
        lines.join("\r\n")
    }

    unsafe fn layout_pack(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const PackDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(18, dpi);
        let width = client.right - margin * 2;
        let edit_h = scale(26, dpi);
        let button_w = scale(104, dpi);
        let button_h = scale(32, dpi);
        let browse_w = scale(88, dpi);
        let mut y = scale(14, dpi);
        place(state.source_label, margin, y, width, scale(20, dpi));
        y += scale(22, dpi);
        place(
            state.source_edit,
            margin,
            y,
            width - browse_w - scale(8, dpi),
            edit_h,
        );
        place(
            state.source_browse,
            client.right - margin - browse_w,
            y,
            browse_w,
            edit_h,
        );
        y += scale(34, dpi);
        place(state.output_label, margin, y, width, scale(20, dpi));
        y += scale(22, dpi);
        place(
            state.output_edit,
            margin,
            y,
            width - browse_w - scale(8, dpi),
            edit_h,
        );
        place(
            state.output_browse,
            client.right - margin - browse_w,
            y,
            browse_w,
            edit_h,
        );
        y += scale(34, dpi);
        place(state.compress, margin, y, scale(230, dpi), scale(24, dpi));
        place(
            state.jobs_label,
            margin + scale(250, dpi),
            y,
            scale(80, dpi),
            scale(24, dpi),
        );
        place(
            state.jobs_edit,
            margin + scale(330, dpi),
            y,
            scale(64, dpi),
            scale(24, dpi),
        );
        y += scale(30, dpi);
        place(state.note, margin, y, width, scale(22, dpi));
        y += scale(26, dpi);
        place(state.stats, margin, y, width, scale(22, dpi));
        y += scale(28, dpi);
        let bottom = client.bottom - margin - button_h;
        let risk_h = scale(46, dpi);
        let risk_y = bottom - scale(10, dpi) - risk_h;
        let issues_h = (risk_y - scale(8, dpi) - y).max(scale(70, dpi));
        place(state.issues, margin, y, width, issues_h);
        place(state.risk, margin, risk_y, width, risk_h);
        place(
            state.check,
            margin,
            bottom,
            button_w,
            button_h,
        );
        place(
            state.close,
            client.right - margin - button_w,
            bottom,
            button_w,
            button_h,
        );
        place(
            state.start,
            client.right - margin - button_w * 2 - scale(10, dpi),
            bottom,
            button_w,
            button_h,
        );
    }

    unsafe fn update_pack_state(dialog: HWND) {
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut PackDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &mut *pointer;
        state.checked = false;
        EnableWindow(state.start, 0);
        let source = read_text(state.source_edit).trim().to_string();
        let output = read_text(state.output_edit).trim().to_string();
        let hint = if source.is_empty() {
            "请选择打包源目录。"
        } else if output.is_empty() {
            "请填写输出文件；修改输入后需要重新预检。"
        } else {
            "配置已修改，请重新预检后开始。"
        };
        set_text(state.risk, hint);
    }

    /// 扫描源目录并刷新预检区；返回 (源目录, 扫描结果, 预检结果)。
    unsafe fn refresh_pack_check(
        dialog: HWND,
    ) -> Option<(PathBuf, PackScan, PackPrecheckResult)> {
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut PackDialogState;
        if pointer.is_null() {
            return None;
        }
        let state = &mut *pointer;
        state.checked = false;
        EnableWindow(state.start, 0);
        let source_text = read_text(state.source_edit).trim().to_string();
        if source_text.is_empty() {
            show_message(dialog, "请选择打包源目录。", MB_OK | MB_ICONWARNING);
            SetFocus(state.source_edit);
            return None;
        }
        let source = PathBuf::from(&source_text);
        let is_dir = std::fs::metadata(&source)
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if !is_dir {
            let text = format!("源目录不存在或不是目录：\n{}", source.display());
            set_text(state.issues, &to_crlf(&text));
            show_message(dialog, &text, MB_OK | MB_ICONERROR);
            SetFocus(state.source_edit);
            return None;
        }
        let scan = match scan_dir(&source) {
            Ok(scan) => scan,
            Err(error) => {
                let text = format!("无法扫描源目录：\n{error}");
                set_text(state.issues, &to_crlf(&text));
                show_message(dialog, &text, MB_OK | MB_ICONERROR);
                return None;
            }
        };
        let mut precheck = pack_precheck(&scan.entries, u64::from(u32::MAX));
        let scanned: std::collections::BTreeSet<String> =
            scan.entries.iter().map(|entry| entry.rel_path.clone()).collect();
        precheck.issues.extend(manifest_mtime_issues(&source, &scanned));

        let note = if scan.manifest_present {
            "检测到 _pak_manifest.json：打包时会自动跳过，且按清单回放 order 与 mtime。"
        } else {
            "未检测到 _pak_manifest.json：条目按磁盘顺序打包，mtime 使用文件当前时间。"
        };
        set_text(state.note, note);
        let stats = format!(
            "文件 {} 个 / 目录 {} 个 / 源文件合计 {} 字节",
            precheck.file_count, precheck.dir_count, precheck.total_size
        );
        set_text(state.stats, &stats);
        let issues_text = if precheck.issues.is_empty() {
            "预检通过：可以开始打包。".to_string()
        } else {
            format!(
                "预检发现 {} 个阻断问题（修正后重新预检）：\r\n{}",
                precheck.issues.len(),
                pack_issue_text(&precheck.issues)
            )
        };
        set_text(state.issues, &issues_text);

        let output = PathBuf::from(read_text(state.output_edit).trim());
        let risk = if output.as_os_str().is_empty() {
            "请填写输出文件。".to_string()
        } else if output.exists() {
            format!("注意：输出文件已存在，开始前会要求确认覆盖：\n{}", output.display())
        } else {
            format!("将写入：{}", output.display())
        };
        set_text(state.risk, &risk);

        state.checked = precheck.issues.is_empty();
        let ready = state.checked && !output.as_os_str().is_empty();
        EnableWindow(state.start, ready as i32);
        Some((source, scan, precheck))
    }

    unsafe fn handle_pack_check(dialog: HWND) {
        if let Some((_, _, precheck)) = refresh_pack_check(dialog) {
            if precheck.issues.is_empty() {
                show_message(dialog, "预检通过，可以开始打包。", MB_OK | MB_ICONINFORMATION);
            }
        }
    }

    unsafe fn handle_pack_start(dialog: HWND) {
        let Some((source, _scan, precheck)) = refresh_pack_check(dialog) else {
            return;
        };
        if !precheck.issues.is_empty() {
            show_message(
                dialog,
                "预检未通过，请先处理问题列表中的阻断项。",
                MB_OK | MB_ICONERROR,
            );
            return;
        }
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut PackDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &mut *pointer;
        let output_text = read_text(state.output_edit).trim().to_string();
        let output = PathBuf::from(&output_text);
        if let Some(problem) = pack_target_problem(&output) {
            show_message(dialog, &problem, MB_OK | MB_ICONERROR);
            SetFocus(state.output_edit);
            return;
        }
        if output.exists() {
            let text = format!(
                "输出文件已存在，将被覆盖：\n{}\n\n原文件内容不可恢复，是否继续？",
                output.display()
            );
            let answer = MessageBoxW(
                dialog,
                wide(&text).as_ptr(),
                wide("确认覆盖").as_ptr(),
                MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
            );
            if answer != 6 {
                return;
            }
        }
        let jobs = match read_text(state.jobs_edit).trim().parse::<usize>() {
            Ok(value) => clamp_jobs(value, default_jobs()),
            Err(_) => {
                show_message(dialog, "并行度必须是 1-8 的整数。", MB_OK | MB_ICONWARNING);
                SetFocus(state.jobs_edit);
                return;
            }
        };
        let compress = is_checked(state.compress);
        {
            if !state.main.is_null() {
                let ui = GetWindowLongPtrW(state.main, GWLP_USERDATA) as *mut Ui;
                if !ui.is_null() {
                    (*ui).config.last_pack_dir =
                        Some(source.to_string_lossy().into_owned());
                    persist_config(&*ui);
                }
            }
        }
        let file_count = precheck.file_count;
        let dir_count = precheck.dir_count;
        let total_size = precheck.total_size;
        let source_worker = source.clone();
        let output_worker = output.clone();
        let outcome = run_write_progress(
            dialog,
            "打包",
            0,
            vec!["扫描源目录".into(), "压缩数据".into(), "写入 PAK".into()],
            false,
            move |_progress, _cancel, stage| {
                let started = Instant::now();
                stage.store(1, Ordering::Relaxed);
                let mut err = Vec::new();
                match cmd_pack(&source_worker, &output_worker, compress, jobs, &mut err) {
                    Ok(()) => {
                        stage.store(2, Ordering::Relaxed);
                        let output_size = std::fs::metadata(&output_worker)
                            .map(|meta| meta.len())
                            .unwrap_or(0);
                        let ratio = output_size as f64 / total_size.max(1) as f64;
                        let message = format!(
                            "打包完成：{} 个文件 / {} 个目录\n输出：{}\n源大小：{} 字节\n输出大小：{} 字节\n存储比：{:.3}\n耗时：{:.2} 秒",
                            file_count,
                            dir_count,
                            output_worker.display(),
                            total_size,
                            output_size,
                            ratio,
                            started.elapsed().as_secs_f64()
                        );
                        WriteOutcome::success(message)
                    }
                    Err(error) => {
                        let log = strip_hash(&String::from_utf8_lossy(&err));
                        let detail = log.trim();
                        let message = if detail.is_empty() {
                            format!("{error}")
                        } else {
                            format!("{error}\n{detail}")
                        };
                        WriteOutcome::failure(message)
                    }
                }
            },
        );
        if outcome.ok {
            let answer = MessageBoxW(
                dialog,
                wide(&format!("{}\n\n是否打开输出目录？", outcome.message)).as_ptr(),
                wide("打包完成").as_ptr(),
                MB_YESNO | MB_ICONINFORMATION,
            );
            if answer == 6 {
                if let Some(parent) = output.parent() {
                    reveal_folder(parent);
                }
            }
            *state.success_output = Some(output.clone());
            DestroyWindow(dialog);
        } else {
            let title = if outcome.cancelled {
                "打包已取消"
            } else {
                "打包失败"
            };
            show_message(
                dialog,
                &format!("{}：\n{}", title, outcome.message),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    unsafe extern "system" fn pack_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_pack(hwnd);
                0
            }
            WM_CTLCOLORSTATIC => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const PackDialogState;
                if !pointer.is_null() && lparam as HWND == (*pointer).issues {
                    let hdc = wparam as HDC;
                    SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
                    SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
                    GetSysColorBrush(COLOR_WINDOW) as isize
                } else {
                    dialog_color(wparam)
                }
            }
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PackDialogState;
                if pointer.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                if id == PACK_SOURCE_BROWSE_ID as i32 && code == BN_CLICKED {
                    let state = &*pointer;
                    if let Some(path) = browse_for_folder(hwnd, "选择打包源目录") {
                        set_text(state.source_edit, &path.to_string_lossy());
                        if read_text(state.output_edit).trim().is_empty() {
                            let output = path.join("output.pak");
                            set_text(state.output_edit, &output.to_string_lossy());
                        }
                        refresh_pack_check(hwnd);
                    }
                    return 0;
                }
                if id == PACK_OUTPUT_BROWSE_ID as i32 && code == BN_CLICKED {
                    let state = &*pointer;
                    let current = PathBuf::from(read_text(state.output_edit).trim());
                    let initial = current
                        .parent()
                        .map(|parent| parent.to_path_buf())
                        .filter(|parent| parent.is_dir())
                        .or_else(|| {
                            let source = PathBuf::from(read_text(state.source_edit).trim());
                            source.parent().map(|parent| parent.to_path_buf())
                        });
                    let default_name = current
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "output.pak".into());
                    if let Some(path) = pick_save_dialog(
                        hwnd,
                        "保存 PAK 文件",
                        "PAK 文件\0*.pak\0所有文件\0*.*\0",
                        initial.as_deref(),
                        &default_name,
                    ) {
                        set_text(state.output_edit, &path.to_string_lossy());
                        refresh_pack_check(hwnd);
                    }
                    return 0;
                }
                if id == PACK_CHECK_ID as i32 && code == BN_CLICKED {
                    handle_pack_check(hwnd);
                    return 0;
                }
                if id == PACK_START_ID as i32 && code == BN_CLICKED {
                    handle_pack_start(hwnd);
                    return 0;
                }
                if id == PACK_CLOSE_ID as i32
                    || (id == IDCANCEL as i32 && code == BN_CLICKED)
                {
                    DestroyWindow(hwnd);
                    return 0;
                }
                if code == EN_CHANGE {
                    update_pack_state(hwnd);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                let pointer =
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut PackDialogState;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn open_pack_dialog(main: HWND) {
        let (default_source, default_output) = {
            let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const Ui;
            if pointer.is_null() {
                return;
            }
            let ui = &*pointer;
            let source = ui
                .config
                .last_pack_dir
                .clone()
                .map(PathBuf::from)
                .filter(|path| path.is_dir())
                .or_else(|| {
                    ui.path
                        .as_ref()
                        .and_then(|path| path.parent())
                        .map(|parent| parent.to_path_buf())
                })
                .or_else(|| ui.last_dir.clone())
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            (source, ui.path.clone().unwrap_or_default())
        };

        let (class_name, instance) =
            register_dialog_class(PACK_CLASS, Some(pack_wndproc), COLOR_BTNFACE);

        let dpi = dpi_of(main);
        let width = scale(720, dpi);
        let height = scale(600, dpi);
        let dialog = create_centered_dialog(
            main,
            &class_name,
            instance,
            "从目录打包",
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            width,
            height,
        );
        if dialog.is_null() {
            return;
        }
        let source_label = create_label(dialog, instance, "源目录：");
        let source_edit = create_text_field(dialog, instance, Some(""), PACK_SOURCE_ID);
        let source_browse = create_button(dialog, instance, "浏览…", 0, PACK_SOURCE_BROWSE_ID);
        let output_label = create_label(dialog, instance, "输出 PAK：");
        let output_edit = create_text_field(dialog, instance, Some(""), PACK_OUTPUT_ID);
        let output_browse = create_button(dialog, instance, "浏览…", 0, PACK_OUTPUT_BROWSE_ID);
        let compress = create_button(
            dialog,
            instance,
            "压缩（LZSS）",
            BS_AUTOCHECKBOX as u32,
            PACK_COMPRESS_ID,
        );
        let jobs_label = create_label(dialog, instance, "并行度：");
        let jobs_edit = create_text_field(dialog, instance, Some(""), PACK_JOBS_ID);
        let note = create_label(dialog, instance, "");
        let stats = create_label(dialog, instance, "");
        let issues = create_text_area(dialog, instance);
        let risk = create_label(dialog, instance, "");
        let check = create_button(dialog, instance, "预检", 0, PACK_CHECK_ID);
        let start = create_button(
            dialog,
            instance,
            "开始打包",
            BS_DEFPUSHBUTTON as u32,
            PACK_START_ID,
        );
        let close = create_button(dialog, instance, "关闭", 0, PACK_CLOSE_ID);
        if !all_present(&[
            source_label,
            source_edit,
            source_browse,
            output_label,
            output_edit,
            output_browse,
            compress,
            jobs_label,
            jobs_edit,
            note,
            stats,
            issues,
            risk,
            check,
            start,
            close,
        ]) {
            DestroyWindow(dialog);
            return;
        }
        apply_default_font(&[
            source_label,
            source_edit,
            source_browse,
            output_label,
            output_edit,
            output_browse,
            compress,
            jobs_label,
            jobs_edit,
            note,
            stats,
            issues,
            risk,
            check,
            start,
            close,
        ]);
        SendMessageW(jobs_edit, EM_LIMITTEXT, 3, 0);
        set_checked(compress, true);
        set_text(source_edit, &default_source.to_string_lossy());
        set_text(output_edit, &default_output.to_string_lossy());
        set_text(jobs_edit, &default_jobs().to_string());
        set_text(note, "打包会跳过 _pak_manifest.json；点击“预检”扫描源目录。");
        set_text(stats, "尚未预检。");
        set_text(risk, "选择源目录后点击“预检”，通过后即可开始。");
        EnableWindow(start, 0);

        let mut success_output: Option<PathBuf> = None;
        let state = Box::new(PackDialogState {
            main,
            source_label,
            source_edit,
            source_browse,
            output_label,
            output_edit,
            output_browse,
            compress,
            jobs_label,
            jobs_edit,
            note,
            stats,
            issues,
            risk,
            check,
            start,
            close,
            checked: false,
            success_output: &mut success_output,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_pack(dialog);
        run_modal(dialog, main, source_edit);
        if let Some(output) = success_output {
            let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const Ui;
            let same = !pointer.is_null()
                && (*pointer)
                    .path
                    .as_ref()
                    .map(|path| {
                        wd_pak::platform::path_key(path)
                            == wd_pak::platform::path_key(&output)
                    })
                    .unwrap_or(false);
            if same {
                reload_pak(main);
            }
        }
    }

    // -----------------------------------------------------------------------
    // G4-06/07 解包向导
    // -----------------------------------------------------------------------

    struct UnpackDialogState {
        main: HWND,
        pak_path: PathBuf,
        dir_label: HWND,
        dir_edit: HWND,
        browse: HWND,
        open: HWND,
        note: HWND,
        stats: HWND,
        issues: HWND,
        risk: HWND,
        check: HWND,
        start: HWND,
        close: HWND,
        checked: bool,
    }

    fn nearest_existing_dir(path: &Path) -> Option<PathBuf> {
        let mut current = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        loop {
            if current.is_dir() {
                return Some(current);
            }
            if !current.pop() {
                return None;
            }
        }
    }

    fn disk_free_space(dir: &Path) -> Option<u64> {
        let wide_dir = wide(&dir.to_string_lossy());
        let mut free = 0u64;
        let mut total = 0u64;
        let ok = unsafe {
            GetDiskFreeSpaceExW(wide_dir.as_ptr(), &mut free, &mut total, null_mut())
        };
        (ok != 0).then_some(free)
    }

    fn unpack_issue_line(issue: &UnpackIssue) -> String {
        issue_line(&issue.path, &issue.detail)
    }

    unsafe fn layout_unpack(hwnd: HWND) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const UnpackDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &*pointer;
        let (client, dpi) = client_metrics(hwnd);
        let margin = scale(18, dpi);
        let width = client.right - margin * 2;
        let edit_h = scale(26, dpi);
        let browse_w = scale(88, dpi);
        let button_w = scale(104, dpi);
        let button_h = scale(32, dpi);
        let mut y = scale(14, dpi);
        place(state.dir_label, margin, y, width, scale(20, dpi));
        y += scale(22, dpi);
        let open_x = client.right - margin - browse_w;
        let browse_x = open_x - browse_w - scale(8, dpi);
        place(
            state.dir_edit,
            margin,
            y,
            browse_x - margin - scale(8, dpi),
            edit_h,
        );
        place(state.browse, browse_x, y, browse_w, edit_h);
        place(state.open, open_x, y, browse_w, edit_h);
        y += scale(34, dpi);
        place(state.note, margin, y, width, scale(22, dpi));
        y += scale(26, dpi);
        place(state.stats, margin, y, width, scale(22, dpi));
        y += scale(28, dpi);
        let bottom = client.bottom - margin - button_h;
        let risk_h = scale(64, dpi);
        let risk_y = bottom - scale(10, dpi) - risk_h;
        let issues_h = (risk_y - scale(8, dpi) - y).max(scale(70, dpi));
        place(state.issues, margin, y, width, issues_h);
        place(state.risk, margin, risk_y, width, risk_h);
        place(state.check, margin, bottom, button_w, button_h);
        place(
            state.close,
            client.right - margin - button_w,
            bottom,
            button_w,
            button_h,
        );
        place(
            state.start,
            client.right - margin - button_w * 2 - scale(10, dpi),
            bottom,
            button_w,
            button_h,
        );
    }

    unsafe fn update_unpack_state(dialog: HWND) {
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut UnpackDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &mut *pointer;
        state.checked = false;
        EnableWindow(state.start, 0);
        let hint = if read_text(state.dir_edit).trim().is_empty() {
            "请选择解包输出目录。"
        } else {
            "配置已修改，请重新预检后开始。"
        };
        set_text(state.risk, hint);
    }

    unsafe fn refresh_unpack_check(
        dialog: HWND,
    ) -> Option<(PathBuf, Vec<SessionEntry>, UnpackPrecheckResult, usize)> {
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut UnpackDialogState;
        if pointer.is_null() {
            return None;
        }
        let state = &mut *pointer;
        state.checked = false;
        EnableWindow(state.start, 0);
        let dir_text = read_text(state.dir_edit).trim().to_string();
        if dir_text.is_empty() {
            show_message(dialog, "请选择解包输出目录。", MB_OK | MB_ICONWARNING);
            SetFocus(state.dir_edit);
            return None;
        }
        let raw_dir = PathBuf::from(&dir_text);
        let out_dir = std::path::absolute(&raw_dir).unwrap_or(raw_dir);
        if out_dir.exists() && !out_dir.is_dir() {
            let text = format!("目标路径已存在且不是目录：\n{}", out_dir.display());
            set_text(state.issues, &to_crlf(&text));
            show_message(dialog, &text, MB_OK | MB_ICONERROR);
            SetFocus(state.dir_edit);
            return None;
        }
        let entries = {
            let ui = GetWindowLongPtrW(state.main, GWLP_USERDATA) as *const Ui;
            if ui.is_null() {
                return None;
            }
            let Some(session) = (*ui).session.as_ref() else {
                show_message(dialog, "当前没有打开的 PAK。", MB_OK | MB_ICONERROR);
                return None;
            };
            session.entries().to_vec()
        };
        let writable = nearest_existing_dir(&out_dir)
            .map(|existing| dir_is_writable(&existing))
            .unwrap_or(false);
        let free = nearest_existing_dir(&out_dir)
            .and_then(|existing| disk_free_space(&existing))
            .unwrap_or(u64::MAX);
        let tuples: Vec<(String, u64, bool)> = entries
            .iter()
            .map(|entry| (entry.path.clone(), u64::from(entry.org), entry.is_dir()))
            .collect();
        let base_len = out_dir.to_string_lossy().chars().count();
        let precheck = unpack_precheck_full(&tuples, writable, free, base_len, 260);
        let existing = entries
            .iter()
            .filter(|entry| !entry.is_dir())
            .filter(|entry| out_dir.join(&entry.path).exists())
            .count();

        let stats = format!(
            "文件 {} 个 / 目录 {} 个 / 解压后 {} 字节\n目标：{}",
            precheck.file_count,
            precheck.dir_count,
            precheck.total_size,
            out_dir.display()
        );
        set_text(state.stats, &stats);

        let mut hard = Vec::new();
        let mut soft = Vec::new();
        for issue in &precheck.issues {
            let line = unpack_issue_line(issue);
            if issue.kind == UnpackIssueKind::CaseConflict {
                soft.push(line);
            } else {
                hard.push(line);
            }
        }
        let mut issues_text = String::new();
        if !hard.is_empty() {
            issues_text.push_str(&format!("阻断问题 {} 个：\r\n", hard.len()));
            issues_text.push_str(&hard.join("\r\n"));
        } else {
            issues_text.push_str("预检通过：可以开始解包。");
        }
        if !soft.is_empty() {
            if !issues_text.is_empty() {
                issues_text.push_str("\r\n\r\n");
            }
            issues_text.push_str("提示（确认后可继续）：\r\n");
            issues_text.push_str(&soft.join("\r\n"));
        }
        set_text(state.issues, &issues_text);

        let mut risk = String::new();
        if existing > 0 {
            risk.push_str(&format!(
                "目标目录已有 {existing} 个同名文件，开始前会要求确认覆盖。\n"
            ));
        }
        if precheck.has_kind(UnpackIssueKind::CaseConflict) {
            risk.push_str("存在大小写冲突：后写入的条目会覆盖先写入的。\n");
        }
        if !out_dir.exists() {
            risk.push_str("目标目录不存在，开始时自动创建。\n");
        }
        risk.push_str("解包只读取源包；取消时已写入文件保留。");
        set_text(state.risk, &risk);

        state.checked = !precheck.blocking();
        EnableWindow(state.start, state.checked as i32);
        Some((out_dir, entries, precheck, existing))
    }

    unsafe fn handle_unpack_check(dialog: HWND) {
        if let Some((_, _, precheck, _)) = refresh_unpack_check(dialog) {
            if !precheck.blocking() {
                show_message(dialog, "预检通过，可以开始解包。", MB_OK | MB_ICONINFORMATION);
            }
        }
    }

    unsafe fn handle_unpack_start(dialog: HWND) {
        let Some((out_dir, entries, precheck, existing)) = refresh_unpack_check(dialog) else {
            return;
        };
        if precheck.blocking() {
            show_message(
                dialog,
                "预检未通过，请先处理问题列表中的阻断项。",
                MB_OK | MB_ICONERROR,
            );
            return;
        }
        let case_conflict = precheck.has_kind(UnpackIssueKind::CaseConflict);
        if existing > 0 || case_conflict {
            let mut text = String::new();
            if existing > 0 {
                text.push_str(&format!(
                    "目标目录已有 {existing} 个同名文件，将被覆盖。\n"
                ));
            }
            if case_conflict {
                text.push_str("包内存在大小写冲突，后写入的条目会覆盖先写入的。\n");
            }
            text.push_str("\n是否继续解包？");
            let answer = MessageBoxW(
                dialog,
                wide(&text).as_ptr(),
                wide("确认解包").as_ptr(),
                MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
            );
            if answer != 6 {
                return;
            }
        }
        if let Err(error) = std::fs::create_dir_all(&out_dir) {
            show_message(
                dialog,
                &format!("无法创建输出目录：\n{error}"),
                MB_OK | MB_ICONERROR,
            );
            return;
        }
        if !dir_is_writable(&out_dir) {
            show_message(
                dialog,
                "输出目录不可写，无法解包。",
                MB_OK | MB_ICONERROR,
            );
            return;
        }
        let pointer = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut UnpackDialogState;
        if pointer.is_null() {
            return;
        }
        let state = &mut *pointer;
        {
            let ui = GetWindowLongPtrW(state.main, GWLP_USERDATA) as *mut Ui;
            if !ui.is_null() {
                (*ui).config.last_unpack_dir =
                    Some(out_dir.to_string_lossy().into_owned());
                let _ = persist::save_config(&(*ui).config_path, &(*ui).config);
            }
        }
        let pak_path = state.pak_path.clone();
        let file_count = precheck.file_count;
        let dir_count = precheck.dir_count;
        let total_entries = entries.len();
        let out_worker = out_dir.clone();
        let outcome = run_write_progress(
            dialog,
            "解包",
            total_entries,
            vec!["准备目录".into(), "写入文件".into(), "生成清单".into()],
            true,
            move |progress, cancel, stage| {
                let reader = match PakReader::open(&pak_path) {
                    Ok(reader) => reader,
                    Err(error) => {
                        return WriteOutcome::failure(format!("打开源包失败：{error}"))
                    }
                };
                stage.store(1, Ordering::Relaxed);
                let mut manifest = Manifest {
                    pak: pak_path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    files: Vec::new(),
                    order: Vec::new(),
                };
                let mut written = 0usize;
                let mut failures: Vec<(String, String)> = Vec::new();
                let mut index = 0usize;
                for entry in &entries {
                    if cancel.load(Ordering::Relaxed) {
                        return WriteOutcome {
                            ok: false,
                            cancelled: true,
                            message: format!(
                                "解包已取消：已写入 {} 个文件，文件保留在目标目录。",
                                written
                            ),
                        };
                    }
                    manifest.order.push(entry.path.clone());
                    let target = out_worker.join(&entry.path);
                    if entry.is_dir() {
                        if let Err(error) = std::fs::create_dir_all(&target) {
                            failures.push((
                                entry.path.clone(),
                                format!("创建目录失败: {error}"),
                            ));
                        }
                    } else {
                        let data = match reader.find(&entry.path) {
                            Some(node) => match reader.read_file(node) {
                                Ok(data) => Ok(data),
                                Err(error) => Err(format!("读取失败: {error}")),
                            },
                            None => Err("包内条目不存在".into()),
                        };
                        match data {
                            Ok(data) => {
                                let parent_ok = target
                                    .parent()
                                    .map(|parent| {
                                        std::fs::create_dir_all(parent)
                                            .map_err(|error| {
                                                format!("创建目录失败: {error}")
                                            })
                                    })
                                    .unwrap_or(Ok(()));
                                match parent_ok {
                                    Ok(()) => match std::fs::write(&target, &data) {
                                        Ok(()) => {
                                            written += 1;
                                            manifest.files.push((
                                                entry.path.clone(),
                                                FileStat {
                                                    attrib: entry.attrib,
                                                    mtime: entry.mtime,
                                                    stored: u64::from(entry.size),
                                                    orig: u64::from(entry.org),
                                                },
                                            ));
                                        }
                                        Err(error) => failures.push((
                                            entry.path.clone(),
                                            format!("写入失败: {error}"),
                                        )),
                                    },
                                    Err(error) => {
                                        failures.push((entry.path.clone(), error))
                                    }
                                }
                            }
                            Err(error) => {
                                failures.push((entry.path.clone(), error))
                            }
                        }
                    }
                    index += 1;
                    progress.store(index, Ordering::Relaxed);
                }
                if !failures.is_empty() {
                    let detail = failures
                        .iter()
                        .take(8)
                        .map(|(path, error)| format!("{path}: {error}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let more = if failures.len() > 8 {
                        format!("\n…另有 {} 项", failures.len() - 8)
                    } else {
                        String::new()
                    };
                    return WriteOutcome {
                        ok: false,
                        cancelled: false,
                        message: format!(
                            "解包未完成：成功写入 {} 个文件，失败 {} 项。\n\n{detail}{more}",
                            written,
                            failures.len()
                        ),
                    };
                }
                stage.store(2, Ordering::Relaxed);
                let manifest_bytes = match manifest.to_bytes() {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        return WriteOutcome::failure(format!(
                            "生成 manifest 失败：{error}"
                        ))
                    }
                };
                if let Err(error) =
                    std::fs::write(out_worker.join(MANIFEST_NAME), &manifest_bytes)
                {
                    return WriteOutcome::failure(format!(
                        "写入 {MANIFEST_NAME} 失败：{error}"
                    ));
                }
                WriteOutcome::success(format!(
                    "解包完成：{} 个文件 / {} 个目录\n输出：{}\n已生成 {}",
                    file_count,
                    dir_count,
                    out_worker.display(),
                    MANIFEST_NAME
                ))
            },
        );
        if outcome.ok {
            let answer = MessageBoxW(
                dialog,
                wide(&format!("{}\n\n是否打开输出目录？", outcome.message)).as_ptr(),
                wide("解包完成").as_ptr(),
                MB_YESNO | MB_ICONINFORMATION,
            );
            if answer == 6 {
                reveal_folder(&out_dir);
            }
            DestroyWindow(dialog);
        } else if outcome.cancelled {
            show_message(
                dialog,
                &format!("{}\n可以修正选项后重新开始。", outcome.message),
                MB_OK | MB_ICONWARNING,
            );
        } else {
            show_message(
                dialog,
                &format!("解包失败：\n{}", outcome.message),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    unsafe extern "system" fn unpack_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                layout_unpack(hwnd);
                0
            }
            WM_CTLCOLORSTATIC => {
                let pointer =
                    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const UnpackDialogState;
                if !pointer.is_null() && lparam as HWND == (*pointer).issues {
                    let hdc = wparam as HDC;
                    SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
                    SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
                    GetSysColorBrush(COLOR_WINDOW) as isize
                } else {
                    dialog_color(wparam)
                }
            }
            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let code = ((wparam >> 16) & 0xffff) as u32;
                let pointer =
                    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut UnpackDialogState;
                if pointer.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                if id == UNPACK_BROWSE_ID as i32 && code == BN_CLICKED {
                    let state = &*pointer;
                    if let Some(path) = browse_for_folder(hwnd, "选择解包目录") {
                        set_text(state.dir_edit, &path.to_string_lossy());
                        refresh_unpack_check(hwnd);
                    }
                    return 0;
                }
                if id == UNPACK_OPEN_ID as i32 && code == BN_CLICKED {
                    let state = &*pointer;
                    let dir = PathBuf::from(read_text(state.dir_edit).trim());
                    if dir.is_dir() {
                        reveal_folder(&dir);
                    } else {
                        show_message(
                            hwnd,
                            "输出目录尚不存在，解包完成后会自动创建。",
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                    return 0;
                }
                if id == UNPACK_CHECK_ID as i32 && code == BN_CLICKED {
                    handle_unpack_check(hwnd);
                    return 0;
                }
                if id == UNPACK_START_ID as i32 && code == BN_CLICKED {
                    handle_unpack_start(hwnd);
                    return 0;
                }
                if id == UNPACK_CLOSE_ID as i32
                    || (id == IDCANCEL as i32 && code == BN_CLICKED)
                {
                    DestroyWindow(hwnd);
                    return 0;
                }
                if code == EN_CHANGE {
                    update_unpack_state(hwnd);
                    return 0;
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                let pointer =
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut UnpackDialogState;
                if !pointer.is_null() {
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn open_unpack_dialog(main: HWND) {
        let (pak_path, default_dir, initial_stats) = {
            let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const Ui;
            if pointer.is_null() {
                return;
            }
            let ui = &*pointer;
            let (Some(pak_path), Some(session)) = (ui.path.clone(), ui.session.as_ref()) else {
                show_message(
                    main,
                    "请先打开一个 PAK 文件，再执行解包。",
                    MB_OK | MB_ICONINFORMATION,
                );
                return;
            };
            let stats = session.stats();
            let default_dir = ui
                .config
                .last_unpack_dir
                .clone()
                .map(PathBuf::from)
                .filter(|path| path.is_dir())
                .unwrap_or_else(|| {
                    let parent = pak_path
                        .parent()
                        .map(|parent| parent.to_path_buf())
                        .unwrap_or_else(|| {
                            std::env::current_dir().unwrap_or_default()
                        });
                    parent.join(
                        pak_path
                            .file_stem()
                            .map(|stem| stem.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "unpacked".into()),
                    )
                });
            (
                pak_path,
                default_dir,
                (stats.files, stats.dirs, stats.total_org),
            )
        };

        let (class_name, instance) =
            register_dialog_class(UNPACK_CLASS, Some(unpack_wndproc), COLOR_BTNFACE);

        let dpi = dpi_of(main);
        let width = scale(700, dpi);
        let height = scale(560, dpi);
        let dialog = create_centered_dialog(
            main,
            &class_name,
            instance,
            "解包到目录",
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            width,
            height,
        );
        if dialog.is_null() {
            return;
        }
        let dir_label = create_label(dialog, instance, "输出目录：");
        let dir_edit = create_text_field(dialog, instance, Some(""), UNPACK_DIR_ID);
        let browse = create_button(dialog, instance, "浏览…", 0, UNPACK_BROWSE_ID);
        let open = create_button(dialog, instance, "打开目录", 0, UNPACK_OPEN_ID);
        let note = create_label(dialog, instance, &format!("源包：{}", pak_path.display()));
        let stats_text = format!(
            "文件 {} 个 / 目录 {} 个 / 解压后 {} 字节",
            initial_stats.0, initial_stats.1, initial_stats.2
        );
        let stats = create_label(dialog, instance, &stats_text);
        let issues = create_text_area(dialog, instance);
        let risk = create_label(dialog, instance, "解包只读取源包；取消时已写入文件保留。");
        let check = create_button(dialog, instance, "预检", 0, UNPACK_CHECK_ID);
        let start = create_button(
            dialog,
            instance,
            "开始解包",
            BS_DEFPUSHBUTTON as u32,
            UNPACK_START_ID,
        );
        let close = create_button(dialog, instance, "关闭", 0, UNPACK_CLOSE_ID);
        if !all_present(&[
            dir_label,
            dir_edit,
            browse,
            open,
            note,
            stats,
            issues,
            risk,
            check,
            start,
            close,
        ]) {
            DestroyWindow(dialog);
            return;
        }
        apply_default_font(&[
            dir_label,
            dir_edit,
            browse,
            open,
            note,
            stats,
            issues,
            risk,
            check,
            start,
            close,
        ]);
        set_text(dir_edit, &default_dir.to_string_lossy());
        EnableWindow(start, 0);

        let state = Box::new(UnpackDialogState {
            main,
            pak_path,
            dir_label,
            dir_edit,
            browse,
            open,
            note,
            stats,
            issues,
            risk,
            check,
            start,
            close,
            checked: false,
        });
        SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize);
        layout_unpack(dialog);
        if default_dir.is_dir() {
            refresh_unpack_check(dialog);
        }
        run_modal(dialog, main, dir_edit);
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

    /// 把文本以 CF_UNICODETEXT 写入剪贴板（复制详情/包内路径）。
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
        let selected = unsafe { selected_entries(ui) };
        detail_text_multi(ui.session.as_ref(), &selected, mtime_text)
    }

    fn status_text(ui: &Ui) -> String {
        let selected = unsafe { selected_entries(ui) };
        let base = status_text_multi(ui.path.as_deref(), ui.session.as_ref(), &selected);
        match &ui.session {
            Some(session) if !ui.filter.is_empty() => {
                format!("{base} | 筛选中：{} / {}", ui.visible.len(), session.entries().len())
            }
            _ => base,
        }
    }

    unsafe fn paint(hwnd: HWND) {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);

        let (_client, dpi, rects) = shell_layout(hwnd);
        zone(hdc, rects.command_bar, dpi, COLOR_BTNFACE);
        zone(hdc, rects.nav, dpi, COLOR_BTNFACE);
        zone(hdc, rects.tabs, dpi, COLOR_WINDOW);
        zone(hdc, rects.content, dpi, COLOR_WINDOW);
        zone(hdc, rects.detail_overlay.unwrap_or(rects.detail), dpi, COLOR_WINDOW);
        zone(hdc, rects.task_panel, dpi, COLOR_BTNFACE);
        zone(hdc, rects.status_bar, dpi, COLOR_BTNFACE);
        paint_tabs(hwnd, hdc, rects.tabs, dpi);

        EndPaint(hwnd, &ps);
    }

    unsafe fn paint_tabs(hwnd: HWND, hdc: HDC, tabs: Rect, dpi: u32) {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
        if pointer.is_null() || (*pointer).font.is_null() {
            return;
        }
        let ui = &*pointer;
        let layout = tab_layout(ui, tabs, dpi);
        if layout.is_empty() {
            return;
        }
        let previous_font = SelectObject(hdc, ui.font);
        SetBkMode(hdc, TRANSPARENT as i32);
        for (index, (rect, close)) in layout.iter().enumerate() {
            let active = ui.workspace.active_index() == Some(index);
            if active {
                let bounds = RECT {
                    left: rect.x,
                    top: rect.y,
                    right: rect.right(),
                    bottom: rect.bottom(),
                };
                FillRect(hdc, &bounds, GetSysColorBrush(COLOR_WINDOW));
                let accent = RECT {
                    left: rect.x,
                    top: rect.bottom() - scale(2, dpi),
                    right: rect.right(),
                    bottom: rect.bottom(),
                };
                FillRect(hdc, &accent, GetSysColorBrush(COLOR_HIGHLIGHT));
            }
            let name = ui
                .workspace
                .tab(index)
                .and_then(|tab| tab.path().file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut buffer = wide(&name);
            let mut text_rect = RECT {
                left: rect.x + scale(8, dpi),
                top: rect.y,
                right: (close.x - scale(4, dpi)).max(rect.x),
                bottom: rect.bottom(),
            };
            SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
            DrawTextW(
                hdc,
                buffer.as_mut_ptr(),
                buffer.len() as i32 - 1,
                &mut text_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let mut glyph = wide("×");
            let mut close_rect = RECT {
                left: close.x,
                top: close.y,
                right: close.right() - scale(4, dpi),
                bottom: close.bottom(),
            };
            DrawTextW(
                hdc,
                glyph.as_mut_ptr(),
                glyph.len() as i32 - 1,
                &mut close_rect,
                DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
        SelectObject(hdc, previous_font);
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
        let open_button = create_button(hwnd, instance, "打开 PAK", 0, OPEN_BUTTON_ID);
        let export_button = create_button(hwnd, instance, "导出所选…", 0, EXPORT_BUTTON_ID);
        let edit = create_text_field(hwnd, instance, None, 0);
        let hits = create_control(
            hwnd,
            instance,
            "STATIC",
            null(),
            WS_CHILD | WS_VISIBLE | SS_NOPREFIX,
            0,
            0,
        );
        let list = create_control(
            hwnd,
            instance,
            "SysListView32",
            null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | LVS_REPORT | LVS_OWNERDATA,
            WS_EX_CLIENTEDGE,
            0,
        );
        let empty = create_control(
            hwnd,
            instance,
            "STATIC",
            null(),
            WS_CHILD | WS_VISIBLE | SS_CENTER | SS_NOPREFIX,
            0,
            0,
        );
        let detail = create_control(
            hwnd,
            instance,
            "STATIC",
            null(),
            WS_CHILD | WS_VISIBLE | WS_BORDER,
            0,
            0,
        );
        let status = create_control(
            hwnd,
            instance,
            "STATIC",
            null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
        );
        if !all_present(&[
            open_button,
            export_button,
            edit,
            hits,
            list,
            empty,
            detail,
            status,
        ]) {
            fatal("创建控件失败");
            return None;
        }

        set_text(list, "包内路径列表");
        set_text(empty, "空状态提示");

        let dpi = dpi_of(hwnd);
        let font = create_ui_font(dpi);
        for control in [open_button, export_button, edit, list, detail, status, empty, hits] {
            SendMessageW(control, WM_SETFONT, font as usize, 1);
        }

        let cue = wide("搜索包内路径/文件名（不区分大小写）");
        SendMessageW(edit, EM_SETCUEBANNER, 1, cue.as_ptr() as isize);
        let columns: Vec<Column> = Column::ALL
            .into_iter()
            .filter(|column| default_visible(*column, ViewMode::List))
            .collect();
        for (index, column) in columns.iter().enumerate() {
            insert_column(list, index, column.title(), scale(column.default_width(), dpi));
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
            empty,
            hits,
            open_button,
            export_button,
            columns,
            column_mode: ViewMode::List,
            column_tab: None,
            visible: Vec::new(),
            depths: Vec::new(),
            highlight: None,
            session: None,
            path: None,
            last_dir: None,
            filter: String::new(),
            detail_hidden: false,
            font,
            text: Vec::new(),
            workspace: Workspace::new(),
            runtime: Vec::new(),
            config: Config::default(),
            config_path: PathBuf::new(),
            session_path: PathBuf::new(),
            recent_menu: null_mut(),
            config_read_only: false,
            error_log: ErrorLog::default(),
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
                dwICC: ICC_LISTVIEW_CLASSES | ICC_PROGRESS_CLASS,
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
            // §5.1：窗口尺寸存 DIP，按系统 DPI 换算成物理像素。
            let system_dpi = GetDpiForSystem();
            let (config_path, session_path) = persist::paths();
            let (config, config_reason, config_state) = persist::load_config(&config_path);
            let had_config = matches!(config_state, persist::ConfigLoadState::Ok);
            let config_read_only = matches!(config_state, persist::ConfigLoadState::ReadOnly);
            let (x, y) = if had_config {
                (
                    scale(config.window.x, system_dpi),
                    scale(config.window.y, system_dpi),
                )
            } else {
                (CW_USEDEFAULT, CW_USEDEFAULT)
            };
            let hwnd = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                x,
                y,
                scale(config.window.width, system_dpi),
                scale(config.window.height, system_dpi),
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                fatal("创建窗口失败");
                return;
            }

            let Some(mut ui) = create_ui(hwnd) else {
                return;
            };

            let recent_menu;
            {
                let hmenu = CreateMenu();
                let file_menu = CreatePopupMenu();
                append_menu(file_menu, MF_STRING, MENU_OPEN, "打开 PAK…\tCtrl+O");
                append_menu(file_menu, MF_STRING, MENU_CLOSE, "关闭标签\tCtrl+W");
                append_menu(file_menu, MF_STRING, MENU_REFRESH, "刷新\tF5");
                append_separator(file_menu);
                append_menu(file_menu, MF_STRING, MENU_PREVIEW, "预览所选");
                append_menu(file_menu, MF_STRING, MENU_EXPORT, "导出所选…\tCtrl+E");
                append_menu(file_menu, MF_STRING, MENU_REPLACE, "替换…\tCtrl+R");
                append_separator(file_menu);
                append_menu(file_menu, MF_STRING, MENU_PACK, "从目录打包…\tCtrl+N");
                append_menu(file_menu, MF_STRING, MENU_UNPACK, "解包到目录…");
                append_separator(file_menu);
                recent_menu = CreatePopupMenu();
                append_menu(file_menu, MF_POPUP, recent_menu as usize, "最近打开");
                append_separator(file_menu);
                append_menu(file_menu, MF_STRING, MENU_EXIT, "退出\tAlt+F4");
                append_menu(hmenu, MF_POPUP, file_menu as usize, "文件(&F)");

                let edit_menu = CreatePopupMenu();
                append_menu(edit_menu, MF_STRING, MENU_FIND, "查找\tCtrl+F");
                append_menu(edit_menu, MF_STRING, MENU_COPY_PATH, "复制包内路径\tCtrl+Shift+C");
                append_menu(hmenu, MF_POPUP, edit_menu as usize, "编辑(&E)");

                let view_menu = CreatePopupMenu();
                let columns_menu = CreatePopupMenu();
                for (ordinal, column) in Column::ALL.iter().enumerate() {
                    let flags = if *column == Column::Name {
                        MF_STRING | MF_GRAYED | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    append_menu(columns_menu, flags, MENU_COLUMN_BASE + ordinal, column.title());
                }
                append_menu(view_menu, MF_STRING, MENU_VIEW_TREE, "树视图");
                append_menu(view_menu, MF_STRING, MENU_VIEW_LIST, "列表视图");
                append_separator(view_menu);
                append_menu(view_menu, MF_POPUP, columns_menu as usize, "显示列");
                append_menu(view_menu, MF_STRING, MENU_COLUMNS_RESET, "重置列宽");
                append_separator(view_menu);
                append_menu(view_menu, MF_STRING, MENU_LOCATE, "定位首个匹配\tCtrl+L");
                append_menu(hmenu, MF_POPUP, view_menu as usize, "视图(&V)");

                let help_menu = CreatePopupMenu();
                append_menu(help_menu, MF_STRING, MENU_ABOUT, "关于 wd-pak");
                append_menu(hmenu, MF_POPUP, help_menu as usize, "帮助(&H)");

                SetMenu(hwnd, hmenu);
                DrawMenuBar(hwnd);
            }
            ui.config = config;
            ui.config_path = config_path;
            ui.session_path = session_path;
            ui.recent_menu = recent_menu;
            ui.config_read_only = config_read_only;
            let edit = ui.edit;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ui) as isize);
            layout_controls(hwnd);
            update_menu_state(hwnd);
            refresh_texts(hwnd);
            rebuild_recent_menu(hwnd);

            let mut accelerators = vec![
                ACCEL { fVirt: FVIRTKEY | FCONTROL, key: 0x4F, cmd: MENU_OPEN as u16 },
                ACCEL { fVirt: FVIRTKEY | FCONTROL, key: 0x46, cmd: MENU_FIND as u16 },
                ACCEL { fVirt: FVIRTKEY | FCONTROL, key: 0x57, cmd: MENU_CLOSE as u16 },
                ACCEL { fVirt: FVIRTKEY, key: 0x74, cmd: MENU_REFRESH as u16 },
                ACCEL { fVirt: FVIRTKEY, key: 0x1B, cmd: MENU_CLEAR as u16 },
                ACCEL { fVirt: FVIRTKEY | FCONTROL, key: 0x4C, cmd: MENU_LOCATE as u16 },
                ACCEL { fVirt: FVIRTKEY | FCONTROL, key: 0x45, cmd: MENU_EXPORT as u16 },
                ACCEL { fVirt: FVIRTKEY | FCONTROL, key: 0x52, cmd: MENU_REPLACE as u16 },
                ACCEL { fVirt: FVIRTKEY | FCONTROL, key: 0x4E, cmd: MENU_PACK as u16 },
                ACCEL {
                    fVirt: FVIRTKEY | FCONTROL | FSHIFT,
                    key: 0x43,
                    cmd: MENU_COPY_PATH as u16,
                },
                ACCEL {
                    fVirt: FVIRTKEY | FCONTROL,
                    key: 0x09,
                    cmd: MENU_NEXT_TAB as u16,
                },
            ];
            for digit in 1..=9u8 {
                accelerators.push(ACCEL {
                    fVirt: FVIRTKEY | FCONTROL,
                    key: 0x30 + digit as u16,
                    cmd: (MENU_TAB_BASE + digit as usize - 1) as u16,
                });
            }
            let haccel = CreateAcceleratorTableW(accelerators.as_ptr(), accelerators.len() as i32);

            let ui_pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
            if !ui_pointer.is_null() && (*ui_pointer).config.window.maximized {
                ShowWindow(hwnd, SW_MAXIMIZE);
            } else {
                ShowWindow(hwnd, SW_SHOW);
            }
            UpdateWindow(hwnd);
            SetFocus(edit);
            if let Some(reason) = config_reason {
                let text = if config_read_only {
                    format!("配置不可读，已用默认值且不会保存：{reason}")
                } else {
                    format!("配置已回退默认值：{reason}")
                };
                show_message(hwnd, &text, MB_OK | MB_ICONWARNING);
            }

            let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
            if arguments.is_empty() {
                restore_session(hwnd);
            } else {
                for path in arguments {
                    open_path(hwnd, path, OpenSource::CommandBar);
                }
            }

            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                if !haccel.is_null() && TranslateAcceleratorW(hwnd, haccel, &msg) != 0 {
                    continue;
                }
                // IsDialogMessage 会把主窗口上的回车当默认按钮消息吞掉，列表因此收不到；
                // 列表有焦点时在此直接预览，其余按键仍交给 IsDialogMessage 做焦点导航。
                if msg.message == WM_KEYDOWN && msg.wParam == 0x0D {
                    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui;
                    if !pointer.is_null() && GetFocus() == (*pointer).list {
                        preview_selected(hwnd);
                        continue;
                    }
                }
                if IsDialogMessageW(hwnd, &msg) != 0 {
                    continue;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if !haccel.is_null() {
                DestroyAcceleratorTable(haccel);
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



