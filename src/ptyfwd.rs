use std::io;
use std::mem;
use std::os::unix::io::{AsRawFd, RawFd};

use libc::{winsize, TIOCGWINSZ, TIOCSWINSZ};
use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::epoll::{self, EpollEvent, EpollFlags, EpollOp};
use nix::sys::signal::{sigprocmask, SigmaskHow, Signal};
use nix::sys::signalfd::{SigSet, SignalFd};
use nix::sys::termios::{self, SetArg, Termios};
use nix::unistd::{read, write};
use nix::Result;

pub struct PTYForward {
    epoll: RawFd,

    stdin_fd: RawFd,
    stdout_fd: RawFd,
    master_fd: RawFd,

    stdin_readable: bool,
    master_readable: bool,

    in_buffer: [u8; 4096],
    out_buffer: [u8; 4096],

    stdin_origin: Termios,
    stdout_origin: Termios,
    signal_fd: SignalFd,
}

impl PTYForward {
    pub fn new(master_fd: RawFd) -> Result<Self> {
        let epoll = epoll::epoll_create()?;

        let stdin_fd = io::stdin().as_raw_fd();
        let stdout_fd = io::stdout().as_raw_fd();
        let (stdin_origin, stdout_origin) = Self::set_termios(stdin_fd, stdout_fd)?;

        let mut sig_set = SigSet::empty();
        sig_set.add(Signal::SIGWINCH);
        sigprocmask(SigmaskHow::SIG_SETMASK, Some(&sig_set), None)?;
        let signal_fd = SignalFd::new(&sig_set)?;

        let mut stdin_event = EpollEvent::new(EpollFlags::EPOLLIN, 0);
        let mut master_event = EpollEvent::new(EpollFlags::EPOLLIN, 1);
        let mut sig_event = EpollEvent::new(EpollFlags::EPOLLIN, 2);
        epoll::epoll_ctl(epoll, EpollOp::EpollCtlAdd, stdin_fd, &mut stdin_event)?;
        epoll::epoll_ctl(epoll, EpollOp::EpollCtlAdd, master_fd, &mut master_event)?;
        epoll::epoll_ctl(epoll, EpollOp::EpollCtlAdd, signal_fd.as_raw_fd(), &mut sig_event)?;

        let f = Self {
            epoll,
            stdin_fd,
            stdout_fd,
            master_fd,
            stdin_readable: false,
            master_readable: false,
            in_buffer: [0; 4096],
            out_buffer: [0; 4096],
            stdin_origin,
            stdout_origin,
            signal_fd,
        };
        f.set_nonblock(true)?;
        f.window_resize()?;

        Ok(f)
    }

    pub fn wait(&mut self) {
        let mut events: Vec<EpollEvent> = Vec::with_capacity(256);
        'epoll: loop {
            events.clear();
            unsafe { events.set_len(256) };
            epoll::epoll_wait(self.epoll, &mut events, 1000).unwrap();
            for ev in &events {
                match ev.data() {
                    0 => {
                        // stdin
                        self.stdin_readable = true;
                        if !self.handle_io_event().unwrap_or_else(|e| {
                            // panic with unexpected error
                            panic!("{}", e.desc());
                        }) {
                            break 'epoll;
                        }
                    }
                    1 => {
                        // master
                        self.master_readable = true;
                        if !self.handle_io_event().unwrap_or_else(|e| {
                            // panic with unexpected error
                            panic!("{}", e.desc());
                        }) {
                            break 'epoll;
                        }
                    }
                    2 => {
                        // signal
                        self.signal_fd.read_signal().unwrap();
                        self.window_resize().unwrap();
                    }
                    _ => {}
                }
            }
        }
        self.disconnect();
    }

    // Return with (stdin_origin, stdout_origin)
    fn set_termios(stdin_fd: RawFd, stdout_fd: RawFd) -> Result<(Termios, Termios)> {
        let stdin_origin = termios::tcgetattr(stdin_fd)?;
        let mut stdin_attr = stdin_origin.clone();
        termios::cfmakeraw(&mut stdin_attr);
        stdin_attr.output_flags = stdin_origin.output_flags;
        termios::tcsetattr(stdin_fd, SetArg::TCSANOW, &stdin_attr)?;

        let stdout_origin = termios::tcgetattr(stdout_fd)?;
        let mut stdout_attr = stdout_origin.clone();
        termios::cfmakeraw(&mut stdout_attr);
        stdout_attr.input_flags = stdout_origin.input_flags;
        stdout_attr.local_flags = stdout_origin.local_flags;
        termios::tcsetattr(stdout_fd, SetArg::TCSANOW, &stdout_attr)?;
        Ok((stdin_origin, stdout_origin))
    }

    fn set_nonblock(&self, nonblock: bool) -> Result<()> {
        fn set(fd: RawFd, nonblock: bool) -> Result<()> {
            let orign = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
            fcntl(
                fd,
                FcntlArg::F_SETFL(if nonblock {
                    orign | OFlag::O_NONBLOCK
                } else {
                    orign & !OFlag::O_NONBLOCK
                }),
            )?;
            Ok(())
        }

        set(self.stdin_fd, nonblock)?;
        set(self.master_fd, nonblock)?;
        Ok(())
    }

    fn window_resize(&self) -> Result<()> {
        unsafe {
            let mut window: winsize = mem::zeroed();
            if libc::ioctl(self.stdout_fd, TIOCGWINSZ, &mut window) >= 0 {
                libc::ioctl(self.master_fd, TIOCSWINSZ, &window);
                return Ok(());
            }
        }
        Err(Errno::last())
    }

    fn handle_io_event(&mut self) -> Result<bool> {
        while self.stdin_readable || self.master_readable {
            if self.stdin_readable {
                match read(self.stdin_fd, &mut self.in_buffer) {
                    Ok(s) => {
                        write(self.master_fd, &self.in_buffer[..s])?;
                        if s == 0 {
                            self.stdin_readable = false;
                        }
                    }
                    Err(Errno::EWOULDBLOCK) => self.stdin_readable = false,
                    Err(e) => return Err(e),
                }
            }
            if self.master_readable {
                match read(self.master_fd, &mut self.out_buffer) {
                    Ok(s) => {
                        write(self.stdout_fd, &self.out_buffer[..s])?;
                        if s == 0 {
                            self.master_readable = false;
                        }
                    }
                    Err(Errno::EWOULDBLOCK) => self.master_readable = false,
                    Err(Errno::EIO) => return Ok(false),
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(true)
    }

    fn disconnect(&self) {
        termios::tcsetattr(self.stdout_fd, SetArg::TCSANOW, &self.stdout_origin).unwrap();
        termios::tcsetattr(self.stdin_fd, SetArg::TCSANOW, &self.stdin_origin).unwrap();
        self.set_nonblock(false).unwrap();
    }
}
