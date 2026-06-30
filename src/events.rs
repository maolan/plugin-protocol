use std::io;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::io::RawFd;

/// Lightweight cross-process event signalling.
///
/// Unix: two independent pipes provide bidirectional wake-up.
/// Windows: two named auto-reset events provide bidirectional wake-up.
#[cfg(unix)]
pub struct EventPair {
    daw_to_host: [RawFd; 2],
    host_to_daw: [RawFd; 2],
}

#[cfg(windows)]
pub struct EventPair {
    daw_to_host: *mut std::ffi::c_void,
    host_to_daw: *mut std::ffi::c_void,
    daw_to_host_name: String,
    host_to_daw_name: String,
}

unsafe impl Send for EventPair {}
unsafe impl Sync for EventPair {}

#[cfg(unix)]
impl EventPair {
    /// Create two pipes. Returns `Err` if `pipe(2)` fails.
    pub fn new() -> io::Result<Self> {
        let mut daw_to_host = [0; 2];
        let mut host_to_daw = [0; 2];
        if unsafe { libc::pipe(daw_to_host.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::pipe(host_to_daw.as_mut_ptr()) } != 0 {
            unsafe {
                libc::close(daw_to_host[0]);
                libc::close(daw_to_host[1]);
            }
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            daw_to_host,
            host_to_daw,
        })
    }

    /// # Safety
    /// `daw_to_host_read` and `host_to_daw_write` must be valid,
    /// already-open file descriptors inherited from the parent process.
    pub unsafe fn from_fds(daw_to_host_read: RawFd, host_to_daw_write: RawFd) -> Self {
        let mut pair = Self {
            daw_to_host: [daw_to_host_read, -1],
            host_to_daw: [-1, host_to_daw_write],
        };
        pair.close_host_unused();
        pair
    }

    pub fn daw_write_fd(&self) -> RawFd {
        self.daw_to_host[1]
    }

    pub fn daw_read_fd(&self) -> RawFd {
        self.host_to_daw[0]
    }

    pub fn host_read_fd(&self) -> RawFd {
        self.daw_to_host[0]
    }

    pub fn host_write_fd(&self) -> RawFd {
        self.host_to_daw[1]
    }

    /// DAW wakes the host.
    pub fn signal_host(&self) -> io::Result<()> {
        write_byte(self.daw_to_host[1])
    }

    /// DAW waits for host completion (with timeout).
    pub fn wait_host(&self, timeout: Duration) -> io::Result<()> {
        read_byte(self.host_to_daw[0], timeout)
    }

    /// Host waits for DAW wake (with timeout).
    pub fn wait_daw(&self, timeout: Duration) -> io::Result<()> {
        read_byte(self.daw_to_host[0], timeout)
    }

    /// Host signals completion to DAW.
    pub fn signal_daw(&self) -> io::Result<()> {
        write_byte(self.host_to_daw[1])
    }

    /// Close the file descriptors that the DAW side does not need.
    pub fn close_daw_unused(&mut self) {
        unsafe {
            libc::close(self.daw_to_host[0]);
            libc::close(self.host_to_daw[1]);
        }
        self.daw_to_host[0] = -1;
        self.host_to_daw[1] = -1;
    }

    /// Close the file descriptors that the host side does not need.
    pub fn close_host_unused(&mut self) {
        unsafe {
            libc::close(self.daw_to_host[1]);
            libc::close(self.host_to_daw[0]);
        }
        self.daw_to_host[1] = -1;
        self.host_to_daw[0] = -1;
    }
}

#[cfg(unix)]
impl Drop for EventPair {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.daw_to_host[0]);
            libc::close(self.daw_to_host[1]);
            libc::close(self.host_to_daw[0]);
            libc::close(self.host_to_daw[1]);
        }
    }
}

#[cfg(unix)]
fn write_byte(fd: RawFd) -> io::Result<()> {
    let buf = [1u8];
    let n = unsafe { libc::write(fd, buf.as_ptr().cast(), 1) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn read_byte(fd: RawFd, timeout: Duration) -> io::Result<()> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().clamp(0, i32::MAX as u128) as i32;
    let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    if rc == 0 {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "poll timeout"));
    }
    let mut buf = [0u8; 1];
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), 1) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else if n == 0 {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "event peer closed the pipe",
        ))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
impl EventPair {
    /// Create two named auto-reset events.
    pub fn new() -> io::Result<Self> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Threading::CreateEventW;

        let pid = std::process::id();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let daw_to_host_name = format!("Local\\maolan-d2h-{}-{}", pid, nonce);
        let host_to_daw_name = format!("Local\\maolan-h2d-{}-{}", pid, nonce);

