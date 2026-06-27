use std::path::Path;
use std::process::ExitCode;

use contextpatch_core::replace::exact::replace_exact;

pub fn run(args: &[String]) -> ExitCode {
    let request = match parse_args(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    match replace_exact(Path::new(&request.path), &request.old, &request.new) {
        Ok(summary) => {
            println!(
                "replaced bytes {}..{} in {} ({} bytes written)",
                summary.start_byte,
                summary.end_byte,
                summary.path.display(),
                summary.bytes_written
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("replace-exact refused: {error}");
            ExitCode::from(1)
        }
    }
}

struct ReplaceExactArgs {
    path: String,
    old: String,
    new: String,
}

fn parse_args(args: &[String]) -> Result<ReplaceExactArgs, String> {
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
            unknown => return Err(format!("unknown replace-exact argument: {unknown}")),
        }
        index += 1;
    }

    Ok(ReplaceExactArgs {
        path,
        old: old.ok_or_else(|| "missing --old value".to_string())?,
        new: new.ok_or_else(|| "missing --new value".to_string())?,
    })
}

fn print_usage() {
    eprintln!("usage: contextpatch replace-exact <path> --old <text> --new <text>");
}
