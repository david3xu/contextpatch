use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde_json::{Map, Value};

pub fn run(args: &[String]) -> ExitCode {
    match configure(args) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("configure-claude-desktop refused: {message}");
            ExitCode::from(1)
        }
    }
}

fn configure(args: &[String]) -> Result<String, String> {
    let options = parse_options(args)?;
    let config_path = match options.config {
        Some(path) => path,
        None => default_config_path()?,
    };
    let _lock = acquire_config_lock(&config_path)?;
    let original = fs::read(&config_path).map_err(|error| {
        format!(
            "failed to read `{}`: {error}. Add a ContextPatch MCP server in Claude Desktop first",
            config_path.display()
        )
    })?;
    let mut config: Value = serde_json::from_slice(&original)
        .map_err(|error| format!("`{}` is not valid JSON: {error}", config_path.display()))?;
    let servers = config
        .as_object_mut()
        .and_then(|root| root.get_mut("mcpServers"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            format!(
                "`{}` does not contain an `mcpServers` object",
                config_path.display()
            )
        })?;

    let mut matched = Vec::new();
    let mut cleaned = 0;
    let mut surface_updates = 0;
    for (name, server) in servers {
        let Some(server) = server.as_object_mut() else {
            continue;
        };
        if !is_contextpatch_server(server) {
            continue;
        }
        matched.push(name.clone());
        if set_tool_surface(name, server, options.tool_surface)? {
            surface_updates += 1;
        }
        if has_legacy_contextpatch_tool_policy(server) {
            server.remove("toolPolicy");
            cleaned += 1;
        }
    }
    matched.sort();

    if matched.is_empty() {
        return Err(format!(
            "no MCP server using `contextpatch-server` was found in `{}`",
            config_path.display()
        ));
    }

    let names = matched.join(", ");
    let authorization_notice = authorization_notice(options.tool_surface);
    if options.dry_run {
        if cleaned == 0 && surface_updates == 0 {
            return Ok(format!(
                "would make no changes; {} ordinary Claude Desktop ContextPatch server(s) already \
                 use the `{}` tool surface: {names}\n{authorization_notice}",
                matched.len(),
                options.tool_surface.as_str()
            ));
        }
        return Ok(format!(
            "would update {} ordinary Claude Desktop ContextPatch server(s): {names}\nwould set the \
             `{}` tool surface on {surface_updates} targeted server(s); would remove the exact \
             legacy ContextPatch wildcard `toolPolicy` from {cleaned} targeted server(s); unrelated \
             command arguments remain unchanged\n{authorization_notice}",
            matched.len(),
            options.tool_surface.as_str()
        ));
    }

    if cleaned == 0 && surface_updates == 0 {
        return Ok(format!(
            "{} ordinary Claude Desktop ContextPatch server(s) are configured in the normal \
             `mcpServers` map with the `{}` tool surface: {names}\nno exact legacy ContextPatch \
             wildcard `toolPolicy` was present; no approval policy was written\n\
             {authorization_notice}",
            matched.len(),
            options.tool_surface.as_str()
        ));
    }

    let backup = unique_backup_path(&config_path)?;
    let permissions = fs::metadata(&config_path)
        .map_err(|error| format!("failed to inspect `{}`: {error}", config_path.display()))?
        .permissions();
    write_backup(&backup, &original, &permissions)?;
    let mut updated = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("failed to serialize Claude Desktop config: {error}"))?;
    updated.push(b'\n');
    if let Err(error) = write_atomic_preserving_permissions(&config_path, &updated, &original) {
        let _ = fs::remove_file(&backup);
        return Err(error);
    }

    Ok(format!(
        "updated {} ordinary Claude Desktop ContextPatch server(s) in the normal `mcpServers` map: \
         {names}\nset the `{}` tool surface on {surface_updates} targeted server(s); removed the \
         exact legacy ContextPatch wildcard `toolPolicy` from {cleaned} targeted server(s); \
         unrelated command arguments remain unchanged\nbackup: {}\nrestart Claude Desktop to reload \
         the updated configuration\n{authorization_notice}",
        matched.len(),
        options.tool_surface.as_str(),
        backup.display()
    ))
}

