use std::fs;
use std::io;
use std::str::FromStr;

use nix::mount::{self, MsFlags};
use nix::sys::signal;
use nix::unistd::Pid;
use unshare::{Command, Namespace};

pub struct Systemd {
    pub pid: i32,
    pub is_running: bool,
}

impl Systemd {
    /// Try to fetch systemd or create a new one
    pub fn new() -> Systemd {
        let process = Systemd::from_proc().expect("Unable to read /proc");
        if process.is_running {
            return process;
        }
        Systemd::create()
    }

    /// Try to fetch systemd from /proc
    pub fn from_proc() -> io::Result<Systemd> {
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            let mut path = entry.path();
            if path.is_dir() {
                let pid = match path.file_name().and_then(|f| f.to_str()).map(i32::from_str) {
                    Some(Ok(p)) => p,
                    _ => continue
                };
                path.push("comm");
                let comm = match fs::read_to_string(path) {
                    Ok(str) => str,
                    _ => continue,
                };
                if comm.trim() == "systemd" {
                    return Ok(Systemd {
                        pid,
                        is_running: true,
                    });
                }
            }
        }
        return Ok(Systemd {
            pid: 0,
            is_running: false,
        });
    }

    pub fn shutdown(&mut self) {
        if self.is_running {
            signal::kill(Pid::from_raw(self.pid), signal::Signal::SIGKILL).unwrap_or_else(|_| {
                println!(
                    "Unable to kill to PID: {}. Are you in container?",
                    self.pid
                );
            });
            self.is_running = false;
        }
        self.pid = 0;
    }

    fn create() -> Systemd {
        // Spawn child process
        let child = unsafe {
            Command::new("/lib/systemd/systemd")
                .unshare(&[Namespace::Pid, Namespace::Mount])
                .allow_daemonize()
                .pre_exec(|| {
                    // Mount proc
                    mount::mount(
                        Some("proc"),
                        "/proc",
                        Some("proc"),
                        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
                        None::<&str>,
                    )
                    .unwrap();
                    Ok(())
                })
                .spawn()
                .unwrap()
        };

        return Systemd {
            pid: child.pid(),
            is_running: true,
        };
    }
}
