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

use std::path::{Component, Path};

use serde_json::Value;

/// The sentinel Harbor prints before the path to its result file.
const RESULTS_MARKER: &str = "Results written to ";

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
            let count = trials.as_array().map(Vec::len).unwrap_or(1).max(1);
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
pub(crate) fn rewards_for_run(repo_root: &Path, output: &str) -> Option<Vec<f64>> {
    let relative = result_path(output)?;
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let root = repo_root.canonicalize().ok()?;
    let result_file = root.join(relative).canonicalize().ok()?;
    if !result_file.starts_with(&root) || !result_file.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(result_file).ok()?;
    let document = serde_json::from_str::<Value>(&text).ok()?;
    let rewards = rewards_from_result_json(&document);
    if rewards.is_empty() {
        None
    } else {
        Some(rewards)
    }
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
            .unwrap_or(1)
            .max(1);
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
    fn refuses_a_result_path_that_escapes_the_repository() {
        let escaping = "stdout:\nResults written to ../../etc/passwd\n";
        assert!(rewards_for_run(Path::new("/tmp"), escaping).is_none());
        let absolute = "stdout:\nResults written to /etc/passwd\n";
        assert!(rewards_for_run(Path::new("/tmp"), absolute).is_none());
    }
}
