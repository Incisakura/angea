use std::collections::HashMap;
use std::env;
use std::ffi::c_void;
use std::mem;
use std::mem::MaybeUninit;
use std::os::raw::c_char;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::prelude::IntoRawFd;
use std::ptr;

use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::{posix_openpt, ptsname_r, unlockpt};
use nix::sys::epoll::{self, EpollEvent, EpollFlags, EpollOp};
use nix::sys::signal::{sigprocmask, SigmaskHow, Signal};
use nix::sys::signalfd::{SigSet, SignalFd};
use nix::sys::termios::{self, SetArg, Termios};
use nix::unistd::{read, write};
use nix::Result;

use libsystemd_sys::bus::*;

/// Init and get pty master fd
pub fn get_pty(user: &str) -> Result<RawFd> {
    // pty peer
    let pty = posix_openpt(OFlag::O_NONBLOCK | OFlag::O_RDWR | OFlag::O_NOCTTY)?;
    unlockpt(&pty)?;
    let pts = ptsname_r(&pty)?;

    // dbus method call
    unsafe { dbus(user, pts.as_str())? };
    window_resize(pty.as_raw_fd())?;
    Ok(pty.into_raw_fd())
}

const SD_BUS_TYPE_ARRAY: c_char = 'a' as c_char;

const SD_BUS_TYPE_VARIANT: c_char = 'v' as c_char;

const SD_BUS_TYPE_STRUCT: c_char = 'r' as c_char;

#[rustfmt::skip]
/// D-Bus call to spawn a shell service in systemd
unsafe fn dbus(user: &str, slave: &str) -> Result<()> {
    let os_envs: HashMap<String, String> = env::vars().collect();
    // Arguments
    let mut raw_args = os_envs.get("ANGEA_ARGS").map(|v|
        v.split_whitespace().map(append_null).collect()
    ).unwrap_or_else(|| vec![
        "/bin/bash\0".to_string(),
        "-l\0".to_string(),
    ]);
    let mut args: Vec<*mut c_char> = raw_args.iter_mut().map(|s| s.as_mut_ptr().cast()).collect();
    args.push(ptr::null_mut());

    // Environment Variables
    let mut raw_envs = Vec::new();
    if let Some(s) = os_envs.get("ANGEA_ENVS") {
        raw_envs.extend(s.split(',').map(append_null));
    }
    raw_envs.extend(
        os_envs
            .get("ANGEA_ENV_INHERIT")
            .map(|s| s.as_str())
            .unwrap_or("TERM")
            .split(',')
            .filter_map(|k| os_envs.get(k).map(|v| format!("{}={}\0", k, v))),
    );
    let mut envs: Vec<*mut c_char> = raw_envs.iter_mut().map(|s| s.as_mut_ptr().cast()).collect();
    envs.push(ptr::null_mut());

    let pts_id = slave.trim_start_matches("/dev/pts/");
    let service = format!("angea-shell@{}.service\0", pts_id);
    let slave = append_null(slave);
    let user = append_null(user);

    // Init bus and message
    let mut bus = MaybeUninit::uninit();
    assert(sd_bus_default_system(bus.as_mut_ptr()))?;
    let bus = bus.assume_init();

    let mut message = MaybeUninit::uninit();
    assert(sd_bus_message_new_method_call(
        bus,
        message.as_mut_ptr(),
        char("org.freedesktop.systemd1\0"),
        char("/org/freedesktop/systemd1\0"),
        char("org.freedesktop.systemd1.Manager\0"),
        char("StartTransientUnit\0"),
    ))?;
    let message = message.assume_init();

    // Append message arguments
    assert(sd_bus_message_append(
        message,
        char("ss\0"),
        void(service.as_str()),
        void("fail\0"),
    ))?;

    // Enter a(sv)
    assert(sd_bus_message_open_container(message, SD_BUS_TYPE_ARRAY, char("(sv)\0")))?;
    assert(sd_bus_message_append(
        message,
        char("(sv)(sv)(sv)(sv)(sv)(sv)(sv)\0"),
        void("Description\0"),      void("s\0"), void("Angea Shell Serivice\0"),
        void("WorkingDirectory\0"), void("s\0"), void("~\0"),
        void("StandardOutput\0"),   void("s\0"), void("tty\0"),
        void("StandardInput\0"),    void("s\0"), void("tty\0"),
        void("StandardError\0"),    void("s\0"), void("tty\0"),
        void("TTYPath\0"),          void("s\0"), void(slave.as_str()),
        void("User\0"),             void("s\0"), void(user.as_str()),
    ))?;

    // Environment
    assert(sd_bus_message_open_container(message, SD_BUS_TYPE_STRUCT, char("sv\0")))?;
    assert(sd_bus_message_append(message, char("s\0"), void("Environment\0")))?;
    assert(sd_bus_message_open_container(message, SD_BUS_TYPE_VARIANT, char("as\0")))?;
    assert(sd_bus_message_append_strv(message, envs.as_mut_ptr()))?;
    assert(sd_bus_message_close_container(message))?;
    assert(sd_bus_message_close_container(message))?;

    // ExecStart
    assert(sd_bus_message_open_container(message, SD_BUS_TYPE_STRUCT, char("sv\0")))?;
    assert(sd_bus_message_append(message, char("s\0"), void("ExecStart\0")))?;
    assert(sd_bus_message_open_container(message, SD_BUS_TYPE_VARIANT, char("a(sasb)\0")))?;
    assert(sd_bus_message_open_container(message, SD_BUS_TYPE_ARRAY, char("(sasb)\0")))?;
    assert(sd_bus_message_open_container(message, SD_BUS_TYPE_STRUCT, char("sasb\0")))?;
    assert(sd_bus_message_append(message, char("s\0"), void(args[0])))?;
    assert(sd_bus_message_append_strv(message, args.as_mut_ptr()))?;
    assert(sd_bus_message_append(message, char("b\0"), 1))?; // 1 stands for `true`
    assert(sd_bus_message_close_container(message))?;
    assert(sd_bus_message_close_container(message))?;
    assert(sd_bus_message_close_container(message))?;
    assert(sd_bus_message_close_container(message))?;
    // Exit a(sv)
    assert(sd_bus_message_close_container(message))?;

    // Auxiliary
    assert(sd_bus_message_append(message, char("a(sa(sv))\0"), 0))?;

    // Send message
    assert(sd_bus_call(
        bus,
        message,
        0,
        ptr::null_mut(),
        ptr::null_mut(),
    ))?;

    // Free pointer resource
    sd_bus_close(bus);
    sd_bus_unref(bus);
    sd_bus_message_unref(message);

    Ok(())
}

