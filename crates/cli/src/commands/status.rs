use std::path::Path;
use std::process::ExitCode;

use contextpatch_core::git::status::{status_summary, status_summary_for_path};

pub fn run(args: &[String]) -> ExitCode {
    let request = match parse_args(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    let result = match request.path {
        Some(path) => status_summary_for_path(Path::new("."), Some(Path::new(&path))),
        None => status_summary(Path::new(".")),
    };

    match result {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("status-guard refused: {error}");
            ExitCode::from(1)
        }
    }
}

struct StatusArgs {
    path: Option<String>,
}

fn parse_args(args: &[String]) -> Result<StatusArgs, String> {
    let mut path = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--path" => {
                index += 1;
                path = Some(
                    args.get(index)
                        .ok_or_else(|| "--path requires a value".to_string())?
                        .clone(),
                );
            }
            value if path.is_none() => path = Some(value.to_string()),
            unknown => return Err(format!("unknown status argument: {unknown}")),
        }
        index += 1;
    }

    Ok(StatusArgs { path })
}

fn print_usage() {
    eprintln!("usage: contextpatch status-guard [path]");
}
