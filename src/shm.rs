#[cfg(unix)]
use std::ffi::CString;

/// Owned mapping of a shared-memory segment.
pub struct ShmMapping {
    ptr: *mut u8,
    size: usize,
    #[allow(dead_code)]
    name: String,
    #[cfg(windows)]
    handle: *mut std::ffi::c_void,
}

// Safety: ShmMapping is Send+Sync because the mapped memory is process-shared.
unsafe impl Send for ShmMapping {}
unsafe impl Sync for ShmMapping {}

impl ShmMapping {
    /// Create a new shared-memory segment, truncate to `size`, and map it.
    #[cfg(unix)]
    pub fn create(name: &str, size: usize) -> Result<Self, String> {
        let c_name = CString::new(name).map_err(|e| e.to_string())?;
        let fd = unsafe { libc::shm_open(c_name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o644) };
        if fd < 0 {
            return Err(format!(
                "shm_open({}, O_CREAT|O_RDWR) failed: {:?}",
                name,
                std::io::Error::last_os_error()
            ));
        }
        if unsafe { libc::ftruncate(fd, size as libc::off_t) } < 0 {
            unsafe { libc::close(fd) };
            return Err(format!(
                "ftruncate failed: {:?}",
                std::io::Error::last_os_error()
            ));
        }
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        unsafe { libc::close(fd) };
        if ptr == libc::MAP_FAILED {
            unsafe { libc::shm_unlink(c_name.as_ptr()) };
            return Err(format!(
                "mmap failed: {:?}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            ptr: ptr as *mut u8,
            size,
            name: name.to_string(),
        })
    }

    /// Open an existing shared-memory segment and map it.
    #[cfg(unix)]
    pub fn open_existing(name: &str, size: usize) -> Result<Self, String> {
        let c_name = CString::new(name).map_err(|e| e.to_string())?;
        let fd = unsafe { libc::shm_open(c_name.as_ptr(), libc::O_RDWR, 0) };
        if fd < 0 {
            return Err(format!(
                "shm_open({}, O_RDWR) failed: {:?}",
                name,
                std::io::Error::last_os_error()
            ));
        }
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        unsafe { libc::close(fd) };
        if ptr == libc::MAP_FAILED {
            return Err(format!(
                "mmap failed: {:?}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            ptr: ptr as *mut u8,
            size,
            name: name.to_string(),
        })
    }

    /// Create a new pagefile-backed shared-memory segment on Windows.
    #[cfg(windows)]
    pub fn create(name: &str, size: usize) -> Result<Self, String> {
        use windows_sys::Win32::Foundation::{GetLastError, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE,
        };

        let wide_name: Vec<u16> = format!("Local\\{}", name)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null_mut(),
                PAGE_READWRITE,
                0,
                size as u32,
                wide_name.as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(format!("CreateFileMappingW failed: {}", unsafe {
                GetLastError()
            }));
        }
        let ptr = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) };
        if ptr.Value.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
            return Err(format!("MapViewOfFile failed: {}", unsafe {
                GetLastError()
            }));
        }
        Ok(Self {
            ptr: ptr.Value as *mut u8,
            size,
            name: name.to_string(),
            handle,
        })
    }

    /// Open an existing shared-memory segment on Windows.
    #[cfg(windows)]
    pub fn open_existing(name: &str, size: usize) -> Result<Self, String> {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        use windows_sys::Win32::System::Memory::{
            FILE_MAP_ALL_ACCESS, MapViewOfFile, OpenFileMappingW,
        };

        let wide_name: Vec<u16> = format!("Local\\{}", name)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, wide_name.as_ptr()) };
        if handle.is_null() {
            return Err(format!("OpenFileMappingW failed: {}", unsafe {
                GetLastError()
            }));
        }
        let ptr = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) };
        if ptr.Value.is_null() {
            unsafe { CloseHandle(handle) };
            return Err(format!("MapViewOfFile failed: {}", unsafe {
                GetLastError()
            }));
        }
        Ok(Self {
            ptr: ptr.Value as *mut u8,
            size,
            name: name.to_string(),
            handle,
        })
    }

    /// Raw pointer to the start of the mapping.
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Size of the mapping in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Name used to create/open the segment.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Unlink the underlying POSIX shared-memory object.
    #[cfg(unix)]
    pub fn unlink(name: &str) -> Result<(), String> {
        let c_name = CString::new(name).map_err(|e| e.to_string())?;
        let res = unsafe { libc::shm_unlink(c_name.as_ptr()) };
        if res < 0 {
            Err(format!(
                "shm_unlink failed: {:?}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    pub fn unlink(_name: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for ShmMapping {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.ptr != libc::MAP_FAILED as *mut u8 {
            unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size) };
        }
    }
}

#[cfg(windows)]
impl Drop for ShmMapping {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Memory::UnmapViewOfFile;
        if !self.ptr.is_null() {
            unsafe {
                UnmapViewOfFile(
                    windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: self.ptr as *mut std::ffi::c_void,
                    },
                )
            };
        }
        if !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
        }
    }
}
