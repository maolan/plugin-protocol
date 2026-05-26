use std::io;
use std::os::unix::io::RawFd;
use std::time::Duration;

/// Lightweight cross-process event signalling using Unix pipes.
///
/// Two independent pipes provide bidirectional wake-up:
/// - `daw_to_host`: DAW writes a byte to wake the host process.
/// - `host_to_daw`: Host writes a byte to signal completion.
pub struct EventPair {
    daw_to_host: [RawFd; 2],
    host_to_daw: [RawFd; 2],
}

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

    // --- DAW side ---

    pub fn daw_write_fd(&self) -> RawFd {
        self.daw_to_host[1]
    }

    pub fn daw_read_fd(&self) -> RawFd {
        self.host_to_daw[0]
    }

    /// DAW wakes the host.
    pub fn signal_host(&self) -> io::Result<()> {
        write_byte(self.daw_to_host[1])
    }

    /// DAW waits for host completion (with timeout).
    pub fn wait_host(&self, timeout: Duration) -> io::Result<()> {
        read_byte(self.host_to_daw[0], timeout)
    }

    // --- Host side ---

    pub fn host_read_fd(&self) -> RawFd {
        self.daw_to_host[0]
    }

    pub fn host_write_fd(&self) -> RawFd {
        self.host_to_daw[1]
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
    /// Call this on the DAW after spawning the child.
    pub fn close_daw_unused(&mut self) {
        unsafe {
            libc::close(self.daw_to_host[0]);
            libc::close(self.host_to_daw[1]);
        }
        self.daw_to_host[0] = -1;
        self.host_to_daw[1] = -1;
    }

    /// Close the file descriptors that the host side does not need.
    /// Call this on the host after constructing from inherited fds.
    pub fn close_host_unused(&mut self) {
        unsafe {
            libc::close(self.daw_to_host[1]);
            libc::close(self.host_to_daw[0]);
        }
        self.daw_to_host[1] = -1;
        self.host_to_daw[0] = -1;
    }
}

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

fn write_byte(fd: RawFd) -> io::Result<()> {
    let buf = [1u8];
    let n = unsafe { libc::write(fd, buf.as_ptr().cast(), 1) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

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
    } else {
        Ok(())
    }
}
