use std::collections::HashMap;
use std::env;
use std::ffi::{c_void, CStr, CString};
use std::mem;
use std::mem::MaybeUninit;
use std::os::raw::c_char;
use std::os::unix::io::{AsRawFd, RawFd};
use std::ptr;
use std::thread;
use std::time::Duration;

use libc::{winsize, TIOCGWINSZ, TIOCSWINSZ};
use nix::errno::Errno;
use nix::fcntl::{fcntl, readlink, FcntlArg, OFlag};
use nix::pty::openpty;
use nix::sys::epoll::{self, EpollEvent, EpollFlags, EpollOp};
use nix::sys::signal::{sigprocmask, SigmaskHow, Signal};
use nix::sys::signalfd::{SigSet, SignalFd};
use nix::sys::termios::{self, SetArg, Termios};
use nix::unistd::{close, read, write};
use nix::Result;

use libsystemd_sys::bus::*;

/// stdin fd
const STDIN: RawFd = libc::STDIN_FILENO;

/// stdout fd
const STDOUT: RawFd = libc::STDOUT_FILENO;

/// Init and get pty master fd
pub fn get_pty(user: &str) -> Result<RawFd> {
    // init pty peer
    let pty = openpty(None, None)?;
    let path = readlink(format!("/proc/self/fd/{}", pty.slave).as_str())?;
    close(pty.slave)?; // no longer used, close

    wait_dbus(user, path.to_str().unwrap())?;
    wait_master(pty.master)?;

    Ok(pty.master)
}

/// Wait systemd init
fn wait_dbus(user: &str, slave: &str) -> Result<()> {
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(100));
        match unsafe { dbus(user, slave) } {
            Ok(_) => return Ok(()),
            Err(Errno::ECONNREFUSED) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(Errno::ETIMEDOUT)
}

/// Wait master for readable
fn wait_master(master: RawFd) -> Result<()> {
    let mut buff = [0; 128];
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(100));
        match read(master, &mut buff) {
            Ok(_) => return Ok(()),
            Err(Errno::EIO) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(Errno::ETIMEDOUT)
}

pub struct PTYForward {
    epoll: RawFd,
    master_fd: RawFd,
    stdin_origin: Termios,
    stdout_origin: Termios,
    signal_fd: SignalFd,
}

impl PTYForward {
    /// Setup ptyforward event
    pub fn new(master_fd: RawFd) -> Result<PTYForward> {
        let mut sig_set = SigSet::empty();
        sig_set.add(Signal::SIGWINCH);
        sigprocmask(SigmaskHow::SIG_SETMASK, Some(&sig_set), None)?;
        let signal_fd = SignalFd::new(&sig_set)?;
        let sig_fd = signal_fd.as_raw_fd();

        let epoll = epoll::epoll_create()?;
        let mut stdin_event = EpollEvent::new(EpollFlags::EPOLLIN, 0);
        let mut master_event = EpollEvent::new(EpollFlags::EPOLLIN, 1);
        let mut sig_event = EpollEvent::new(EpollFlags::EPOLLIN, 2);
        epoll::epoll_ctl(epoll, EpollOp::EpollCtlAdd, STDIN, &mut stdin_event)?;
        epoll::epoll_ctl(epoll, EpollOp::EpollCtlAdd, master_fd, &mut master_event)?;
        epoll::epoll_ctl(epoll, EpollOp::EpollCtlAdd, sig_fd, &mut sig_event)?;

        let stdin_origin = Self::set_raw_termios(STDIN)?;
        let stdout_origin = Self::set_raw_termios(STDOUT)?;
        Self::set_nonblock(STDIN, true)?;
        Self::set_nonblock(master_fd, true)?;

        let f = PTYForward {
            epoll,
            master_fd,
            stdin_origin,
            stdout_origin,
            signal_fd,
        };
        f.window_resize()?;
        Ok(f)
    }

