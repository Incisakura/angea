use std::ffi::{c_void, CString};
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
            epoll::epoll_wait(self.epoll, &mut events, 1000)?;
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

/// D-Bus call to spawn a shell service in systemd
unsafe fn dbus(user: &str, slave: &str) -> Result<()> {
    /// Convert sd_bus_* return value to `nix::Result`
    fn assert(v: i32) -> Result<()> {
        if v < 0 {
            return Err(Errno::from_i32(-v));
        }
        Ok(())
    }
    // Prepare arguments
    let mut args = [
        "/bin/bash\0".as_ptr() as *mut c_char,
        "-l\0".as_ptr() as *mut c_char,
        ptr::null_mut(),
    ];
    let mut envs = [
        "TERM=xterm-256color\0".as_ptr() as *mut c_char,
        ptr::null_mut(),
    ];
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
        "org.freedesktop.systemd1\0".as_ptr() as _,
        "/org/freedesktop/systemd1\0".as_ptr() as _,
        "org.freedesktop.systemd1.Manager\0".as_ptr() as _,
        "StartTransientUnit\0".as_ptr() as _,
    ))?;
    let message = message.assume_init();

    // Append message arguments
    assert(sd_bus_message_append(
        message,
        "ss\0".as_ptr() as _,
        service.as_ptr() as *const _ as *const c_void,
        "fail\0".as_ptr() as *const _ as *const c_void,
    ))?;

    // Enter a(sv)
    assert(sd_bus_message_open_container(
        message,
        'a' as c_char,
        "(sv)\0".as_ptr() as _,
    ))?;
    assert(sd_bus_message_append(
        message,
        "(sv)(sv)(sv)(sv)(sv)(sv)\0".as_ptr() as _,
        "WorkingDirectory\0".as_ptr() as *const _ as *const c_void,
        "s\0".as_ptr() as *const _ as *const c_void,
        "~\0".as_ptr() as *const _ as *const c_void,
        "StandardInput\0".as_ptr() as *const _ as *const c_void,
        "s\0".as_ptr() as *const _ as *const c_void,
        "tty\0".as_ptr() as *const _ as *const c_void,
        "StandardOutput\0".as_ptr() as *const _ as *const c_void,
        "s\0".as_ptr() as *const _ as *const c_void,
        "tty\0".as_ptr() as *const _ as *const c_void,
        "StandardError\0".as_ptr() as *const _ as *const c_void,
        "s\0".as_ptr() as *const _ as *const c_void,
        "tty\0".as_ptr() as *const _ as *const c_void,
        "TTYPath\0".as_ptr() as *const _ as *const c_void,
        "s\0".as_ptr() as *const _ as *const c_void,
        slave.as_ptr() as *const _ as *const c_void, // slave
        "User\0".as_ptr() as *const _ as *const c_void,
        "s\0".as_ptr() as *const _ as *const c_void,
        user.as_ptr() as *const _ as *const c_void, // user
    ))?;

    // Environment
    assert(sd_bus_message_open_container(
        message,
        'r' as _,
        "sv\0".as_ptr() as _,
    ))?;
    assert(sd_bus_message_append(
        message,
        "s\0".as_ptr() as _,
        "Environment\0".as_ptr() as *const _ as *const c_void,
    ))?;
    assert(sd_bus_message_open_container(
        message,
        'v' as _,
        "as\0".as_ptr() as _,
    ))?;
    assert(sd_bus_message_append_strv(message, envs.as_mut_ptr()))?;
    assert(sd_bus_message_close_container(message))?;
    assert(sd_bus_message_close_container(message))?;

    // ExecStart
    assert(sd_bus_message_open_container(
        message,
        'r' as c_char,
        "sv\0".as_ptr() as _,
    ))?;
    assert(sd_bus_message_append(
        message,
        "s\0".as_ptr() as _,
        "ExecStart\0".as_ptr() as *const _ as *const c_void,
    ))?;
    assert(sd_bus_message_open_container(
        message,
        'v' as c_char,
        "a(sasb)\0".as_ptr() as _,
    ))?;
    assert(sd_bus_message_open_container(
        message,
        'a' as c_char,
        "(sasb)\0".as_ptr() as _,
    ))?;
    assert(sd_bus_message_open_container(
        message,
        'r' as c_char,
        "sasb\0".as_ptr() as _,
    ))?;
    assert(sd_bus_message_append(
        message,
        "s\0".as_ptr() as _,
        args[0] as *const _ as *const c_void,
    ))?;
    assert(sd_bus_message_append_strv(message, args.as_mut_ptr()))?;
    assert(sd_bus_message_append(message, "b\0".as_ptr() as _, 1))?; // 1 stands for `true`
    assert(sd_bus_message_close_container(message))?;
    assert(sd_bus_message_close_container(message))?;
    assert(sd_bus_message_close_container(message))?;
    assert(sd_bus_message_close_container(message))?;

    // Exit a(sv)
    assert(sd_bus_message_close_container(message))?;

    // Auxiliary
    assert(sd_bus_message_append(
        message,
        "a(sa(sv))\0".as_ptr() as _,
        0,
    ))?;

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
