#![recursion_limit = "256"]

mod protocol;
mod server;
mod tools;

use std::process::ExitCode;

fn main() -> ExitCode {
    let options = match server::parse_server_options(std::env::args().skip(1).collect()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    server::run_stdio_server(&options)
}
