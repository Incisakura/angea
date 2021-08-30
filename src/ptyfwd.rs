use std::io;
use std::mem;
use std::os::unix::io::{AsRawFd, RawFd};

use libc::{winsize, TIOCGWINSZ, TIOCSWINSZ};
use nix::Error;
use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::epoll::{self, EpollEvent, EpollFlags, EpollOp};
use nix::sys::signal::{sigprocmask, SigmaskHow, Signal};
use nix::sys::signalfd::{signalfd, SfdFlags, SigSet, SignalFd, SIGNALFD_NEW};
use nix::sys::termios::{self, Termios, LocalFlags, SetArg};
use nix::unistd::{read, write};

pub struct PTYForward {
    epoll: RawFd,

    stdin_fd: RawFd,
    stdout_fd: RawFd,
    master_fd: RawFd,

    stdin_readable: bool,
    master_readable: bool,

    in_buffer: [u8; 4096],
    out_buffer: [u8; 4096],
    in_buffer_size: usize,
    master_buffer_size: usize,

    stdin_origin: Termios,
    stdout_origin: Termios,

    sig_fd: RawFd,
    sig_set: SigSet,
}

impl PTYForward {
    pub fn new(master_fd: RawFd) -> Self {
        let epoll = epoll::epoll_create().unwrap();

        let stdin_fd = io::stdin().as_raw_fd();
        let stdout_fd = io::stdout().as_raw_fd();
        let (stdin_origin, stdout_origin) = Self::set_termios(stdin_fd, stdout_fd);

        let mut sig_set = SigSet::empty();
        sig_set.add(Signal::SIGWINCH);
        sigprocmask(SigmaskHow::SIG_SETMASK, Some(&sig_set), None).unwrap();
        let sig_fd = signalfd(SIGNALFD_NEW, &sig_set, SfdFlags::empty()).unwrap();
        Self {
            epoll,
            stdin_fd,
            stdout_fd,
            master_fd,
            stdin_readable: false,
            master_readable: false,
            in_buffer: [0; 4096],
            out_buffer: [0; 4096],
            in_buffer_size: 0,
            master_buffer_size: 0,
            stdin_origin,
            stdout_origin,
            sig_fd,
            sig_set,
        }
    }

    pub fn wait(&mut self) {
        let mut stdin_event = EpollEvent::new(EpollFlags::EPOLLIN, 0);
        let mut master_event = EpollEvent::new(EpollFlags::EPOLLIN, 1);
        let mut sig_event = EpollEvent::new(EpollFlags::EPOLLIN, 2);
        let mut sfd = SignalFd::with_flags(&self.sig_set, SfdFlags::empty()).unwrap();
        self.add_event(self.stdin_fd, &mut stdin_event);
        self.add_event(self.master_fd, &mut master_event);
        self.add_event(self.sig_fd, &mut sig_event);
        self.set_nonblock(true);
        self.window_resize();

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
                        if !self.handle_io_event() {
                            break 'epoll;
                        }
                    }
                    1 => {
                        // master
                        self.master_readable = true;
                        if !self.handle_io_event() {
                            break 'epoll;
                        }
                    }
                    2 => {
                        // signal
                        sfd.read_signal().unwrap();
                        self.window_resize();
                    }
                    _ => {}
                }
            }
        }
        self.disconnect();
    }

    fn add_event(&mut self, fd: RawFd, event: &mut EpollEvent) {
        epoll::epoll_ctl(self.epoll, EpollOp::EpollCtlAdd, fd, event).unwrap();
    }

    // Return with (stdin_origin, stdout_origin)
    fn set_termios(stdin_fd: RawFd, stdout_fd: RawFd) -> (Termios, Termios) {
        let stdin_origin = termios::tcgetattr(stdin_fd).unwrap();
        let mut stdin_attr = stdin_origin.clone();
        termios::cfmakeraw(&mut stdin_attr);
        stdin_attr.output_flags = stdin_origin.output_flags;
        termios::tcsetattr(stdin_fd, SetArg::TCSANOW, &stdin_attr).unwrap();

        let stdout_origin = termios::tcgetattr(stdout_fd).unwrap();
        let mut stdout_attr = stdout_origin.clone();
        termios::cfmakeraw(&mut stdout_attr);
        stdout_attr.input_flags = stdout_origin.input_flags;
        stdout_attr.local_flags = stdout_origin.local_flags;
        termios::tcsetattr(stdout_fd, SetArg::TCSANOW, &stdout_attr).unwrap();
        (stdin_origin, stdout_origin)
    }

    fn set_nonblock(&self, nonblock: bool) {
        set(self.stdin_fd, nonblock);
        set(self.master_fd, nonblock);
        fn set(fd: RawFd, nonblock: bool) {
            let orign = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL).unwrap());
            fcntl(
                fd,
                FcntlArg::F_SETFL(if nonblock {
                    orign | OFlag::O_NONBLOCK
                } else {
                    orign & !OFlag::O_NONBLOCK
                }),
            )
            .unwrap();
        }
    }

    fn window_resize(&self) {
        unsafe {
            let mut window: winsize = mem::zeroed();
            if libc::ioctl(self.stdout_fd, TIOCGWINSZ, &mut window) >= 0 {
                libc::ioctl(self.master_fd, TIOCSWINSZ, &window);
            }
        }
    }

    fn handle_io_event(&mut self) -> bool {
        while self.stdin_readable
            || self.master_readable
            || self.in_buffer_size > 0
            || self.master_buffer_size > 0
        {
            if self.stdin_readable {
                match read(self.stdin_fd, &mut self.in_buffer) {
                    Ok(s) => {
                        self.in_buffer_size = s;
                        if s == 0 {
                            self.stdin_readable = false;
                        }
                    }
                    Err(e) => {
                        // EWOULDBLOCK
                        if e == Error::Sys(Errno::EAGAIN) {
                            self.stdin_readable = false;
                            self.in_buffer_size = 0;
                        }
                    }
                }
            }
            if self.in_buffer_size > 0 {
                write(self.master_fd, &self.in_buffer[..self.in_buffer_size]).unwrap();
            }
            if self.master_readable {
                match read(self.master_fd, &mut self.out_buffer) {
                    Ok(s) => {
                        self.master_buffer_size = s;
                        if s == 0 {
                            self.master_readable = false;
                        }
                    }
                    Err(e) => {
                        // EWOULDBLOCK
                        if e == Error::Sys(Errno::EAGAIN) {
                            self.master_readable = false;
                            self.master_buffer_size = 0;
                        } else if e == Error::Sys(Errno::EIO) {
                            // master is closed, disconnect
                            return false;
                        }
                    }
                }
            }
            if self.master_buffer_size > 0 {
                write(self.stdout_fd, &self.out_buffer[..self.master_buffer_size]).unwrap();
            }
        }
        true
    }

    fn disconnect(&self) {
        let mut out_attr = self.stdout_origin.clone();
        out_attr.local_flags |= LocalFlags::ECHO;
        termios::tcsetattr(self.stdout_fd, SetArg::TCSANOW, &out_attr).unwrap();
        termios::tcsetattr(self.stdin_fd, SetArg::TCSANOW, &self.stdin_origin).unwrap();
        self.set_nonblock(false);
    }
}