        let d2h_wide: Vec<u16> = daw_to_host_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let h2d_wide: Vec<u16> = host_to_daw_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let daw_to_host = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, d2h_wide.as_ptr()) };
        if daw_to_host.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("CreateEventW failed: {}", unsafe { GetLastError() }),
            ));
        }
        let host_to_daw = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, h2d_wide.as_ptr()) };
        if host_to_daw.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(daw_to_host) };
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("CreateEventW failed: {}", unsafe { GetLastError() }),
            ));
        }

        Ok(Self {
            daw_to_host,
            host_to_daw,
            daw_to_host_name,
            host_to_daw_name,
        })
    }

    /// Reconstruct from event names (host process).
    pub fn from_names(daw_to_host_name: &str, host_to_daw_name: &str) -> io::Result<Self> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Threading::OpenEventW;

        let d2h_wide: Vec<u16> = daw_to_host_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let h2d_wide: Vec<u16> = host_to_daw_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let daw_to_host = unsafe {
            OpenEventW(
                windows_sys::Win32::System::Threading::EVENT_ALL_ACCESS,
                0,
                d2h_wide.as_ptr(),
            )
        };
        if daw_to_host.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("OpenEventW failed: {}", unsafe { GetLastError() }),
            ));
        }
        let host_to_daw = unsafe {
            OpenEventW(
                windows_sys::Win32::System::Threading::EVENT_ALL_ACCESS,
                0,
                h2d_wide.as_ptr(),
            )
        };
        if host_to_daw.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(daw_to_host) };
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("OpenEventW failed: {}", unsafe { GetLastError() }),
            ));
        }

        Ok(Self {
            daw_to_host,
            host_to_daw,
            daw_to_host_name: daw_to_host_name.to_string(),
            host_to_daw_name: host_to_daw_name.to_string(),
        })
    }

    pub fn daw_to_host_name(&self) -> &str {
        &self.daw_to_host_name
    }

    pub fn host_to_daw_name(&self) -> &str {
        &self.host_to_daw_name
    }

    /// DAW wakes the host.
    pub fn signal_host(&self) -> io::Result<()> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Threading::SetEvent;
        if unsafe { SetEvent(self.daw_to_host) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("SetEvent failed: {}", unsafe { GetLastError() }),
            ));
        }
        Ok(())
    }

    /// DAW waits for host completion (with timeout).
    pub fn wait_host(&self, timeout: Duration) -> io::Result<()> {
        self.wait_object(self.host_to_daw, timeout)
    }

    /// Host waits for DAW wake (with timeout).
    pub fn wait_daw(&self, timeout: Duration) -> io::Result<()> {
        self.wait_object(self.daw_to_host, timeout)
    }

    /// Host waits for DAW wake (with timeout) while pumping the Win32 message queue.
    pub fn wait_daw_with_message_pump(&self, timeout: Duration) -> io::Result<()> {
        use windows_sys::Win32::Foundation::{GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
            TranslateMessage,
        };

        let start = std::time::Instant::now();
        let handles = [self.daw_to_host];
        let ms_total = timeout.as_millis().clamp(0, u32::MAX as u128) as u32;

        loop {
            let elapsed = start.elapsed().as_millis().clamp(0, u32::MAX as u128) as u32;
            let remaining = ms_total.saturating_sub(elapsed);

            let rc = unsafe {
                MsgWaitForMultipleObjects(1, handles.as_ptr(), 0, remaining, QS_ALLINPUT)
            };

            if rc == WAIT_OBJECT_0 {
                return Ok(());
            } else if rc == WAIT_OBJECT_0 + 1 {
                unsafe {
                    let mut msg: MSG = std::mem::zeroed();
                    while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            } else if rc == WAIT_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "MsgWaitForMultipleObjects timeout",
                ));
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("MsgWaitForMultipleObjects failed: {}", unsafe {
                        GetLastError()
                    }),
                ));
            }
        }
    }

    /// Host signals completion to DAW.
    pub fn signal_daw(&self) -> io::Result<()> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Threading::SetEvent;
        if unsafe { SetEvent(self.host_to_daw) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("SetEvent failed: {}", unsafe { GetLastError() }),
            ));
        }
        Ok(())
    }

    /// No-op on Windows (events are opened by name).
    pub fn close_daw_unused(&mut self) {}

    /// No-op on Windows (events are opened by name).
    pub fn close_host_unused(&mut self) {}

    fn wait_object(&self, handle: *mut std::ffi::c_void, timeout: Duration) -> io::Result<()> {
        use windows_sys::Win32::Foundation::{GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::WaitForSingleObject;

        let ms = timeout.as_millis().clamp(0, u32::MAX as u128) as u32;
        let rc = unsafe { WaitForSingleObject(handle, ms) };
        if rc == WAIT_OBJECT_0 {
            Ok(())
        } else if rc == WAIT_TIMEOUT {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "WaitForSingleObject timeout",
            ))
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("WaitForSingleObject failed: {}", unsafe { GetLastError() }),
            ))
        }
    }
}

#[cfg(windows)]
impl Drop for EventPair {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        if !self.daw_to_host.is_null() {
            unsafe { CloseHandle(self.daw_to_host) };
        }
        if !self.host_to_daw.is_null() {
            unsafe { CloseHandle(self.host_to_daw) };
        }
    }
}
