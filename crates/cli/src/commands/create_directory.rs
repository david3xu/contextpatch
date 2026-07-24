use std::path::Path;
use std::process::ExitCode;

use contextpatch_core::fs::create_directory::create_directory;

pub fn run(args: &[String]) -> ExitCode {
    let request = match parse_args(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    match create_directory(Path::new(&request.path), request.parents) {
        Ok(summary) => {
            println!(
                "created directory {} ({} directories created)",
                summary.path.display(),
                summary.directories_created.len()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("create-directory refused: {error}");
            ExitCode::from(1)
        }
    }
}

struct CreateDirectoryArgs {
    path: String,
    parents: bool,
}

fn parse_args(args: &[String]) -> Result<CreateDirectoryArgs, String> {
    let path = args
        .first()
        .ok_or_else(|| "missing path argument".to_string())?
        .clone();
    let mut parents = false;

    for arg in &args[1..] {
        match arg.as_str() {
            "--parents" | "-p" => parents = true,
            unknown => return Err(format!("unknown create-directory argument: {unknown}")),
        }
    }

    Ok(CreateDirectoryArgs { path, parents })
}

fn print_usage() {
    eprintln!("usage: contextpatch create-directory <path> [--parents]");
}
