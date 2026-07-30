use std::ptr;

use windows::{
    Win32::{
        Foundation::{HANDLE, HWND},
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{
                EVENT_SYSTEM_FOREGROUND, GA_ROOT, GetAncestor, GetForegroundWindow, SetPropW,
                WINEVENT_OUTOFCONTEXT,
            },
        },
    },
    core::{Error, w},
};

pub fn allow_foreground_activation() -> Result<HWINEVENTHOOK, String> {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_invalid() {
            return Err(format!("监听前台窗口变化失败: {:?}", Error::from_thread()));
        }

        if let Err(err) = mark_foreground_window(GetForegroundWindow()) {
            let _ = UnhookWinEvent(hook);
            return Err(format!("允许 UAC 激活前台窗口失败: {err:?}"));
        }

        Ok(hook)
    }
}

unsafe extern "system" fn foreground_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    let _ = unsafe { mark_foreground_window(hwnd) };
}

unsafe fn mark_foreground_window(hwnd: HWND) -> windows::core::Result<()> {
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    if root.0.is_null() {
        return Ok(());
    }

    unsafe {
        SetPropW(
            root,
            w!("AllowConsentToStealFocus"),
            Some(HANDLE(ptr::without_provenance_mut(1))),
        )
    }
}
