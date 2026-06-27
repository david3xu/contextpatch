use std::path::Path;
use std::process::ExitCode;

use contextpatch_core::patch::diff::preview_exact_replacement;

pub fn run(args: &[String]) -> ExitCode {
    let request = match parse_args(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    match preview_exact_replacement(Path::new(&request.path), &request.old, &request.new) {
        Ok(diff) => {
            print!("{diff}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("diff-preview refused: {error}");
            ExitCode::from(1)
        }
    }
}

struct DiffPreviewArgs {
    path: String,
    old: String,
    new: String,
}

fn parse_args(args: &[String]) -> Result<DiffPreviewArgs, String> {
    let path = args
        .first()
        .ok_or_else(|| "missing path argument".to_string())?
        .clone();
    let mut old = None;
    let mut new = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].as_str() {
            "--old" => {
                index += 1;
                old = Some(
                    args.get(index)
                        .ok_or_else(|| "--old requires a value".to_string())?
                        .clone(),
                );
            }
            "--new" => {
                index += 1;
                new = Some(
                    args.get(index)
                        .ok_or_else(|| "--new requires a value".to_string())?
                        .clone(),
                );
            }
            unknown => return Err(format!("unknown diff-preview argument: {unknown}")),
        }
        index += 1;
    }

    Ok(DiffPreviewArgs {
        path,
        old: old.ok_or_else(|| "missing --old value".to_string())?,
        new: new.ok_or_else(|| "missing --new value".to_string())?,
    })
}

fn print_usage() {
    eprintln!("usage: contextpatch diff-preview <path> --old <text> --new <text>");
}
