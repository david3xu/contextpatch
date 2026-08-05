//! Read Harbor rewards from the result file Harbor writes, not from its rendered table.
//!
//! The previous extractor scanned stdout for a line containing "reward" and a number. Harbor renders
//! rewards as a box-drawing table, so the header `┃ Reward ┃ Count ┃` carries the word and the row
//! `│ 1.0    │     1 │` carries the number, and no single line has both. The scan therefore found
//! nothing and the profile reported `missing_rewards` with `passed: false` for runs that had in fact
//! scored 1.000 and 0.000 exactly as intended.
//!
//! That is worse than reporting nothing, because it cannot report success under any circumstance: a
//! caller trusting the summary would conclude the gates failed and start debugging a healthy task.
//!
//! Harbor already writes a machine readable `result.json` and prints its path. Reading that is stable
//! against any future change to the table styling, and it yields the individual per trial rewards
//! rather than one scraped figure, which is what the determinism checks actually want. The text scan
//! stays as a fallback for the case where the file is absent or unreadable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use contextpatch_core::fs::guarded_file::{
    inspect_path_in_root, open_regular_file_in_root, GuardedPathKind,
};
use contextpatch_core::git::RepositoryRoot;
use contextpatch_core::process::guarded_command::redact_and_truncate_output;
use serde_json::Value;

/// The sentinel Harbor prints before the path to its result file.
const RESULTS_MARKER: &str = "Results written to ";
const MAX_STRUCTURED_TRIALS: usize = 100;
const MAX_STRUCTURED_REWARDS: usize = 1_000;
const MAX_STRUCTURED_EVIDENCE_BYTES: usize = 700_000;

/// Where Harbor recorded this run, as a repository relative path.
///
/// Returned as a borrowed slice so the caller decides whether the path is acceptable; this module
/// deliberately does no path validation of its own, leaving that to the guarded read.
pub(crate) fn result_path(output: &str) -> Option<&str> {
    output.lines().find_map(|line| {
        let candidate = line.trim().strip_prefix(RESULTS_MARKER)?;
        let candidate = candidate.trim();
        if candidate.is_empty() {
            None
        } else {
            Some(candidate)
        }
    })
}

/// Every per trial reward recorded in one Harbor result file.
///
/// `reward_stats.reward` maps a reward value, as a string key, to the list of trials that scored it,
/// so a value is repeated once per trial. That preserves the multiplicity the determinism check needs;
/// collapsing to `metrics[].mean` would hide a run that scored 1.0 and 0.0 once each.
pub(crate) fn rewards_from_result_json(document: &Value) -> Vec<f64> {
    let mut rewards = Vec::new();
    let Some(evals) = document
        .get("stats")
        .and_then(|stats| stats.get("evals"))
        .and_then(Value::as_object)
    else {
        return rewards;
    };
    for evaluation in evals.values() {
        let Some(by_reward) = evaluation
            .get("reward_stats")
            .and_then(|stats| stats.get("reward"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (value, trials) in by_reward {
            let Ok(reward) = value.parse::<f64>() else {
                continue;
            };
            let Some(count) = trials.as_array().map(Vec::len) else {
                continue;
            };
            rewards.extend(std::iter::repeat_n(reward, count));
        }
    }
    rewards
}

/// Rewards for one Harbor invocation, preferring its result file over its rendered output.
///
/// `None` means neither source produced a reward, which is the only case the caller should report as
/// missing. An empty vector is treated as no answer for the same reason: a result file that parses but
/// records nothing is not evidence of a reward.
pub(crate) fn rewards_for_run<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    output: &str,
) -> Option<Vec<f64>> {
    let root = root.into();
    let (_, result_relative, _) = resolve_result_file(output).ok()?;
    let (text, truncated, _) = read_bounded_text(root, &result_relative, 2_000_000).ok()?;
    if truncated {
        return None;
    }
    let document = serde_json::from_str::<Value>(&text).ok()?;
    let rewards = rewards_from_result_json(&document);
    if rewards.is_empty() {
        None
    } else {
        Some(rewards)
    }
}

/// Structured, bounded evidence for one completed Harbor invocation.
///
/// The job result is the authority for trial names. Result and trial paths must use Harbor's exact
/// `jobs/<job>/...` layout, remain beneath that job directory, and contain no symlink components.
pub(crate) fn structured_evidence<'a>(root: impl Into<RepositoryRoot<'a>>, output: &str) -> Value {
    let root = root.into();
    structured_evidence_result(root, output).unwrap_or_else(|error| {
        serde_json::json!({
            "available": false,
            "reported_result_path": result_path(output),
            "evidence_error": error
        })
    })
}

