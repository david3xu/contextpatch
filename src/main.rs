use std::env;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        None | Some("help") | Some("--help") | Some("-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("version") | Some("--version") | Some("-V") => {
            println!("contextpatch {VERSION}");
            ExitCode::SUCCESS
        }
        Some("status") => {
            eprintln!("status: not implemented yet");
            ExitCode::from(2)
        }
        Some("diff") => {
            eprintln!("diff: not implemented yet");
            ExitCode::from(2)
        }
        Some("replace-exact") => {
            eprintln!("replace-exact: not implemented yet");
            ExitCode::from(2)
        }
        Some("apply-patch") => {
            eprintln!("apply-patch: not implemented yet");
            ExitCode::from(2)
        }
        Some("serve") => {
            eprintln!("serve: not implemented yet");
            ExitCode::from(2)
        }
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("run `contextpatch help` for usage");
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    println!(
        "\
contextpatch {VERSION}

Guarded patch editing for AI context servers.

Usage:
  contextpatch <command>

Commands:
  status          Show repository edit readiness
  diff            Preview a guarded edit
  replace-exact   Replace text only when an anchor matches exactly once
  apply-patch     Apply a guarded unified patch
  serve           Run the local context server
  version         Print version
  help            Print this help
"
    );
}