/// Convert sd_bus_* return value to `nix::Result`
fn assert(v: i32) -> Result<()> {
    if v < 0 {
        return Err(Errno::from_i32(-v));
    }
    Ok(())
}

/// Convert to `*const c_char`
fn char<T: StrPtrCast>(v: T) -> *const c_char {
    v.as_char_ptr()
}

/// Convert to `*const c_void`
fn void<T: StrPtrCast>(v: T) -> *const c_void {
    v.as_void_ptr()
}

trait StrPtrCast: Sized {
    fn as_char_ptr(&self) -> *const c_char;

    fn as_void_ptr(&self) -> *const c_void {
        self.as_char_ptr().cast()
    }
}

impl StrPtrCast for &'_ str {
    fn as_char_ptr(&self) -> *const c_char {
        self.as_ptr().cast()
    }
}

impl StrPtrCast for *mut c_char {
    fn as_char_ptr(&self) -> *const c_char {
        *self
    }
}

pub struct PTYForward {
    epoll: RawFd,
    master: RawFd,
    signal_fd: SignalFd,
    stdin_origin: Termios,
    stdout_origin: Termios,
}

impl PTYForward {
    /// Setup ptyforward event
    pub fn new(master: RawFd) -> Result<PTYForward> {
        let mut sig_set = SigSet::empty();
        sig_set.add(Signal::SIGWINCH);
        sigprocmask(SigmaskHow::SIG_SETMASK, Some(&sig_set), None)?;
        let signal_fd = SignalFd::new(&sig_set)?;
        let sig_fd = signal_fd.as_raw_fd();

        let epoll = epoll::epoll_create()?;
        let mut stdin_event = EpollEvent::new(EpollFlags::EPOLLIN, 0);
        let mut master_event = EpollEvent::new(EpollFlags::EPOLLIN, 1);
        let mut sig_event = EpollEvent::new(EpollFlags::EPOLLIN, 2);
        epoll::epoll_ctl(
            epoll,
            EpollOp::EpollCtlAdd,
            libc::STDIN_FILENO,
            &mut stdin_event,
        )?;
        epoll::epoll_ctl(epoll, EpollOp::EpollCtlAdd, master, &mut master_event)?;
        epoll::epoll_ctl(epoll, EpollOp::EpollCtlAdd, sig_fd, &mut sig_event)?;

        wait_service(master)?;
        Self::set_nonblock(libc::STDIN_FILENO, true)?;
        let stdin_origin = Self::set_raw_termios(libc::STDIN_FILENO)?;
        let stdout_origin = Self::set_raw_termios(libc::STDOUT_FILENO)?;
        Ok(PTYForward {
            epoll,
            master,
            signal_fd,
            stdin_origin,
            stdout_origin,
        })
    }