fn authorization_notice(surface: ToolSurface) -> &'static str {
    match surface {
        ToolSurface::Project => {
            "The local ContextPatch MCP connection requires no authentication. The project surface \
             reduces Claude's persistent approval scope to one stable `project_execute` tool per \
             configured project, but Claude Desktop still controls runtime authorization. Local MCP \
             metadata cannot silently preapprove that tool; approve it persistently when Claude \
             offers Always allow/Allow for all tasks."
        }
        ToolSurface::Full => {
            "The local ContextPatch MCP connection requires no authentication. The full surface \
             exposes every direct tool, and Claude Desktop controls runtime authorization for each \
             one. Local MCP metadata cannot silently preapprove those tools."
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ToolSurface {
    Full,
    #[default]
    Project,
}

impl ToolSurface {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "full" => Ok(Self::Full),
            "project" => Ok(Self::Project),
            _ => Err(format!(
                "--tool-surface must be `project` or `full`, got `{value}`"
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Project => "project",
        }
    }
}

#[derive(Default)]
struct Options {
    config: Option<PathBuf>,
    dry_run: bool,
    tool_surface: ToolSurface,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--config" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--config requires a path".to_string())?;
                if options.config.is_some() {
                    return Err("--config may be provided only once".to_string());
                }
                options.config = Some(PathBuf::from(value));
                index += 2;
            }
            "--dry-run" => {
                options.dry_run = true;
                index += 1;
            }
            "--tool-surface" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--tool-surface requires a value".to_string())?;
                if args[..index]
                    .iter()
                    .any(|argument| argument == "--tool-surface")
                {
                    return Err("--tool-surface may be provided only once".to_string());
                }
                options.tool_surface = ToolSurface::parse(value)?;
                index += 2;
            }
            unknown => return Err(format!("unknown argument `{unknown}`")),
        }
    }

    Ok(options)
}

fn set_tool_surface(
    server_name: &str,
    server: &mut Map<String, Value>,
    surface: ToolSurface,
) -> Result<bool, String> {
    let args = server
            .entry("args".to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                format!(
                    "ContextPatch server `{server_name}` has a non-array `args` value; no changes were \
                     written"
                )
            })?;

    if args.iter().any(|argument| !argument.is_string()) {
        return Err(format!(
                "ContextPatch server `{server_name}` has a non-string command argument; no changes were \
                 written"
            ));
    }
    if args.iter().any(|argument| {
        argument
            .as_str()
            .is_some_and(|value| value.starts_with("--tool-surface="))
    }) {
        return Err(format!(
            "ContextPatch server `{server_name}` uses malformed `--tool-surface=<value>` syntax; \
                 use separate array entries"
        ));
    }

    let positions: Vec<usize> = args
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            (argument.as_str() == Some("--tool-surface")).then_some(index)
        })
        .collect();
    if positions.len() > 1 {
        return Err(format!(
                "ContextPatch server `{server_name}` contains duplicate `--tool-surface` arguments; no \
                 changes were written"
            ));
    }

    let Some(index) = positions.first().copied() else {
        args.push(Value::String("--tool-surface".to_string()));
        args.push(Value::String(surface.as_str().to_string()));
        return Ok(true);
    };
    let current = args.get(index + 1).and_then(Value::as_str).ok_or_else(|| {
        format!(
            "ContextPatch server `{server_name}` has `--tool-surface` without a string value; no \
                 changes were written"
        )
    })?;
    ToolSurface::parse(current).map_err(|_| {
        format!(
            "ContextPatch server `{server_name}` has invalid tool surface `{current}`; no changes \
                 were written"
        )
    })?;
    if current == surface.as_str() {
        return Ok(false);
    }

    args[index + 1] = Value::String(surface.as_str().to_string());
    Ok(true)
}

fn default_config_path() -> Result<PathBuf, String> {
    if cfg!(target_os = "macos") {
        return home_dir().map(|home| {
            home.join("Library/Application Support/Claude/claude_desktop_config.json")
        });
    }
    if cfg!(target_os = "windows") {
        let app_data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "APPDATA is not set; pass --config explicitly".to_string())?;
        return Ok(app_data.join("Claude/claude_desktop_config.json"));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| home_dir().map(|home| home.join(".config")))?;
    Ok(base.join("Claude/claude_desktop_config.json"))
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set; pass --config explicitly".to_string())
}

fn is_contextpatch_server(server: &Map<String, Value>) -> bool {
    let Some(command) = server.get("command").and_then(Value::as_str) else {
        return false;
    };
    let executable = command.rsplit(['/', '\\']).next().unwrap_or(command);
    executable.eq_ignore_ascii_case("contextpatch-server")
        || executable.eq_ignore_ascii_case("contextpatch-server.exe")
}

fn has_legacy_contextpatch_tool_policy(server: &Map<String, Value>) -> bool {
    let Some(policy) = server.get("toolPolicy").and_then(Value::as_object) else {
        return false;
    };
    policy.len() == 1 && policy.get("*").and_then(Value::as_str) == Some("allow")
}

