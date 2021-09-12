use std::ffi::CString;
use std::fs;
use std::io;
use std::str::FromStr;

use nix::mount::{mount, MsFlags};
use nix::sched::{clone, CloneFlags};
use nix::sys::signal;
use nix::unistd::{execve, Pid};

pub struct Systemd {
    pub pid: i32,
}

impl Systemd {
    /// Try to fetch systemd or create a new one
    pub fn new() -> Systemd {
        let process = Systemd::from_proc().expect("Unable to read /proc");
        if process.pid != 0 {
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
                    _ => continue,
                };
                path.push("comm");
                let comm = match fs::read_to_string(path) {
                    Ok(str) => str,
                    _ => continue,
                };
                if comm.trim() == "systemd" {
                    return Ok(Systemd { pid });
                }
            }
        }
        return Ok(Systemd { pid: 0 });
    }

    pub fn shutdown(self) {
        if self.pid != 0 {
            signal::kill(Pid::from_raw(self.pid), signal::Signal::SIGKILL).unwrap_or_else(|_| {
                panic!(
                    "Unable to kill systemd PID: {}. Are you in container?",
                    self.pid
                );
            });
        }
    }

    fn create() -> Systemd {
        // Spawn child process
        let mut stack = [0; 4096];
        let pid = clone(
            Box::new(|| -> isize {
                let args: Vec<CString> = vec![CString::new("/lib/systemd/systemd").unwrap()];
                let environ: Vec<CString> = Vec::new();
                mount(
                    Some("proc"),
                    "/proc",
                    Some("proc"),
                    MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
                    None::<&str>,
                )
                .unwrap();
                execve(args[0].as_c_str(), &args, &environ).unwrap();
                0
            }),
            &mut stack,
            CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNS,
            None,
        )
        .unwrap()
        .as_raw();

        return Systemd { pid };
    }
}
