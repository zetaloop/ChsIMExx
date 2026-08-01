use std::{ffi::c_void, ptr};

use windows::{
    Win32::{
        Foundation::{HANDLE, HWND},
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{
                EVENT_SYSTEM_FOREGROUND, GA_ROOT, GetAncestor, GetForegroundWindow,
                SPI_GETFOREGROUNDLOCKTIMEOUT, SPI_SETFOREGROUNDLOCKTIMEOUT, SPIF_SENDCHANGE,
                SPIF_UPDATEINIFILE, SetPropW, SystemParametersInfoW, WINEVENT_OUTOFCONTEXT,
            },
        },
    },
    core::{Error, w},
};

pub fn allow_foreground_activation() -> Result<HWINEVENTHOOK, String> {
    allow_regular_foreground_activation()?;
    allow_consent_foreground_activation()
}

fn allow_regular_foreground_activation() -> Result<(), String> {
    unsafe {
        SystemParametersInfoW(
            SPI_SETFOREGROUNDLOCKTIMEOUT,
            0,
            None,
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
        .map_err(|err| format!("关闭前台窗口锁定超时失败: {err:?}"))?;

        let mut timeout = u32::MAX;
        SystemParametersInfoW(
            SPI_GETFOREGROUNDLOCKTIMEOUT,
            0,
            Some((&raw mut timeout).cast::<c_void>()),
            Default::default(),
        )
        .map_err(|err| format!("读取前台窗口锁定超时失败: {err:?}"))?;

        if timeout != 0 {
            return Err(format!("前台窗口锁定超时仍为 {timeout} 毫秒"));
        }
    }

    Ok(())
}

fn allow_consent_foreground_activation() -> Result<HWINEVENTHOOK, String> {
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
