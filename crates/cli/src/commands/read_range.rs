use std::path::Path;
use std::process::ExitCode;

use contextpatch_core::fs::read_range::read_range;

pub fn run(args: &[String]) -> ExitCode {
    let request = match parse_args(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    match read_range(
        Path::new(&request.path),
        request.start_line,
        request.end_line,
    ) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("read-range refused: {error}");
            ExitCode::from(1)
        }
    }
}

struct ReadRangeArgs {
    path: String,
    start_line: usize,
    end_line: usize,
}

fn parse_args(args: &[String]) -> Result<ReadRangeArgs, String> {
    let path = args
        .first()
        .ok_or_else(|| "missing path argument".to_string())?
        .clone();
    let mut start_line = None;
    let mut end_line = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].as_str() {
            "--start" => {
                index += 1;
                start_line = Some(parse_line_number("--start", args.get(index))?);
            }
            "--end" => {
                index += 1;
                end_line = Some(parse_line_number("--end", args.get(index))?);
            }
            unknown => return Err(format!("unknown read-range argument: {unknown}")),
        }
        index += 1;
    }

    Ok(ReadRangeArgs {
        path,
        start_line: start_line.ok_or_else(|| "missing --start value".to_string())?,
        end_line: end_line.ok_or_else(|| "missing --end value".to_string())?,
    })
}

fn parse_line_number(flag: &str, value: Option<&String>) -> Result<usize, String> {
    value
        .ok_or_else(|| format!("{flag} requires a value"))?
        .parse::<usize>()
        .map_err(|error| format!("{flag} requires a positive integer: {error}"))
}

fn print_usage() {
    eprintln!("usage: contextpatch read-range <path> --start <line> --end <line>");
}
