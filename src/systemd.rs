use std::ffi::CString;

use nix::dir::Dir;
use nix::fcntl::{open, OFlag};
use nix::mount::{mount, MsFlags};
use nix::sched::{clone, CloneFlags};
use nix::sys::signal::{kill, Signal};
use nix::sys::stat::Mode;
use nix::unistd::{close, execve, read, Pid};
use nix::Result;

/// Start a new systemd process
pub fn start() -> Result<Pid> {
    let mut stack = [0; 4096];
    let pid = clone(
        Box::new(|| -> isize {
            let args = [CString::new("/lib/systemd/systemd").unwrap()];
            let environ: [CString; 0] = [];
            mount(
                Some("proc"),
                "/proc",
                Some("proc"),
                MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
                None::<&str>,
            )
            .unwrap();
            execve(args[0].as_c_str(), &args, &environ).unwrap();
            unreachable!();
        }),
        &mut stack,
        CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNS,
        None,
    )?;
    Ok(pid)
}

/// Try to fetch `Systemd` from procfs
pub fn from_proc() -> Result<Option<Pid>> {
    let proc = Dir::open("/proc", OFlag::O_DIRECTORY, Mode::empty())?;
    for entry in proc {
        match entry {
            Ok(e) => {
                let file_name = e.file_name().to_string_lossy();
                if file_name == "." || file_name == ".." {
                    continue;
                }
                let pid = match file_name.parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let mut path = String::from("/proc/");
                path.push_str(&file_name);
                path.push_str("/comm");

                let fd = open(path.as_str(), OFlag::O_RDONLY, Mode::empty())?;
                let mut buf = [0; 8];
                let n = read(fd, &mut buf)?;
                if &buf[..n] == "systemd\n".as_bytes() {
                    return Ok(Some(Pid::from_raw(pid)));
                }
                close(fd)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Kill running systemd process
pub fn shutdown(pid: Pid) {
    if let Err(e) = kill(pid, Signal::SIGKILL) {
        eprintln!("{}", e);
    }
}
