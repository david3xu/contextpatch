use std::path::{Component, Path};

use crate::error::ContextPatchError;
use crate::fs::guarded_file::{open_regular_file_in_root, GuardedRegularFile};
use crate::fs::hash::{sha256_bytes, validate_sha256};
use crate::fs::mutation_lock::try_file_mutation_lock_for_open_file;

pub const CONFIRMATION: &str = "set file executable";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutableChange {
    pub path: String,
    pub sha256: String,
    pub before_mode: String,
    pub after_mode: String,
    pub before_executable: bool,
    pub after_executable: bool,
    pub changed: bool,
    pub dry_run: bool,
}

pub fn set_file_executable_in_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
    executable: bool,
    expected_sha256: Option<&str>,
    expected_mode: Option<&str>,
    dry_run: bool,
    confirm: Option<&str>,
) -> Result<ExecutableChange, ContextPatchError> {
    #[cfg(not(unix))]
    {
        let _ = (
            repo_root,
            path,
            executable,
            expected_sha256,
            expected_mode,
            dry_run,
            confirm,
        );
        return Err(ContextPatchError::new(
            "executable-bit changes are supported only on Unix",
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let authority = repo_root.into();
        let root = crate::fs::rooted::canonical_label(authority)?;
        let relative = normalize_relative_path(path)?;
        if dry_run {
            let file = open_regular_file_in_root(authority, path)?;
            return inspect_change(&file, relative, executable, true);
        }

        if confirm != Some(CONFIRMATION) {
            return Err(ContextPatchError::new(format!(
                "dry_run=false requires confirm: {CONFIRMATION:?}"
            )));
        }
        let expected_sha256 = expected_sha256.ok_or_else(|| {
            ContextPatchError::new("dry_run=false requires expected_sha256 from a current dry run")
        })?;
        validate_sha256(expected_sha256)?;
        let expected_mode = expected_mode.ok_or_else(|| {
            ContextPatchError::new("dry_run=false requires expected_mode from a current dry run")
        })?;
        let expected_mode = normalize_mode(expected_mode)?;

        let file = open_regular_file_in_root(authority, path)?;
        let target = file.target_path();
        let _mutation_lock = try_file_mutation_lock_for_open_file(&root, &target, file.file())?;
        let change = inspect_change(&file, relative.clone(), executable, false)?;
        if expected_sha256 != change.sha256 {
            return Err(ContextPatchError::new(format!(
                "SHA-256 mismatch for `{relative}`: expected {expected_sha256}, current {}",
                change.sha256
            )));
        }
        if expected_mode != change.before_mode {
            return Err(ContextPatchError::new(format!(
                "mode mismatch for `{relative}`: expected {expected_mode}, current {}",
                change.before_mode
            )));
        }

        if change.changed {
            file.revalidate_current_path()?;
            let after_mode_value = u32::from_str_radix(&change.after_mode, 8)
                .expect("validated mode formatting is octal");
            let metadata = file.file().metadata().map_err(|error| {
                ContextPatchError::new(format!("failed to inspect `{relative}`: {error}"))
            })?;
            ensure_single_hard_link(&metadata, &relative)?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(after_mode_value);
            file.file().set_permissions(permissions).map_err(|error| {
                ContextPatchError::new(format!(
                    "failed to set executable mode on `{relative}`: {error}"
                ))
            })?;
            let observed = file
                .file()
                .metadata()
                .map_err(|error| {
                    ContextPatchError::new(format!(
                        "failed to verify executable mode on `{relative}`: {error}"
                    ))
                })?
                .permissions()
                .mode()
                & 0o7777;
            if observed != after_mode_value {
                return Err(ContextPatchError::new(format!(
                    "mode verification failed for `{relative}`: expected {:04o}, observed {observed:04o}",
                    after_mode_value
                )));
            }
            let observed_sha256 = sha256_bytes(&file.read_all()?);
            if observed_sha256 != change.sha256 {
                return Err(ContextPatchError::new(format!(
                    "content hash changed while setting executable mode on `{relative}`"
                )));
            }
        }
        Ok(change)
    }
}

#[cfg(unix)]
fn inspect_change(
    file: &GuardedRegularFile,
    relative: String,
    executable: bool,
    dry_run: bool,
) -> Result<ExecutableChange, ContextPatchError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = file.file().metadata().map_err(|error| {
        ContextPatchError::new(format!("failed to inspect `{relative}`: {error}"))
    })?;
    ensure_single_hard_link(&metadata, &relative)?;
    let before_mode_value = metadata.permissions().mode() & 0o7777;
    let after_mode_value = if executable {
        before_mode_value | 0o111
    } else {
        before_mode_value & !0o111
    };
    let sha256 = sha256_bytes(&file.read_all()?);
    Ok(ExecutableChange {
        path: relative,
        sha256,
        before_mode: format!("{before_mode_value:04o}"),
        after_mode: format!("{after_mode_value:04o}"),
        before_executable: before_mode_value & 0o111 != 0,
        after_executable: executable,
        changed: before_mode_value != after_mode_value,
        dry_run,
    })
}

fn normalize_mode(mode: &str) -> Result<String, ContextPatchError> {
    if !(3..=4).contains(&mode.len()) || !mode.chars().all(|ch| matches!(ch, '0'..='7')) {
        return Err(ContextPatchError::new(
            "expected_mode must be a three- or four-digit octal mode",
        ));
    }
    let value =
        u32::from_str_radix(mode, 8).expect("validated three- or four-digit octal mode must parse");
    Ok(format!("{value:04o}"))
}

#[cfg(unix)]
fn ensure_single_hard_link(
    metadata: &std::fs::Metadata,
    relative: &str,
) -> Result<(), ContextPatchError> {
    use std::os::unix::fs::MetadataExt;

    let link_count = metadata.nlink();
    if link_count != 1 {
        return Err(ContextPatchError::new(format!(
            "`{relative}` has {link_count} hard links; executable-bit changes require exactly one"
        )));
    }
    Ok(())
}

fn normalize_relative_path(path: &Path) -> Result<String, ContextPatchError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }
    let relative = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::hash::sha256_file;
    use crate::fs::mutation_lock::try_file_mutation_lock;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn plans_then_changes_only_execute_bits() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("set-executable");
        let target = root.join("test.sh");
        fs::write(&target, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&target).unwrap().permissions();
        permissions.set_mode(0o640);
        fs::set_permissions(&target, permissions).unwrap();

        let plan =
            set_file_executable_in_root(&root, Path::new("test.sh"), true, None, None, true, None)
                .unwrap();
        assert_eq!(plan.before_mode, "0640");
        assert_eq!(plan.after_mode, "0751");
        assert!(!plan.before_executable);
        assert!(plan.after_executable);

        let applied = set_file_executable_in_root(
            &root,
            Path::new("test.sh"),
            true,
            Some(&plan.sha256),
            Some(&plan.before_mode),
            false,
            Some(CONFIRMATION),
        )
        .unwrap();
        assert!(applied.changed);
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
            0o751
        );
        assert_eq!(sha256_file(&target).unwrap(), plan.sha256);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_while_the_target_mutation_lock_is_held() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("set-executable-lock");
        let target = root.join("test.sh");
        fs::write(&target, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&target).unwrap().permissions();
        permissions.set_mode(0o640);
        fs::set_permissions(&target, permissions).unwrap();
        let plan =
            set_file_executable_in_root(&root, Path::new("test.sh"), true, None, None, true, None)
                .unwrap();
        let _lock = try_file_mutation_lock(&root, &target).unwrap();

        let error = set_file_executable_in_root(
            &root,
            Path::new("test.sh"),
            true,
            Some(&plan.sha256),
            Some(&plan.before_mode),
            false,
            Some(CONFIRMATION),
        )
        .unwrap_err();

        assert!(error.to_string().contains("still active"));
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn accepts_three_digit_expected_modes() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("set-executable-three-digit-mode");
        let target = root.join("test.sh");
        fs::write(&target, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&target).unwrap().permissions();
        permissions.set_mode(0o640);
        fs::set_permissions(&target, permissions).unwrap();
        let plan =
            set_file_executable_in_root(&root, Path::new("test.sh"), true, None, None, true, None)
                .unwrap();

        let applied = set_file_executable_in_root(
            &root,
            Path::new("test.sh"),
            true,
            Some(&plan.sha256),
            Some("640"),
            false,
            Some(CONFIRMATION),
        )
        .unwrap();

        assert!(applied.changed);
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
            0o751
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_files_with_multiple_hard_links() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("set-executable-hard-link");
        let outside = temp_root("set-executable-hard-link-outside");
        let target = root.join("test.sh");
        let alias = outside.join("alias.sh");
        fs::write(&target, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&target).unwrap().permissions();
        permissions.set_mode(0o640);
        fs::set_permissions(&target, permissions).unwrap();
        fs::hard_link(&target, &alias).unwrap();

        let error =
            set_file_executable_in_root(&root, Path::new("test.sh"), true, None, None, true, None)
                .unwrap_err();

        assert!(error.to_string().contains("hard links"));
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        assert_eq!(
            fs::metadata(&alias).unwrap().permissions().mode() & 0o7777,
            0o640
        );
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
}
