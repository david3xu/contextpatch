use std::path::PathBuf;

use crate::process::runner::display_command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl PlannedCommand {
    pub(crate) fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    pub fn display(&self) -> String {
        display_command(&self.program, &self.args)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPlan {
    pub program: String,
    pub args: Vec<String>,
    pub commands: Vec<PlannedCommand>,
    pub mutates_repo: bool,
    pub external_mutator: bool,
    pub expected_changed_path_classes: Vec<String>,
}

impl CommandPlan {
    pub(crate) fn new(
        program: impl Into<String>,
        args: Vec<String>,
        expected_changed_path_classes: Vec<String>,
    ) -> Self {
        let command = PlannedCommand::new(program, args);
        Self::sequence(vec![command], expected_changed_path_classes)
    }

    pub(crate) fn sequence(
        commands: Vec<PlannedCommand>,
        expected_changed_path_classes: Vec<String>,
    ) -> Self {
        assert!(
            !commands.is_empty(),
            "setup command plan must contain at least one command"
        );
        let first = commands[0].clone();
        Self {
            program: first.program,
            args: first.args,
            commands,
            mutates_repo: true,
            external_mutator: true,
            expected_changed_path_classes,
        }
    }

    pub fn display(&self) -> String {
        self.commands
            .iter()
            .map(PlannedCommand::display)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupProfileResult {
    pub profile: String,
    pub action: String,
    pub dry_run: bool,
    pub cwd: PathBuf,
    pub plan: CommandPlan,
    pub execution: Option<SetupExecution>,
    pub required_confirm_for_mutation: &'static str,
}

impl SetupProfileResult {
    pub fn summary(&self) -> String {
        let command_label = if self.plan.commands.len() == 1 {
            "command"
        } else {
            "commands"
        };
        let mut summary = format!(
            "profile: {}\naction: {}\ndry_run: {}\nexternal_mutator: {}\nmutates_repo: {}\n{}: {}\ncwd: {}\nexpected_changed_path_classes:\n{}\nrequired_confirm_for_mutation: {:?}",
            self.profile,
            self.action,
            self.dry_run,
            self.plan.external_mutator,
            self.plan.mutates_repo,
            command_label,
            self.plan.display(),
            self.cwd.display(),
            self.plan.expected_changed_path_classes.join("\n"),
            self.required_confirm_for_mutation
        );
        if let Some(execution) = &self.execution {
            summary.push_str(&format!(
                "\nexecuted: true\nexit_code: {}\ntimed_out: {}\nduration_ms: {}\nchanged_paths:\n{}\nstatus_before:\n{}\nstatus_after:\n{}\nstdout:\n{}\nstderr:\n{}",
                execution.exit_code,
                execution.timed_out,
                execution.duration_ms,
                format_lines(&execution.changed_paths),
                empty_label(&execution.status_before),
                empty_label(&execution.status_after),
                empty_label(&execution.stdout),
                empty_label(&execution.stderr)
            ));
        }
        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupExecution {
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
    pub status_before: String,
    pub status_after: String,
    pub changed_paths: Vec<String>,
}

fn format_lines(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.join("\n")
    }
}

fn empty_label(value: &str) -> &str {
    if value.trim().is_empty() {
        "(empty)"
    } else {
        value
    }
}
