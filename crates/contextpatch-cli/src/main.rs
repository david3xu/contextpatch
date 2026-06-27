use std::process::ExitCode;

mod args;
mod commands;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    commands::dispatch(args.get(1).map(String::as_str))
}
