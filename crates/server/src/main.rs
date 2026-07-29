#![recursion_limit = "256"]

mod protocol;
mod server;
mod tools;

use std::process::ExitCode;

fn main() -> ExitCode {
    let repo_root = match server::parse_repo_root(std::env::args().skip(1).collect()) {
        Ok(repo_root) => repo_root,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    server::run_stdio_server(&repo_root)
}
