use std::env;

use angea::shell::{get_pty, PTYForward};
use angea::systemd::Systemd;

const VERSION: &str = "0.0.6";

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1) {
        Some(c) => match c.as_str() {
            "boot" => Systemd::fetch_or_create(),
            "shutdown" => {
                if let Some(s) = Systemd::fetch() {
                    s.shutdown();
                }
            }
            "shell" => {
                let user = args.get(2).map_or("root", |s| s.as_str());
                Systemd::fetch_or_create();
                let master = get_pty(user).expect("Failed to create pty peer");
                let mut f = PTYForward::new(master).expect("Failed to start pty forward");
                f.wait().unwrap_or_else(|e| eprintln!("{}", e.desc()));
                f.disconnect().unwrap(); // should no error here
            }
            _ => help(),
        },
        None => help(),
    };
}

fn help() {
    print!(
        "Angea v{}
Usage: angea <command> [more]
Command:
    boot            Start systemd
    shell [user]    Open a shell in systemd. [Default: root]
    shutdown        Kill running systemd
    help            This message
",
        VERSION
    );
}
