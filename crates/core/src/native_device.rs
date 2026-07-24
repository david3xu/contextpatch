use std::path::{Path, PathBuf};

use crate::error::ContextPatchError;
use crate::process::runner::{
    checked_timeout, display_command, resolve_cwd, run_no_shell_command,
    validate_common_command_shape,
};
use crate::setup::profile::{validate_non_empty_single_line, validate_relative_path_param};

pub const NATIVE_DEVICE_CONFIRMATION: &str = "run native device";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeDeviceParams {
    None,
    IosDevice {
        device: String,
    },
    IosCreate {
        name: String,
        device_type: String,
        runtime: Option<String>,
    },
    IosInstall {
        device: String,
        app_path: String,
    },
    IosLaunch {
        device: String,
        app_id: String,
    },
    IosCapRun {
        target: String,
    },
    AndroidSerial {
        serial: Option<String>,
    },
    AndroidInstall {
        serial: Option<String>,
        apk_path: String,
    },
    AndroidLaunch {
        serial: Option<String>,
        app_id: String,
    },
    AndroidLogcat {
        serial: Option<String>,
        lines: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDevicePlan {
    pub action: String,
    pub program: String,
    pub args: Vec<String>,
    pub device_operation: bool,
    pub mutates_repo_source: bool,
    pub changes_device_state: bool,
}

impl NativeDevicePlan {
    pub fn display(&self) -> String {
        display_command(&self.program, &self.args)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDeviceResult {
    pub action: String,
    pub dry_run: bool,
    pub cwd: PathBuf,
    pub plan: NativeDevicePlan,
    pub execution: Option<NativeDeviceExecution>,
    pub required_confirm_for_device_state: &'static str,
}

impl NativeDeviceResult {
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "action: {}\ndry_run: {}\ndevice_operation: {}\nchanges_device_state: {}\nmutates_repo_source: {}\ncommand: {}\ncwd: {}\nrequired_confirm_for_device_state: {:?}",
            self.action,
            self.dry_run,
            self.plan.device_operation,
            self.plan.changes_device_state,
            self.plan.mutates_repo_source,
            self.plan.display(),
            self.cwd.display(),
            self.required_confirm_for_device_state
        );
        if let Some(execution) = &self.execution {
            summary.push_str(&format!(
                "\nexecuted: true\nexit_code: {}\ntimed_out: {}\nduration_ms: {}\nstdout:\n{}\nstderr:\n{}",
                execution.exit_code,
                execution.timed_out,
                execution.duration_ms,
                empty_label(&execution.stdout),
                empty_label(&execution.stderr)
            ));
        }
        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDeviceExecution {
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
}

pub fn native_device_run(
    repo_root: &Path,
    cwd: Option<&Path>,
    action: &str,
    params: NativeDeviceParams,
    timeout_secs: Option<u64>,
    dry_run: bool,
    confirm: Option<&str>,
) -> Result<NativeDeviceResult, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let cwd = resolve_cwd(&root, cwd)?;
    let timeout = checked_timeout(timeout_secs)?;
    let plan = plan_native_device(&cwd, action, params)?;

    if !dry_run && plan.changes_device_state && confirm != Some(NATIVE_DEVICE_CONFIRMATION) {
        return Err(ContextPatchError::new(format!(
            "native_device_run refused: dry_run=false for device-state changes requires confirm: {NATIVE_DEVICE_CONFIRMATION:?}"
        )));
    }

    let execution = if dry_run {
        None
    } else {
        let output = run_no_shell_command(
            &cwd,
            &plan.program,
            &plan.args,
            timeout,
            "native_device_run",
        )?;
        if output.timed_out || output.exit_code != 0 {
            return Err(ContextPatchError::new(format!(
                "native_device_run command failed\nexit_code: {}\ntimed_out: {}\nstdout:\n{}\nstderr:\n{}",
                output.exit_code,
                output.timed_out,
                empty_label(&output.stdout),
                empty_label(&output.stderr)
            )));
        }
        Some(NativeDeviceExecution {
            exit_code: output.exit_code,
            timed_out: output.timed_out,
            duration_ms: output.duration_ms,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    };

    Ok(NativeDeviceResult {
        action: action.to_string(),
        dry_run,
        cwd,
        plan,
        execution,
        required_confirm_for_device_state: NATIVE_DEVICE_CONFIRMATION,
    })
}

fn plan_native_device(
    cwd: &Path,
    action: &str,
    params: NativeDeviceParams,
) -> Result<NativeDevicePlan, ContextPatchError> {
    let (program, args, changes_device_state) = match action {
        "ios_list_simulators" => {
            require_none(action, params)?;
            ("xcrun", strings(&["simctl", "list", "devices"]), false)
        }
        "ios_boot_simulator" => {
            let NativeDeviceParams::IosDevice { device } = params else {
                return Err(required(action, "device"));
            };
            validate_device_id("device", &device)?;
            (
                "xcrun",
                vec!["simctl".to_string(), "boot".to_string(), device],
                true,
            )
        }
        "ios_create_simulator" => {
            let NativeDeviceParams::IosCreate {
                name,
                device_type,
                runtime,
            } = params
            else {
                return Err(required(action, "name and device_type"));
            };
            validate_simctl_label("name", &name)?;
            validate_simctl_label("device_type", &device_type)?;
            let mut args = vec![
                "simctl".to_string(),
                "create".to_string(),
                name,
                device_type,
            ];
            if let Some(runtime) = runtime {
                validate_simctl_label("runtime", &runtime)?;
                args.push(runtime);
            }
            ("xcrun", args, true)
        }
        "ios_install_app" => {
            let NativeDeviceParams::IosInstall { device, app_path } = params else {
                return Err(required(action, "device and app_path"));
            };
            validate_device_id("device", &device)?;
            validate_relative_path_param("native_device_run", "app_path", &app_path)?;
            (
                "xcrun",
                vec![
                    "simctl".to_string(),
                    "install".to_string(),
                    device,
                    app_path,
                ],
                true,
            )
        }
        "ios_launch_app" => {
            let NativeDeviceParams::IosLaunch { device, app_id } = params else {
                return Err(required(action, "device and app_id"));
            };
            validate_device_id("device", &device)?;
            validate_app_id(&app_id)?;
            (
                "xcrun",
                vec!["simctl".to_string(), "launch".to_string(), device, app_id],
                true,
            )
        }
        "ios_cap_run" => {
            let NativeDeviceParams::IosCapRun { target } = params else {
                return Err(required(action, "target"));
            };
            validate_device_id("target", &target)?;
            let package_manager = detect_package_manager(cwd);
            let mut args = package_manager.cap_exec_args();
            args.extend([
                "run".to_string(),
                "ios".to_string(),
                "--target".to_string(),
                target,
                "--no-sync".to_string(),
            ]);
            (package_manager.program(), args, true)
        }
        "ios_read_logs" => {
            let NativeDeviceParams::IosDevice { device } = params else {
                return Err(required(action, "device"));
            };
            validate_device_id("device", &device)?;
            (
                "xcrun",
                vec![
                    "simctl".to_string(),
                    "spawn".to_string(),
                    device,
                    "log".to_string(),
                    "stream".to_string(),
                    "--style".to_string(),
                    "compact".to_string(),
                ],
                false,
            )
        }
        "android_list_devices" => {
            require_android_serial_or_none(action, params)?;
            ("adb", strings(&["devices"]), false)
        }
        "android_install_app" => {
            let NativeDeviceParams::AndroidInstall { serial, apk_path } = params else {
                return Err(required(action, "apk_path"));
            };
            validate_relative_path_param("native_device_run", "apk_path", &apk_path)?;
            let mut args = serial_args(serial)?;
            args.extend(["install".to_string(), apk_path]);
            return android_plan(action, args, true);
        }
        "android_launch_app" => {
            let NativeDeviceParams::AndroidLaunch { serial, app_id } = params else {
                return Err(required(action, "app_id"));
            };
            validate_app_id(&app_id)?;
            let mut args = serial_args(serial)?;
            args.extend([
                "shell".to_string(),
                "monkey".to_string(),
                "-p".to_string(),
                app_id,
                "1".to_string(),
            ]);
            return android_plan(action, args, true);
        }
        "android_read_logcat" => {
            let NativeDeviceParams::AndroidLogcat { serial, lines } = params else {
                return Err(required(action, "optional serial and lines"));
            };
            let lines = lines.unwrap_or(200).clamp(1, 2000).to_string();
            let mut args = serial_args(serial)?;
            args.extend([
                "logcat".to_string(),
                "-d".to_string(),
                "-t".to_string(),
                lines,
            ]);
            return android_plan(action, args, false);
        }
        _ => {
            return Err(ContextPatchError::new(format!(
                "native_device_run refused: unknown action `{action}`"
            )));
        }
    };
    validate_common_command_shape(program, &args)?;
    Ok(NativeDevicePlan {
        action: action.to_string(),
        program: program.to_string(),
        args,
        device_operation: true,
        mutates_repo_source: false,
        changes_device_state,
    })
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Npm,
    Pnpm,
}

impl PackageManager {
    fn program(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
        }
    }

    fn cap_exec_args(self) -> Vec<String> {
        match self {
            Self::Npm => vec!["exec".to_string(), "--".to_string(), "cap".to_string()],
            Self::Pnpm => vec!["exec".to_string(), "cap".to_string()],
        }
    }
}

fn detect_package_manager(cwd: &Path) -> PackageManager {
    if cwd.join("pnpm-lock.yaml").is_file() {
        PackageManager::Pnpm
    } else {
        PackageManager::Npm
    }
}

fn android_plan(
    action: &str,
    args: Vec<String>,
    changes_device_state: bool,
) -> Result<NativeDevicePlan, ContextPatchError> {
    validate_common_command_shape("adb", &args)?;
    Ok(NativeDevicePlan {
        action: action.to_string(),
        program: "adb".to_string(),
        args,
        device_operation: true,
        mutates_repo_source: false,
        changes_device_state,
    })
}

fn serial_args(serial: Option<String>) -> Result<Vec<String>, ContextPatchError> {
    let Some(serial) = serial else {
        return Ok(Vec::new());
    };
    validate_device_id("serial", &serial)?;
    Ok(vec!["-s".to_string(), serial])
}

fn require_none(action: &str, params: NativeDeviceParams) -> Result<(), ContextPatchError> {
    if params == NativeDeviceParams::None {
        Ok(())
    } else {
        Err(ContextPatchError::new(format!(
            "native_device_run refused: action `{action}` does not accept params"
        )))
    }
}

fn require_android_serial_or_none(
    action: &str,
    params: NativeDeviceParams,
) -> Result<(), ContextPatchError> {
    match params {
        NativeDeviceParams::None | NativeDeviceParams::AndroidSerial { .. } => Ok(()),
        _ => Err(ContextPatchError::new(format!(
            "native_device_run refused: action `{action}` accepts only optional serial params"
        ))),
    }
}

fn required(action: &str, fields: &str) -> ContextPatchError {
    ContextPatchError::new(format!(
        "native_device_run refused: action `{action}` requires {fields}"
    ))
}

fn validate_device_id(field: &str, value: &str) -> Result<(), ContextPatchError> {
    validate_non_empty_single_line("native_device_run", field, value, 160)?;
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
    {
        return Err(ContextPatchError::new(format!(
            "native_device_run refused: {field} contains unsupported characters"
        )));
    }

    Ok(())
}

fn validate_simctl_label(field: &str, value: &str) -> Result<(), ContextPatchError> {
    validate_non_empty_single_line("native_device_run", field, value, 160)?;
    if !value.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '.' | ':' | '(' | ')')
    }) {
        return Err(ContextPatchError::new(format!(
            "native_device_run refused: {field} contains unsupported characters"
        )));
    }
    Ok(())
}

fn validate_app_id(value: &str) -> Result<(), ContextPatchError> {
    validate_non_empty_single_line("native_device_run", "app_id", value, 200)?;
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(ContextPatchError::new(
            "native_device_run refused: app_id contains unsupported characters",
        ));
    }
    Ok(())
}

