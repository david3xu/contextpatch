use std::process::ExitCode;

use crate::args::Command;

pub mod apply_patch;
pub mod diff_preview;
pub mod read_range;
pub mod replace_exact;
pub mod status;
pub mod write_new_file;

pub fn dispatch(args: &[String]) -> ExitCode {
    match parse_command(args.first().map(String::as_str)) {
        Command::Help => {
            print_help();
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("contextpatch {}", contextpatch_core::VERSION);
            ExitCode::SUCCESS
        }
        Command::Status => status::run(),
        Command::ReadRange => read_range::run(&args[1..]),
        Command::DiffPreview => diff_preview::run(),
        Command::ReplaceExact => replace_exact::run(&args[1..]),
        Command::WriteNewFile => write_new_file::run(&args[1..]),
        Command::ApplyPatch => apply_patch::run(),
        Command::Serve => {
            eprintln!("serve lives in the server package and is not implemented yet");
            ExitCode::from(2)
        }
    }
}

fn parse_command(command: Option<&str>) -> Command {
    match command {
        None | Some("help") | Some("--help") | Some("-h") => Command::Help,
        Some("version") | Some("--version") | Some("-V") => Command::Version,
        Some("status") => Command::Status,
        Some("read-range") => Command::ReadRange,
        Some("diff") | Some("diff-preview") => Command::DiffPreview,
        Some("replace-exact") => Command::ReplaceExact,
        Some("write-new-file") => Command::WriteNewFile,
        Some("apply-patch") => Command::ApplyPatch,
        Some("serve") => Command::Serve,
        Some(unknown) => {
            eprintln!("unknown command: {unknown}");
            Command::Help
        }
    }
}

fn print_help() {
    println!(
        "\
contextpatch {}

Guarded patch editing for AI context servers.

Usage:
  contextpatch <command>
  contextpatch read-range <path> --start <line> --end <line>
  contextpatch replace-exact <path> --old <text> --new <text>
  contextpatch write-new-file <path> --content <text>

Commands:
  status          Show repository edit readiness
  read-range      Read a bounded file range
  diff-preview    Preview a guarded edit
  replace-exact   Replace text only when an anchor matches exactly once
  write-new-file  Create a file only when it does not already exist
  apply-patch     Apply a guarded unified patch
  serve           Run the local context server
  version         Print version
  help            Print this help
",
        contextpatch_core::VERSION
    );
}
