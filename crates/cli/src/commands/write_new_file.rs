use std::path::Path;
use std::process::ExitCode;

use contextpatch_core::fs::write_new_file::write_new_file;

pub fn run(args: &[String]) -> ExitCode {
    let request = match parse_args(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    match write_new_file(Path::new(&request.path), &request.content) {
        Ok(summary) => {
            println!(
                "created {} ({} bytes written)",
                summary.path.display(),
                summary.bytes_written
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("write-new-file refused: {error}");
            ExitCode::from(1)
        }
    }
}

struct WriteNewFileArgs {
    path: String,
    content: String,
}

fn parse_args(args: &[String]) -> Result<WriteNewFileArgs, String> {
    let path = args
        .first()
        .ok_or_else(|| "missing path argument".to_string())?
        .clone();
    let mut content = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].as_str() {
            "--content" => {
                index += 1;
                content = Some(
                    args.get(index)
                        .ok_or_else(|| "--content requires a value".to_string())?
                        .clone(),
                );
            }
            unknown => return Err(format!("unknown write-new-file argument: {unknown}")),
        }
        index += 1;
    }

    Ok(WriteNewFileArgs {
        path,
        content: content.ok_or_else(|| "missing --content value".to_string())?,
    })
}

fn print_usage() {
    eprintln!("usage: contextpatch write-new-file <path> --content <text>");
}
