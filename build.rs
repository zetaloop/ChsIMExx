fn main() {
    #[cfg(windows)]
    {
        use std::ffi::c_void;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
            fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
            fn SetEvent(handle: *mut c_void) -> i32;
            fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
            fn ReleaseMutex(handle: *mut c_void) -> i32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }

        let event_name = wide("Global\\ChsIMExxStop");
        let mutex_name = wide("Global\\ChsIMExxMutex");

        unsafe {
            let event = OpenEventW(0x0002, 0, event_name.as_ptr());
            if !event.is_null() {
                SetEvent(event);
                CloseHandle(event);

                let mutex = OpenMutexW(0x00100001, 0, mutex_name.as_ptr());
                if !mutex.is_null() {
                    let wait = WaitForSingleObject(mutex, 5000);
                    if wait == 0 || wait == 0x80 {
                        ReleaseMutex(mutex);
                    }
                    CloseHandle(mutex);
                }
            }
        }
    }
}