    /// Wait epoll event
    ///
    /// # Errors
    ///
    /// Unexpected I/O error
    pub fn wait(&mut self) -> Result<()> {
        let mut events: Vec<EpollEvent> = Vec::with_capacity(256);
        'epoll: loop {
            events.clear();
            unsafe { events.set_len(256) };
            epoll::epoll_wait(self.epoll, &mut events, -1)?;
            for ev in &events {
                match ev.data() {
                    0 => {
                        // stdin => master
                        Self::handle_io_event(STDIN, self.master_fd)?;
                    }
                    1 => {
                        // master => stdout
                        if !Self::handle_io_event(self.master_fd, STDOUT)? {
                            break 'epoll;
                        }
                    }
                    2 => {
                        // signal
                        self.signal_fd.read_signal()?;
                        self.window_resize()?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Set master winsize with stdout winsize.
    fn window_resize(&self) -> Result<()> {
        unsafe {
            let mut window: winsize = mem::zeroed();
            Errno::result(libc::ioctl(STDOUT, TIOCGWINSZ, &mut window))?;
            Errno::result(libc::ioctl(self.master_fd, TIOCSWINSZ, &window))?;
        }
        Ok(())
    }

    /// Handle I/O event, forward data `from => to`
    fn handle_io_event(from: RawFd, to: RawFd) -> Result<bool> {
        let mut buffer = [0; 4096];
        loop {
            match read(from, &mut buffer) {
                Ok(s) => {
                    write(to, &buffer[..s])?;
                }
                Err(Errno::EWOULDBLOCK) => return Ok(true),
                Err(Errno::EIO) => return Ok(false),
                Err(e) => return Err(e),
            }
        }
    }

    /// Recovery termios and non-block status
    ///
    /// # Errors
    ///
    /// Unexpected I/O error. But it should be no error because `PTYForward::new()` is ok.
    pub fn disconnect(self) -> Result<()> {
        termios::tcsetattr(STDOUT, SetArg::TCSANOW, &self.stdout_origin)?;
        termios::tcsetattr(STDIN, SetArg::TCSANOW, &self.stdin_origin)?;
        Self::set_nonblock(STDIN, false)?;
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

const SD_BUS_TYPE_ARRAY: c_char = 'a' as c_char;

const SD_BUS_TYPE_VARIANT: c_char = 'v' as c_char;

const SD_BUS_TYPE_STRUCT: c_char = 'r' as c_char;

#[rustfmt::skip]
/// D-Bus call to spawn a shell service in systemd
unsafe fn dbus(user: &str, slave: &str) -> Result<()> {
    let envs: HashMap<String, String> = env::vars().collect();
    // Process Arguments
    let mut raw_args: Vec<String> = envs.get("ARGS").map(|v| {
        let mut vec = Vec::new();
        for s in v.split_whitespace() {
            vec.push(format!("{}\n", s));
        }
        vec
    }).unwrap_or(vec![
        "/bin/bash\0".to_string(),
        "-l\0".to_string(),
    ]);
    let mut args: Vec<*mut c_char> = Vec::new();
    for raw_arg in raw_args.iter_mut() {
        args.push(raw_arg.as_mut_ptr().cast())
    }
    args.push(ptr::null_mut());

    // Environment Variables
    let env_inherit: Option<Vec<&str>> = envs.get("ENV_INHERIT").map(|v| v.split(",").collect());
    let mut raw_envs : Vec<String> = envs.get("ENVS").map(|v| {
        let mut vec = Vec::new();
        for s in v.split(",") {
            vec.push(format!("{}\n", s));
        }
        if let Some(s) = env_inherit {
            for k in s  {
                if let Some(s) = envs.get(k) {
                    vec.push(format!("{}={}\0", k, s));
                }
            }
        } 
        vec
    }).unwrap_or(Vec::new());
    let mut envs: Vec<*mut c_char> = Vec::new();
    for raw_env in raw_envs.iter_mut() {
        envs.push(raw_env.as_mut_ptr().cast())
    }
    envs.push(ptr::null_mut());

    let pts_id = slave.trim_start_matches("/dev/pts/");
    let service =
        CString::new(format!("angea-shell@{}.service", pts_id)).map_err(|_| Errno::EINVAL)?;
    let slave = CString::new(slave).map_err(|_| Errno::EINVAL)?;
    let user = CString::new(user).map_err(|_| Errno::EINVAL)?;

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
        void(service.as_c_str()),
        void("fail\0"),
    ))?;

    // Enter a(sv)
    assert(sd_bus_message_open_container(message, SD_BUS_TYPE_ARRAY, char("(sv)\0")))?;
    assert(sd_bus_message_append(
        message,
        char("(sv)(sv)(sv)(sv)(sv)(sv)\0"),
        void("WorkingDirectory\0"), void("s\0"), void("~\0"),
        void("StandardOutput\0"),   void("s\0"), void("tty\0"),
        void("StandardInput\0"),    void("s\0"), void("tty\0"),
        void("StandardError\0"),    void("s\0"), void("tty\0"),
        void("TTYPath\0"),          void("s\0"), void(slave.as_c_str()),
        void("User\0"),             void("s\0"), void(user.as_c_str()),
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
    sd_bus_close_unref(bus);
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

/// Convert tring like type to `*const c_char`
fn char<T: StrLike>(v: T) -> *const c_char {
    v.as_char_ptr()
}

/// Convert tring like type to `*const c_void`
fn void<T: StrLike>(v: T) -> *const c_void {
    v.as_void_ptr()
}

trait StrLike: Sized {
    fn as_char_ptr(&self) -> *const c_char;

    fn as_void_ptr(&self) -> *const c_void {
        self.as_char_ptr().cast()
    }
}

impl StrLike for &CStr {
    fn as_char_ptr(&self) -> *const c_char {
        self.as_ptr()
    }
}

impl StrLike for &'static str {
    fn as_char_ptr(&self) -> *const c_char {
        self.as_ptr().cast()
    }
}

impl StrLike for *mut c_char {
    fn as_char_ptr(&self) -> *const c_char {
        *self
    }
}