    /// Wait epoll event
    ///
    /// # Errors
    ///
    /// Unexpected I/O error
    pub fn wait(&mut self) -> Result<()> {
        let mut events: Vec<EpollEvent> = Vec::with_capacity(64);
        let mut buf = [0; 256];
        'epoll: loop {
            events.clear();
            unsafe { events.set_len(64) };
            epoll::epoll_wait(self.epoll, &mut events, -1)?;
            for ev in &events {
                match ev.data() {
                    0 => {
                        // stdin => master
                        match read(libc::STDIN_FILENO, &mut buf) {
                            Ok(n) => write(self.master, &buf[..n])?,
                            Err(Errno::EWOULDBLOCK) => continue,
                            Err(e) => return Err(e),
                        };
                    }
                    1 => {
                        // master => stdout
                        match read(self.master, &mut buf) {
                            Ok(n) => write(libc::STDOUT_FILENO, &buf[..n])?,
                            Err(Errno::EWOULDBLOCK) => continue,
                            Err(Errno::EIO) => break 'epoll,
                            Err(e) => return Err(e),
                        };
                    }
                    2 => {
                        // signal
                        self.signal_fd.read_signal()?;
                        window_resize(self.master)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Recovery termios and non-block status
    ///
    /// # Errors
    ///
    /// Unexpected I/O error. But it should be no error because `PTYForward::new()` is ok.
    pub fn disconnect(self) -> Result<()> {
        termios::tcsetattr(libc::STDOUT_FILENO, SetArg::TCSANOW, &self.stdout_origin)?;
        termios::tcsetattr(libc::STDIN_FILENO, SetArg::TCSANOW, &self.stdin_origin)?;
        Self::set_nonblock(libc::STDIN_FILENO, false)?;
        Ok(())
    }

    /// Set I/O non-block
    /// # Errors
    ///
    /// Unexpected I/O error
    fn set_nonblock(fd: RawFd, nonblock: bool) -> Result<()> {
        let bits = fcntl(fd, FcntlArg::F_GETFL)?;
        let mut flags = unsafe { OFlag::from_bits_unchecked(bits) };
        flags = if nonblock {
            flags | OFlag::O_NONBLOCK
        } else {
            flags & !OFlag::O_NONBLOCK
        };
        fcntl(fd, FcntlArg::F_SETFL(flags))?;
        Ok(())
    }

    /// Set raw termios config, return origin config for recovery
    fn set_raw_termios(fd: RawFd) -> Result<Termios> {
        let stdin_origin = termios::tcgetattr(fd)?;
        let mut stdin_attr = stdin_origin.clone();
        termios::cfmakeraw(&mut stdin_attr);
        termios::tcsetattr(fd, SetArg::TCSANOW, &stdin_attr)?;
        Ok(stdin_origin)
    }
}

fn wait_service(master: RawFd) -> Result<()> {
    let mut buf = [0; 8];
    for _ in 0..30 {
        if let Ok(n) = read(master, &mut buf) {
            write(libc::STDOUT_FILENO, &buf[..n])?;
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(Errno::ETIMEDOUT)
}

fn window_resize(master: RawFd) -> Result<()> {
    unsafe {
        let mut size: libc::winsize = mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == -1
            || libc::ioctl(master, libc::TIOCSWINSZ, &size) == -1
        {
            return Err(Errno::last());
        }
    }
    Ok(())
}

fn append_null(str: &str) -> String {
    let mut str = String::from(str);
    str.push('\0');
    str
}
