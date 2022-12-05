mod shell;

mod systemd;

use nix::Result;
use shell::{get_pty, PTYForward};
use std::env;

pub fn cmd() {
    let mut args = env::args();
    args.next();
    let ret = match args.next() {
        Some(s) if s == "boot" => boot(),
        Some(s) if s == "shutdown" => shutdown(),
        Some(s) if s == "shell" => shell(args.next()),
        _ => help(),
    };
    if let Err(e) = ret {
        eprintln!("{}", e);
    }
}

fn shell(user: Option<String>) -> Result<()> {
    boot()?;

    let user = user.unwrap_or_else(|| String::from("root"));
    let master = get_pty(user)?;
    let mut f = PTYForward::new(master)?;
    f.wait()?;
    Ok(())
}

fn boot() -> Result<()> {
    if systemd::get_running()?.is_none() {
        systemd::start()?;
    }
    Ok(())
}

fn shutdown() -> Result<()> {
    systemd::shutdown()
}

fn help() -> Result<()> {
    print!(concat!(
        "Angea version ",
        env!("CARGO_PKG_VERSION"),
        "
Usage: angea <command> [more]
Command:
    boot            Start systemd
    shell [user]    Open a shell in systemd. [Default: root]
    shutdown        Kill running systemd
    help            This message
"
    ));
    Ok(())
}