fn empty_label(value: &str) -> &str {
    if value.trim().is_empty() {
        "(empty)"
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{native_device_run, NativeDeviceParams};

    #[test]
    fn plans_ios_and_android_device_actions_without_raw_commands() {
        let root = git_root("plans_ios_and_android_device_actions_without_raw_commands");

        let ios = native_device_run(
            &root,
            None,
            "ios_launch_app",
            NativeDeviceParams::IosLaunch {
                device: "booted".to_string(),
                app_id: "com.example.app".to_string(),
            },
            Some(30),
            true,
            None,
        )
        .unwrap();
        assert_eq!(
            ios.plan.args,
            ["simctl", "launch", "booted", "com.example.app"]
        );
        assert!(ios.plan.changes_device_state);

        let create = native_device_run(
            &root,
            None,
            "ios_create_simulator",
            NativeDeviceParams::IosCreate {
                name: "ContextPatch iPhone".to_string(),
                device_type: "iPhone 16".to_string(),
                runtime: Some("iOS 26.4".to_string()),
            },
            Some(30),
            true,
            None,
        )
        .unwrap();
        assert_eq!(
            create.plan.args,
            [
                "simctl",
                "create",
                "ContextPatch iPhone",
                "iPhone 16",
                "iOS 26.4"
            ]
        );
        assert!(create.plan.changes_device_state);

        fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
        let cap_run = native_device_run(
            &root,
            None,
            "ios_cap_run",
            NativeDeviceParams::IosCapRun {
                target: "00000000-0000-0000-0000-000000000000".to_string(),
            },
            Some(30),
            true,
            None,
        )
        .unwrap();
        assert_eq!(cap_run.plan.program, "pnpm");
        assert_eq!(
            cap_run.plan.args,
            [
                "exec",
                "cap",
                "run",
                "ios",
                "--target",
                "00000000-0000-0000-0000-000000000000",
                "--no-sync"
            ]
        );
        assert!(cap_run.plan.changes_device_state);

        let android = native_device_run(
            &root,
            None,
            "android_read_logcat",
            NativeDeviceParams::AndroidLogcat {
                serial: Some("emulator-5554".to_string()),
                lines: Some(50),
            },
            Some(30),
            true,
            None,
        )
        .unwrap();
        assert_eq!(
            android.plan.args,
            ["-s", "emulator-5554", "logcat", "-d", "-t", "50"]
        );
        assert!(!android.plan.changes_device_state);
    }

    #[test]
    fn requires_confirmation_for_device_state_mutation() {
        let root = git_root("requires_confirmation_for_device_state_mutation");

        let error = native_device_run(
            &root,
            None,
            "ios_boot_simulator",
            NativeDeviceParams::IosDevice {
                device: "booted".to_string(),
            },
            Some(30),
            false,
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("requires confirm"));
    }

    #[test]
    fn refuses_invalid_device_params() {
        let root = git_root("refuses_invalid_device_params");

        let error = native_device_run(
            &root,
            None,
            "android_install_app",
            NativeDeviceParams::AndroidInstall {
                serial: None,
                apk_path: "../app.apk".to_string(),
            },
            Some(30),
            true,
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("repository-relative path"));
    }

    fn git_root(name: &str) -> PathBuf {
        let root = temp_root(name);
        run_git(&root, &["init", "--quiet"]);
        root
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
}
