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

const STDIN: RawFd = libc::STDIN_FILENO;

const STDOUT: RawFd = libc::STDOUT_FILENO;

pub struct PTYForward {
    epoll: RawFd,

    master_fd: RawFd,

    stdin_origin: Termios,
    stdout_origin: Termios,
    signal_fd: SignalFd,
}

impl PTYForward {
    /// Try to setup ptyforward
    pub fn new(master_fd: RawFd) -> Result<Self> {
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

        let (stdin_origin, stdout_origin) = Self::set_termios()?;

        let f = Self {
            epoll,
            master_fd,
            stdin_origin,
            stdout_origin,
            signal_fd,
        };
        f.set_nonblock(true)?;
        f.window_resize()?;

        Ok(f)
    }

    /// Wait epoll event
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
        self.disconnect()?;
        Ok(())
    }

    /// Set termios config for stdin/stdout, and return origin config.
    ///
    /// Return with `(stdin_origin, stdout_origin)`.
    fn set_termios() -> Result<(Termios, Termios)> {
        fn set(fd: RawFd) -> Result<Termios> {
            let stdin_origin = termios::tcgetattr(fd)?;
            let mut stdin_attr = stdin_origin.clone();
            termios::cfmakeraw(&mut stdin_attr);
            termios::tcsetattr(fd, SetArg::TCSANOW, &stdin_attr)?;
            Ok(stdin_origin)
        }

        Ok((set(STDIN)?, set(STDOUT)?))
    }

    /// Set non-block status of stdin/master
    fn set_nonblock(&self, nonblock: bool) -> Result<()> {
        fn set(fd: RawFd, nonblock: bool) -> Result<()> {
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

        set(STDIN, nonblock)?;
        set(self.master_fd, nonblock)?;
        Ok(())
    }

    /// Set master winsize with stdout winsize.
    fn window_resize(&self) -> Result<()> {
        unsafe {
            let mut window: winsize = mem::zeroed();
            if libc::ioctl(STDOUT, TIOCGWINSZ, &mut window) >= 0
                && libc::ioctl(self.master_fd, TIOCSWINSZ, &window) >= 0
            {
                return Ok(());
            }
        }
        Err(Errno::last())
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
    fn disconnect(&self) -> Result<()> {
        termios::tcsetattr(STDOUT, SetArg::TCSANOW, &self.stdout_origin)?;
        termios::tcsetattr(STDIN, SetArg::TCSANOW, &self.stdin_origin)?;
        self.set_nonblock(false)?;
        Ok(())
    }
}
