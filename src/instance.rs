use std::mem::size_of;

use windows::{
    Win32::{
        Foundation::{
            CloseHandle, HANDLE, HLOCAL, LocalFree, WAIT_ABANDONED, WAIT_FAILED, WAIT_OBJECT_0,
            WAIT_TIMEOUT,
        },
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
        System::Threading::{
            CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, MUTEX_MODIFY_STATE, OpenEventW,
            OpenMutexW, ReleaseMutex, ResetEvent, SYNCHRONIZATION_SYNCHRONIZE, SetEvent,
            WaitForSingleObject,
        },
    },
    core::{self, PCWSTR, w},
};

pub const STOP_EVENT_NAME: PCWSTR = w!("Global\\ChsIMExxStop");
pub const INSTANCE_MUTEX_NAME: PCWSTR = w!("Global\\ChsIMExxMutex");

pub enum InstanceState {
    Fresh,
    Restarted,
}

pub struct InstanceGuard {
    mutex: HANDLE,
    stop_event: HANDLE,
    owns_mutex: bool,
}

impl InstanceGuard {
    pub fn new() -> core::Result<Self> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w!("D:(A;;GA;;;WD)"), // Everyone: Generic All
                1,
                &mut sd,
                None,
            )?;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };

        let mutex = unsafe { CreateMutexW(Some(&sa), false, INSTANCE_MUTEX_NAME)? };
        let stop_event = unsafe { CreateEventW(Some(&sa), true, false, STOP_EVENT_NAME)? };

        if !sd.0.is_null() {
            unsafe { LocalFree(Some(HLOCAL(sd.0))) };
        }

        Ok(Self {
            mutex,
            stop_event,
            owns_mutex: false,
        })
    }

    pub fn acquire(&mut self) -> Result<InstanceState, String> {
        let wait = unsafe { WaitForSingleObject(self.mutex, 0) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            self.owns_mutex = true;
            unsafe {
                ResetEvent(self.stop_event).map_err(|_| "无法重置停止事件".to_string())?;
            }
            return Ok(InstanceState::Fresh);
        }

        if wait == WAIT_TIMEOUT {
            unsafe {
                SetEvent(self.stop_event).map_err(|_| "无法通知旧实例退出".to_string())?;
            }

            let wait = unsafe { WaitForSingleObject(self.mutex, 10_000) };
            if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
                self.owns_mutex = true;
                unsafe {
                    ResetEvent(self.stop_event).map_err(|_| "无法重置停止事件".to_string())?;
                }
                return Ok(InstanceState::Restarted);
            } else if wait == WAIT_TIMEOUT {
                return Err("等待旧实例退出超时".into());
            } else {
                return Err("等待旧实例退出失败".into());
            }
        }

        if wait == WAIT_FAILED {
            Err("检测实例状态失败".into())
        } else {
            Err("未知的等待状态".into())
        }
    }

    pub fn stop_event(&self) -> HANDLE {
        self.stop_event
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ResetEvent(self.stop_event);
            if self.owns_mutex {
                let _ = ReleaseMutex(self.mutex);
            }
            let _ = CloseHandle(self.mutex);
            let _ = CloseHandle(self.stop_event);
        }
    }
}

pub fn signal_shutdown_request() -> Result<Option<()>, String> {
    unsafe {
        let event = OpenEventW(
            EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
            false,
            STOP_EVENT_NAME,
        )
        .ok();

        if let Some(event) = event {
            SetEvent(event).map_err(|_| "无法发送停止请求".to_string())?;
            let _ = CloseHandle(event);

            if let Ok(mutex) = OpenMutexW(
                SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
                false,
                INSTANCE_MUTEX_NAME,
            ) {
                let wait = WaitForSingleObject(mutex, 10_000);
                if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
                    let _ = ReleaseMutex(mutex);
                }
                let _ = CloseHandle(mutex);
            }
            Ok(Some(()))
        } else {
            Ok(None)
        }
    }
}
