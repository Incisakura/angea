use std::env;

use angea::shell;
use angea::systemd::Systemd;

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1) {
        Some(c) => match c.as_str() {
            "boot" => boot(),
            "shell" => shell(),
            "shutdown" => shutdown(),
            _ => help(),
        },
        None => help(),
    };
}

fn boot() {
    Systemd::new();
}

fn shell() {
    shell::enter();
}

fn shutdown() {
    Systemd::from_proc().unwrap().shutdown();
}

fn help() {
    print!("Angea v0.0.2
Usage: angea <command>
Command:
    boot        Boot systemd as daemon
    shell       Init bash shell in container
    shutdown    Kill runing systemd
    help        This message
");
}