fn config_lock_path(config_path: &Path) -> Result<PathBuf, String> {
    let file_name = config_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Claude Desktop config path has no valid file name".to_string())?;
    Ok(config_path.with_file_name(format!("{file_name}.contextpatch.lock")))
}

fn acquire_config_lock(config_path: &Path) -> Result<File, String> {
    let lock_path = config_lock_path(config_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| {
            format!(
                "failed to open configuration lock `{}`: {error}",
                lock_path.display()
            )
        })?;
    match FileExt::try_lock_exclusive(&lock) {
        Ok(()) => Ok(lock),
        Err(error) if error.kind() == ErrorKind::WouldBlock => Err(format!(
            "another ContextPatch process is updating `{}`; this operation was not started. Retry \
             after that process finishes",
            config_path.display()
        )),
        Err(error) => Err(format!(
            "failed to lock `{}` for update: {error}",
            config_path.display()
        )),
    }
}

fn unique_backup_path(config_path: &Path) -> Result<PathBuf, String> {
    let file_name = config_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Claude Desktop config path has no valid file name".to_string())?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_millis();
    for attempt in 0..100 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let candidate = config_path.with_file_name(format!(
            "{file_name}.contextpatch-backup-{timestamp}{suffix}"
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "failed to choose a unique backup path for `{}`",
        config_path.display()
    ))
}

fn write_backup(path: &Path, contents: &[u8], permissions: &Permissions) -> Result<(), String> {
    let mut handle = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to create backup `{}`: {error}", path.display()))?;
    let result = (|| {
        handle
            .set_permissions(permissions.clone())
            .map_err(|error| {
                format!(
                    "failed to preserve permissions on backup `{}`: {error}",
                    path.display()
                )
            })?;
        handle
            .write_all(contents)
            .map_err(|error| format!("failed to write backup `{}`: {error}", path.display()))?;
        handle
            .sync_all()
            .map_err(|error| format!("failed to flush backup `{}`: {error}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn write_atomic_preserving_permissions(
    path: &Path,
    contents: &[u8],
    expected: &[u8],
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Claude Desktop config path has no parent directory".to_string())?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Claude Desktop config path has no valid file name".to_string())?;
    let permissions = fs::metadata(path)
        .map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?
        .permissions();

    for attempt in 0..100 {
        let temporary = parent.join(format!(
            ".{file_name}.contextpatch.{}.{}.tmp",
            std::process::id(),
            attempt
        ));
        let mut handle = match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(handle) => handle,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create temporary config `{}`: {error}",
                    temporary.display()
                ))
            }
        };
        let result = (|| {
            handle
                .set_permissions(permissions.clone())
                .map_err(|error| {
                    format!(
                        "failed to preserve permissions on `{}`: {error}",
                        temporary.display()
                    )
                })?;
            handle.write_all(contents).map_err(|error| {
                format!(
                    "failed to write temporary config `{}`: {error}",
                    temporary.display()
                )
            })?;
            handle.sync_all().map_err(|error| {
                format!(
                    "failed to flush temporary config `{}`: {error}",
                    temporary.display()
                )
            })?;
            let current = fs::read(path)
                .map_err(|error| format!("failed to re-read `{}`: {error}", path.display()))?;
            if current != expected {
                return Err(format!(
                    "`{}` changed while ContextPatch was preparing the update; no ContextPatch \
                     changes were applied. Review the current file and retry",
                    path.display()
                ));
            }
            fs::rename(&temporary, path).map_err(|error| {
                format!(
                    "failed to replace `{}` with `{}`: {error}",
                    path.display(),
                    temporary.display()
                )
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    Err("failed to create a unique temporary config after 100 attempts".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_config_write_refuses_a_stale_read() {
        let root = test_root("stale");
        let config = root.join("claude_desktop_config.json");
        let original = b"{\"mcpServers\":{}}\n";
        let concurrent = b"{\"theme\":\"light\"}\n";
        fs::write(&config, concurrent).unwrap();

        let error =
            write_atomic_preserving_permissions(&config, b"{\"theme\":\"dark\"}\n", original)
                .unwrap_err();

        assert!(error.contains("changed while ContextPatch was preparing"));
        assert_eq!(fs::read(&config).unwrap(), concurrent);
    }

    #[test]
    fn server_detection_accepts_windows_executables() {
        let server = json!({
            "command": r"C:\Program Files\ContextPatch\contextpatch-server.EXE"
        });
        assert!(is_contextpatch_server(server.as_object().unwrap()));
    }

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-config-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