fn structured_evidence_result(root: RepositoryRoot<'_>, output: &str) -> Result<Value, String> {
    let (job_relative, result_relative, result_display) = resolve_result_file(output)?;
    let (text, result_truncated, _) = read_bounded_text(root, &result_relative, 2_000_000)?;
    if result_truncated {
        return Err("Harbor job result exceeds the 2000000-byte evidence limit".to_string());
    }
    let document = serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("Harbor job result is not valid JSON: {error}"))?;
    let mut rewards = rewards_from_result_json(&document);
    let reward_count = rewards.len();
    rewards.truncate(MAX_STRUCTURED_REWARDS);
    let job_display = display_relative(&job_relative);
    let mut trials = Vec::new();
    let mut summaries = trial_summaries(&document).into_iter().collect::<Vec<_>>();
    summaries.sort_by(|(left_name, left_summary), (right_name, right_summary)| {
        left_summary
            .exception_types
            .is_empty()
            .cmp(&right_summary.exception_types.is_empty())
            .then_with(|| left_name.cmp(right_name))
    });
    let trial_count = summaries.len();

    for (trial_name, summary) in summaries.into_iter().take(MAX_STRUCTURED_TRIALS) {
        let mut trial = serde_json::Map::new();
        trial.insert("trial_name".to_string(), Value::String(trial_name.clone()));
        trial.insert(
            "reward".to_string(),
            summary
                .reward
                .map_or(Value::Null, |reward| serde_json::json!(reward)),
        );
        trial.insert(
            "reported_exception_types".to_string(),
            Value::Array(
                summary
                    .exception_types
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );

        let trial_relative = Path::new(&trial_name);
        if !is_single_normal_component(trial_relative) {
            trial.insert("trial_path".to_string(), Value::Null);
            trial.insert(
                "evidence_error".to_string(),
                Value::String(
                    "trial name must be one normalized directory name beneath the Harbor job"
                        .to_string(),
                ),
            );
            trials.push(Value::Object(trial));
            continue;
        }

        let trial_relative = job_relative.join(trial_relative);
        match require_directory_in_root(root, &trial_relative) {
            Ok(()) => {}
            Err(error) => {
                trial.insert("trial_path".to_string(), Value::Null);
                trial.insert("evidence_error".to_string(), Value::String(error));
                trials.push(Value::Object(trial));
                continue;
            }
        }
        trial.insert(
            "trial_path".to_string(),
            Value::String(display_relative(&trial_relative)),
        );

        match read_bounded_text(root, &trial_relative.join("result.json"), 1_000_000) {
            Ok((trial_text, false, trial_result)) => {
                trial.insert(
                    "result_path".to_string(),
                    Value::String(display_relative(&trial_result)),
                );
                match serde_json::from_str::<Value>(&trial_text) {
                    Ok(trial_document) => {
                        trial.insert(
                            "agent".to_string(),
                            trial_document
                                .get("agent_info")
                                .and_then(|value| value.get("name"))
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                        trial.insert(
                            "started_at".to_string(),
                            trial_document
                                .get("started_at")
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                        trial.insert(
                            "finished_at".to_string(),
                            trial_document
                                .get("finished_at")
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                        trial.insert(
                            "exception".to_string(),
                            trial_document
                                .get("exception_info")
                                .map(sanitize_json_evidence)
                                .unwrap_or(Value::Null),
                        );
                    }
                    Err(error) => {
                        trial.insert(
                            "result_error".to_string(),
                            Value::String(format!("trial result is not valid JSON: {error}")),
                        );
                    }
                }
            }
            Ok((_, true, trial_result)) => {
                trial.insert(
                    "result_path".to_string(),
                    Value::String(display_relative(&trial_result)),
                );
                trial.insert(
                    "result_error".to_string(),
                    Value::String(
                        "trial result exceeds the 1000000-byte evidence limit".to_string(),
                    ),
                );
            }
            Err(error) => {
                trial.insert("result_path".to_string(), Value::Null);
                trial.insert("result_error".to_string(), Value::String(error));
            }
        }

        trial.insert(
            "verifier".to_string(),
            serde_json::json!({
                "reward": text_artifact(
                    root,
                    &trial_relative,
                    Path::new("verifier/reward.txt"),
                    4096,
                    4096,
                    true
                ),
                "stdout": text_artifact(
                    root,
                    &trial_relative,
                    Path::new("verifier/test-stdout.txt"),
                    200_000,
                    40_000,
                    false
                )
            }),
        );
        trials.push(Value::Object(trial));
    }

    let returned_reward_count = rewards.len();
    let returned_trial_count = trials.len();
    let mut evidence = serde_json::json!({
        "available": true,
        "result_path": result_display,
        "job_path": job_display,
        "rewards": rewards,
        "reward_summary": {
            "total_count": reward_count,
            "returned_count": returned_reward_count,
            "omitted_count": reward_count.saturating_sub(returned_reward_count),
            "truncated": reward_count > returned_reward_count,
            "max_rewards": MAX_STRUCTURED_REWARDS
        },
        "trials": trials,
        "trial_summary": {
            "total_count": trial_count,
            "returned_count": returned_trial_count,
            "omitted_count": trial_count.saturating_sub(returned_trial_count),
            "truncated": trial_count > returned_trial_count,
            "max_trials": MAX_STRUCTURED_TRIALS,
            "max_serialized_bytes": MAX_STRUCTURED_EVIDENCE_BYTES
        }
    });
    enforce_structured_evidence_limit(&mut evidence, trial_count)?;
    Ok(evidence)
}

fn enforce_structured_evidence_limit(
    evidence: &mut Value,
    total_trial_count: usize,
) -> Result<(), String> {
    loop {
        let bytes = serde_json::to_vec_pretty(evidence)
            .map_err(|error| format!("failed to serialize Harbor evidence: {error}"))?;
        if bytes.len() <= MAX_STRUCTURED_EVIDENCE_BYTES {
            return Ok(());
        }
        let trials = evidence
            .get_mut("trials")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "Harbor evidence trials are not an array".to_string())?;
        if trials.pop().is_none() {
            return Err(format!(
                "Harbor evidence metadata exceeds the {MAX_STRUCTURED_EVIDENCE_BYTES}-byte limit"
            ));
        }
        let returned_count = trials.len();
        evidence["trial_summary"] = serde_json::json!({
            "total_count": total_trial_count,
            "returned_count": returned_count,
            "omitted_count": total_trial_count.saturating_sub(returned_count),
            "truncated": true,
            "max_trials": MAX_STRUCTURED_TRIALS,
            "max_serialized_bytes": MAX_STRUCTURED_EVIDENCE_BYTES
        });
    }
}

/// Validate the reported result path against Harbor's exact layout.
///
/// Pure validation: it decides which repository-relative names are acceptable and touches no filesystem, so
/// there is no root to resolve here. Reaching the files is the caller's job, through the repository's own
/// authority.
fn resolve_result_file(output: &str) -> Result<(PathBuf, PathBuf, String), String> {
    let reported = result_path(output)
        .ok_or_else(|| "Harbor output did not report a result path".to_string())?;
    if reported.contains('\\') {
        return Err("Harbor result path must use normalized `/` separators".to_string());
    }
    let relative = Path::new(reported);
    let components = relative.components().collect::<Vec<_>>();
    let valid_layout = matches!(
        components.as_slice(),
        [Component::Normal(jobs), Component::Normal(_), Component::Normal(result)]
            if jobs == &std::ffi::OsStr::new("jobs")
                && result == &std::ffi::OsStr::new("result.json")
    );
    if !valid_layout {
        return Err(
            "Harbor result path must match `jobs/<job>/result.json` without traversal".to_string(),
        );
    }

    let job_relative = relative
        .parent()
        .ok_or_else(|| "Harbor result has no job directory".to_string())?
        .to_path_buf();
    let result_relative = relative.to_path_buf();
    let display = display_relative(&result_relative);
    Ok((job_relative, result_relative, display))
}

#[derive(Default)]
struct TrialSummary {
    reward: Option<f64>,
    exception_types: BTreeSet<String>,
}

fn trial_summaries(document: &Value) -> BTreeMap<String, TrialSummary> {
    let mut trials = BTreeMap::new();
    let Some(evals) = document
        .get("stats")
        .and_then(|stats| stats.get("evals"))
        .and_then(Value::as_object)
    else {
        return trials;
    };
    for evaluation in evals.values() {
        if let Some(by_reward) = evaluation
            .get("reward_stats")
            .and_then(|stats| stats.get("reward"))
            .and_then(Value::as_object)
        {
            for (reward, names) in by_reward {
                let Ok(reward) = reward.parse::<f64>() else {
                    continue;
                };
                let Some(names) = names.as_array() else {
                    continue;
                };
                for name in names.iter().filter_map(Value::as_str) {
                    trials.entry(name.to_string()).or_default().reward = Some(reward);
                }
            }
        }
        if let Some(by_exception) = evaluation.get("exception_stats").and_then(Value::as_object) {
            for (exception_type, names) in by_exception {
                let Some(names) = names.as_array() else {
                    continue;
                };
                for name in names.iter().filter_map(Value::as_str) {
                    trials
                        .entry(name.to_string())
                        .or_default()
                        .exception_types
                        .insert(exception_type.clone());
                }
            }
        }
    }
    trials
}

fn require_directory_in_root(root: RepositoryRoot<'_>, relative: &Path) -> Result<(), String> {
    let inspection = inspect_path_in_root(root, relative).map_err(|error| {
        format!(
            "failed to inspect evidence path `{}`: {error}",
            relative.display()
        )
    })?;
    match inspection {
        Some(inspection) if inspection.kind == GuardedPathKind::Directory => Ok(()),
        Some(_) => Err(format!(
            "evidence path `{}` is not an existing directory",
            relative.display()
        )),
        None => Err(format!(
            "failed to inspect evidence path `{}`: path does not exist",
            relative.display()
        )),
    }
}

fn read_bounded_text(
    base: RepositoryRoot<'_>,
    relative: &Path,
    max_bytes: u64,
) -> Result<(String, bool, PathBuf), String> {
    let file = open_regular_file_in_root(base, relative).map_err(|error| {
        format!(
            "failed to inspect evidence path `{}`: {error}",
            relative.display()
        )
    })?;
    let (bytes, truncated) = file.read_bounded(max_bytes).map_err(|error| {
        format!(
            "failed to read evidence path `{}`: {error}",
            relative.display()
        )
    })?;
    Ok((
        String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
        relative.to_path_buf(),
    ))
}

fn text_artifact(
    root: RepositoryRoot<'_>,
    trial_dir: &Path,
    relative: &Path,
    max_file_bytes: u64,
    max_output_chars: usize,
    trim: bool,
) -> Value {
    let artifact_relative = trial_dir.join(relative);
    let expected_path = display_relative(&artifact_relative);
    match read_bounded_text(root, &artifact_relative, max_file_bytes) {
        Ok((value, file_truncated, opened_relative)) => {
            let (text, output_truncated) = redact_and_truncate_output(&value, max_output_chars);
            serde_json::json!({
                "available": true,
                "path": display_relative(&opened_relative),
                "text": if trim { text.trim().to_string() } else { text },
                "truncated": file_truncated || output_truncated
            })
        }
        Err(error) => serde_json::json!({
            "available": false,
            "path": expected_path,
            "evidence_error": error
        }),
    }
}

fn sanitize_json_evidence(value: &Value) -> Value {
    let serialized = match serde_json::to_string(value) {
        Ok(serialized) => serialized,
        Err(error) => {
            return serde_json::json!({
                "available": false,
                "evidence_error": format!("failed to serialize evidence: {error}")
            })
        }
    };
    let (text, truncated) = redact_and_truncate_output(&serialized, 12_000);
    if !truncated {
        return serde_json::from_str(&text).unwrap_or(Value::String(text));
    }
    serde_json::json!({
        "text": text,
        "truncated": true
    })
}

fn is_single_normal_component(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn display_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Rewards recovered from Harbor's human-readable output when `result.json` is unavailable.
///
/// Current Harbor versions render a reward/count table and a progress line containing `Mean:`.
/// Older wrappers sometimes emit `reward: <value>` or `score: <value>` on one line. The table is
/// preferred because its count column preserves repeated trials.
pub(crate) fn rewards_from_output(output: &str) -> Option<Vec<f64>> {
    table_rewards(output)
        .or_else(|| labeled_reward(output).map(|reward| vec![reward]))
        .or_else(|| mean_reward(output).map(|reward| vec![reward]))
}

fn table_rewards(output: &str) -> Option<Vec<f64>> {
    let lines = output.lines().collect::<Vec<_>>();
    let header = lines
        .iter()
        .position(|line| line.to_ascii_lowercase().contains("reward"))?;
    let mut rewards = Vec::new();
    for line in lines.iter().skip(header + 1).take(12) {
        let cells = line
            .split(['│', '┃', '|'])
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .collect::<Vec<_>>();
        let Some(reward) = cells.first().and_then(|cell| cell.parse::<f64>().ok()) else {
            continue;
        };
        let count = cells
            .get(1)
            .and_then(|cell| cell.parse::<usize>().ok())
            .unwrap_or(1);
        rewards.extend(std::iter::repeat_n(reward, count));
    }
    (!rewards.is_empty()).then_some(rewards)
}

fn labeled_reward(output: &str) -> Option<f64> {
    output.lines().filter_map(parse_labeled_reward).next_back()
}

fn parse_labeled_reward(line: &str) -> Option<f64> {
    let lower = line.to_ascii_lowercase();
    ["reward", "score"]
        .into_iter()
        .filter_map(|label| lower.find(label))
        .min()
        .and_then(|index| first_number(&line[index..]))
}

fn mean_reward(output: &str) -> Option<f64> {
    output
        .lines()
        .filter_map(|line| {
            let lower = line.to_ascii_lowercase();
            let index = lower.find("mean:")?;
            first_number(&line[index + "mean:".len()..])
        })
        .next_back()
}

fn first_number(text: &str) -> Option<f64> {
    text.split(|ch: char| !(ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | 'e' | 'E')))
        .filter(|token| token.chars().any(|ch| ch.is_ascii_digit()))
        .find_map(|token| token.parse::<f64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const HARBOR_OUTPUT: &str = concat!(
        "command: harbor run -p task --agent oracle\n",
        "exit_code: 0\n",
        "stdout:\n",
        "  1/1 Mean: 1.000 \u{2501}\u{2501}\u{2501} 0:00:19\n",
        "\u{250f}\u{2501}\u{2501}\u{2513}\n",
        "\u{2503} Reward \u{2503} Count \u{2503}\n",
        "\u{2502} 1.0    \u{2502}     1 \u{2502}\n",
        "Job Info\n",
        "Results written to jobs/2026-07-31__16-23-24/result.json\n",
    );

    #[test]
    fn finds_the_result_path_harbor_printed() {
        assert_eq!(
            result_path(HARBOR_OUTPUT),
            Some("jobs/2026-07-31__16-23-24/result.json")
        );
    }

    #[test]
    fn absent_marker_yields_no_path() {
        assert_eq!(result_path("stdout:\nnothing here\n"), None);
    }

    #[test]
    fn parses_harbors_box_table_and_preserves_the_count() {
        let output = concat!(
            "┏━━━━━━━━┳━━━━━━━┓\n",
            "┃ Reward ┃ Count ┃\n",
            "┡━━━━━━━━╇━━━━━━━┩\n",
            "│ 1.0    │     2 │\n",
            "│ 0.0    │     1 │\n",
            "└────────┴───────┘\n",
        );
        assert_eq!(rewards_from_output(output), Some(vec![1.0, 1.0, 0.0]));
    }

    #[test]
    fn parses_mean_and_legacy_same_line_rewards() {
        assert_eq!(
            rewards_from_output("  1/1 Mean: 0.625 ━━━ 0:00:19\n"),
            Some(vec![0.625])
        );
        assert_eq!(
            rewards_from_output("completed with reward: 0.0\n"),
            Some(vec![0.0])
        );
    }

    #[test]
    fn reads_one_reward_per_trial_preserving_multiplicity() {
        // Two trials at 1.0 and one at 0.0 must come back as three rewards, not two distinct values,
        // or the determinism check cannot see the disagreement.
        let document = json!({
            "stats": {"evals": {"oracle__adhoc": {"reward_stats": {"reward": {
                "1.0": ["task__a", "task__b"],
                "0.0": ["task__c"]
            }}}}}
        });
        let mut rewards = rewards_from_result_json(&document);
        rewards.sort_by(|left, right| left.partial_cmp(right).unwrap());
        assert_eq!(rewards, vec![0.0, 1.0, 1.0]);
    }

    #[test]
    fn reads_the_shape_harbor_actually_writes() {
        let document = json!({
            "id": "a338ca31",
            "n_total_trials": 1,
            "stats": {"evals": {"nop__adhoc": {
                "n_trials": 1,
                "metrics": [{"mean": 0.0}],
                "reward_stats": {"reward": {"0.0": ["task__LvJBLpd"]}}
            }}}
        });
        assert_eq!(rewards_from_result_json(&document), vec![0.0]);
    }

    #[test]
    fn a_document_without_reward_stats_yields_nothing() {
        let document = json!({"stats": {"evals": {"oracle__adhoc": {"n_trials": 1}}}});
        assert!(rewards_from_result_json(&document).is_empty());
    }

    #[test]
    fn empty_reward_arrays_do_not_manufacture_a_reward() {
        let document = json!({
            "stats": {"evals": {"oracle__adhoc": {
                "reward_stats": {"reward": {"1.0": []}}
            }}}
        });
        assert!(rewards_from_result_json(&document).is_empty());

        let root = test_root("empty_rewards");
        fs::create_dir_all(root.join("jobs/empty")).unwrap();
        fs::write(
            root.join("jobs/empty/result.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        assert_eq!(
            rewards_for_run(
                &root,
                "stdout:\nResults written to jobs/empty/result.json\n"
            ),
            None
        );
    }

    #[test]
    fn structured_evidence_is_bounded_redacted_and_reports_trial_errors() {
        let root = test_root("structured_evidence");
        let valid = root.join("jobs/job/task__valid");
        let missing = root.join("jobs/job/task__missing");
        fs::create_dir_all(valid.join("verifier")).unwrap();
        fs::create_dir_all(&missing).unwrap();
        fs::write(
            root.join("jobs/job/result.json"),
            serde_json::to_vec(&json!({
                "stats": {"evals": {"oracle": {"reward_stats": {"reward": {
                    "1.0": ["task__valid", "task__missing"],
                    "0.0": ["../escape"]
                }}}}}
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            valid.join("result.json"),
            serde_json::to_vec(&json!({
                "agent_info": {"name": "oracle"},
                "started_at": "2026-01-01T00:00:00Z",
                "finished_at": "2026-01-01T00:00:01Z",
                "exception_info": {
                    "message": "token=not-a-real-secret-value-123456",
                    "trace": "x".repeat(20_000)
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(valid.join("verifier/reward.txt"), "1.0\n").unwrap();
        fs::write(
            valid.join("verifier/test-stdout.txt"),
            format!(
                "Authorization: Bearer never-return-this\n{}",
                "x".repeat(50_000)
            ),
        )
        .unwrap();

        let evidence =
            structured_evidence(&root, "stdout:\nResults written to jobs/job/result.json\n");
        assert_eq!(evidence["available"], true);
        assert_eq!(evidence["result_path"], "jobs/job/result.json");
        assert_eq!(evidence["job_path"], "jobs/job");
        assert_eq!(evidence["rewards"].as_array().unwrap().len(), 3);

        let trials = evidence["trials"].as_array().unwrap();
        let valid = trials
            .iter()
            .find(|trial| trial["trial_name"] == "task__valid")
            .unwrap();
        assert_eq!(valid["agent"], "oracle");
        let exception = valid["exception"].to_string();
        assert!(exception.contains("redacted potential secret line"));
        assert!(!exception.contains("not-a-real-secret-value-123456"));
        assert!(exception.chars().count() < 13_000);
        assert_eq!(valid["verifier"]["reward"]["text"], "1.0");
        assert_eq!(valid["verifier"]["stdout"]["truncated"], true);
        let stdout = valid["verifier"]["stdout"]["text"].as_str().unwrap();
        assert!(
            stdout.contains("[redacted potential secret line]"),
            "{stdout}"
        );
        assert!(!stdout.contains("never-return-this"));
        assert!(stdout.chars().count() <= 40_012);

        let missing = trials
            .iter()
            .find(|trial| trial["trial_name"] == "task__missing")
            .unwrap();
        assert!(missing["result_error"]
            .as_str()
            .unwrap()
            .contains("failed to inspect"));
        assert_eq!(missing["verifier"]["stdout"]["available"], false);

        let escaping = trials
            .iter()
            .find(|trial| trial["trial_name"] == "../escape")
            .unwrap();
        assert!(escaping["evidence_error"]
            .as_str()
            .unwrap()
            .contains("one normalized directory name"));
        assert!(escaping["trial_path"].is_null());
    }

    #[test]
    fn structured_evidence_includes_exception_only_trials() {
        let root = test_root("structured_evidence_exception_only");
        let trial = root.join("jobs/job/task__failed");
        fs::create_dir_all(&trial).unwrap();
        fs::write(
            root.join("jobs/job/result.json"),
            serde_json::to_vec(&json!({
                "stats": {"evals": {"oracle": {
                    "reward_stats": {"reward": {}},
                    "exception_stats": {
                        "AgentTimeoutError": ["task__failed"],
                        "InfrastructureError": ["task__failed"]
                    }
                }}}
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            trial.join("result.json"),
            serde_json::to_vec(&json!({
                "agent_info": {"name": "oracle"},
                "exception_info": {
                    "exception_type": "AgentTimeoutError",
                    "message": "timed out"
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let evidence =
            structured_evidence(&root, "stdout:\nResults written to jobs/job/result.json\n");
        let trials = evidence["trials"].as_array().unwrap();

        assert_eq!(trials.len(), 1);
        assert_eq!(trials[0]["trial_name"], "task__failed");
        assert!(trials[0]["reward"].is_null());
        assert_eq!(
            trials[0]["reported_exception_types"],
            json!(["AgentTimeoutError", "InfrastructureError"])
        );
        assert_eq!(
            trials[0]["exception"]["exception_type"],
            "AgentTimeoutError"
        );
    }

    #[test]
    fn structured_evidence_caps_rewards_and_trials_without_losing_exceptions() {
        let root = test_root("structured_evidence_count_limits");
        fs::create_dir_all(root.join("jobs/job")).unwrap();
        let trial_names = (0..=MAX_STRUCTURED_REWARDS)
            .map(|index| format!("task__{index:04}"))
            .collect::<Vec<_>>();
        let exception_trial = trial_names.last().unwrap().clone();
        fs::write(
            root.join("jobs/job/result.json"),
            serde_json::to_vec(&json!({
                "stats": {"evals": {"oracle": {
                    "reward_stats": {"reward": {"1.0": trial_names}},
                    "exception_stats": {
                        "InfrastructureError": [exception_trial]
                    }
                }}}
            }))
            .unwrap(),
        )
        .unwrap();

        let evidence =
            structured_evidence(&root, "stdout:\nResults written to jobs/job/result.json\n");
        let rewards = evidence["rewards"].as_array().unwrap();
        let trials = evidence["trials"].as_array().unwrap();

        assert_eq!(rewards.len(), MAX_STRUCTURED_REWARDS);
        assert_eq!(
            evidence["reward_summary"]["total_count"],
            MAX_STRUCTURED_REWARDS + 1
        );
        assert_eq!(
            evidence["reward_summary"]["returned_count"],
            MAX_STRUCTURED_REWARDS
        );
        assert_eq!(evidence["reward_summary"]["omitted_count"], 1);
        assert_eq!(evidence["reward_summary"]["truncated"], true);
        assert_eq!(trials.len(), MAX_STRUCTURED_TRIALS);
        assert_eq!(
            evidence["trial_summary"]["total_count"],
            MAX_STRUCTURED_REWARDS + 1
        );
        assert_eq!(
            evidence["trial_summary"]["returned_count"],
            MAX_STRUCTURED_TRIALS
        );
        assert_eq!(
            evidence["trial_summary"]["omitted_count"],
            MAX_STRUCTURED_REWARDS + 1 - MAX_STRUCTURED_TRIALS
        );
        assert_eq!(evidence["trial_summary"]["truncated"], true);
        assert_eq!(trials[0]["trial_name"], exception_trial);
        assert_eq!(
            trials[0]["reported_exception_types"],
            json!(["InfrastructureError"])
        );
    }

    #[test]
    fn structured_evidence_byte_limit_preserves_exception_trials() {
        let root = test_root("structured_evidence_byte_limit");
        let job = root.join("jobs/job");
        fs::create_dir_all(&job).unwrap();
        let normal_trials = (0..20)
            .map(|index| format!("task__normal_{index:02}"))
            .collect::<Vec<_>>();
        let exception_trial = "task__exception";
        for trial_name in normal_trials
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(exception_trial))
        {
            let trial = job.join(trial_name);
            fs::create_dir_all(trial.join("verifier")).unwrap();
            fs::write(trial.join("result.json"), "{}").unwrap();
            fs::write(trial.join("verifier/test-stdout.txt"), "x".repeat(50_000)).unwrap();
        }
        fs::write(
            job.join("result.json"),
            serde_json::to_vec(&json!({
                "stats": {"evals": {"oracle": {
                    "reward_stats": {"reward": {"1.0": normal_trials}},
                    "exception_stats": {
                        "AgentTimeoutError": [exception_trial]
                    }
                }}}
            }))
            .unwrap(),
        )
        .unwrap();

        let evidence =
            structured_evidence(&root, "stdout:\nResults written to jobs/job/result.json\n");
        let serialized = serde_json::to_vec_pretty(&evidence).unwrap();
        let trials = evidence["trials"].as_array().unwrap();

        assert!(serialized.len() <= MAX_STRUCTURED_EVIDENCE_BYTES);
        assert_eq!(evidence["trial_summary"]["total_count"], 21);
        assert!(
            evidence["trial_summary"]["returned_count"]
                .as_u64()
                .unwrap()
                < 21
        );
        assert_eq!(evidence["trial_summary"]["truncated"], true);
        assert_eq!(trials[0]["trial_name"], exception_trial);
        assert_eq!(
            trials[0]["reported_exception_types"],
            json!(["AgentTimeoutError"])
        );
    }

    #[test]
    fn structured_evidence_reports_missing_malformed_oversized_and_unsafe_results() {
        let root = test_root("structured_evidence_errors");
        fs::create_dir_all(root.join("jobs/malformed")).unwrap();
        fs::write(root.join("jobs/malformed/result.json"), "{").unwrap();
        let malformed = structured_evidence(
            &root,
            "stdout:\nResults written to jobs/malformed/result.json\n",
        );
        assert_eq!(malformed["available"], false);
        assert!(malformed["evidence_error"]
            .as_str()
            .unwrap()
            .contains("not valid JSON"));

        fs::create_dir_all(root.join("jobs/oversized")).unwrap();
        fs::write(
            root.join("jobs/oversized/result.json"),
            vec![b'x'; 2_000_001],
        )
        .unwrap();
        let oversized = structured_evidence(
            &root,
            "stdout:\nResults written to jobs/oversized/result.json\n",
        );
        assert_eq!(oversized["available"], false);
        assert!(oversized["evidence_error"]
            .as_str()
            .unwrap()
            .contains("exceeds the 2000000-byte"));

        let missing = structured_evidence(
            &root,
            "stdout:\nResults written to jobs/missing/result.json\n",
        );
        assert_eq!(missing["available"], false);
        assert!(missing["evidence_error"]
            .as_str()
            .unwrap()
            .contains("failed to inspect"));

        for unsafe_path in [
            "../../etc/passwd",
            "/etc/passwd",
            "jobs/job/nested/result.json",
            "jobs\\job\\result.json",
        ] {
            let evidence = structured_evidence(
                &root,
                &format!("stdout:\nResults written to {unsafe_path}\n"),
            );
            assert_eq!(evidence["available"], false, "{evidence}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn structured_evidence_refuses_symlinked_job_and_verifier_paths() {
        use std::os::unix::fs::symlink;

        let root = test_root("structured_evidence_symlinks");
        let outside = test_root("structured_evidence_symlinks_outside");
        fs::create_dir_all(outside.join("task__trial/verifier")).unwrap();
        fs::write(
            outside.join("result.json"),
            serde_json::to_vec(&json!({
                "stats": {"evals": {"oracle": {"reward_stats": {"reward": {
                    "1.0": ["task__trial"]
                }}}}}
            }))
            .unwrap(),
        )
        .unwrap();
        fs::create_dir_all(root.join("jobs")).unwrap();
        symlink(&outside, root.join("jobs/job")).unwrap();
        let job_link =
            structured_evidence(&root, "stdout:\nResults written to jobs/job/result.json\n");
        assert_eq!(job_link["available"], false);
        assert!(job_link["evidence_error"]
            .as_str()
            .unwrap()
            .contains("symlink"));

        let leaf_job = root.join("jobs/leaf");
        fs::create_dir(&leaf_job).unwrap();
        symlink(outside.join("result.json"), leaf_job.join("result.json")).unwrap();
        let result_link =
            structured_evidence(&root, "stdout:\nResults written to jobs/leaf/result.json\n");
        assert_eq!(result_link["available"], false);
        assert!(result_link["evidence_error"]
            .as_str()
            .unwrap()
            .contains("symlink"));

        let job = root.join("jobs/regular");
        let trial = job.join("task__trial");
        fs::create_dir_all(&trial).unwrap();
        fs::write(
            job.join("result.json"),
            fs::read(outside.join("result.json")).unwrap(),
        )
        .unwrap();
        fs::write(trial.join("result.json"), "{}").unwrap();
        symlink(outside.join("task__trial/verifier"), trial.join("verifier")).unwrap();
        let verifier_link = structured_evidence(
            &root,
            "stdout:\nResults written to jobs/regular/result.json\n",
        );
        assert_eq!(verifier_link["available"], true);
        let trial = &verifier_link["trials"][0];
        assert_eq!(trial["verifier"]["stdout"]["available"], false);
        assert!(trial["verifier"]["stdout"]["evidence_error"]
            .as_str()
            .unwrap()
            .contains("symlink"));
    }

    #[test]
    fn refuses_a_result_path_that_escapes_the_repository() {
        let escaping = "stdout:\nResults written to ../../etc/passwd\n";
        assert!(rewards_for_run(Path::new("/tmp"), escaping).is_none());
        let absolute = "stdout:\nResults written to /etc/passwd\n";
        assert!(rewards_for_run(Path::new("/tmp"), absolute).is_none());
    }

    fn test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "contextpatch-harbor-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
