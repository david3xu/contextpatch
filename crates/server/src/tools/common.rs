use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn validate_sha256_hex(tool_name: &str, value: &str) -> Result<String, String> {
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!(
            "{tool_name} refused: SHA-256 digest must be 64 hexadecimal characters"
        ));
    }
    if value.chars().any(|ch| ch.is_ascii_uppercase()) {
        return Err(format!(
            "{tool_name} refused: SHA-256 digest must be lowercase hex"
        ));
    }
    Ok(value.to_string())
}

pub(crate) fn required_string<'a>(
    arguments: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid string argument: {key}"))
}

pub(crate) fn optional_string<'a>(
    arguments: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    match arguments.get(key) {
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("invalid string argument: {key}")),
        None => Ok(None),
    }
}

pub(crate) fn required_string_array(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let values = arguments
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing or invalid string array argument: {key}"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| format!("invalid string array item in argument: {key}"))
        })
        .collect()
}

pub(crate) fn optional_string_array(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    match arguments.get(key) {
        Some(value) => {
            let values = value
                .as_array()
                .ok_or_else(|| format!("invalid string array argument: {key}"))?;
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(ToString::to_string)
                        .ok_or_else(|| format!("invalid string array item in argument: {key}"))
                })
                .collect()
        }
        None => Ok(Vec::new()),
    }
}

pub(crate) fn required_usize(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<usize, String> {
    let value = arguments
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing or invalid integer argument: {key}"))?;

    usize::try_from(value).map_err(|_| format!("integer argument out of range: {key}"))
}

pub(crate) fn optional_u64(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<u64>, String> {
    match arguments.get(key) {
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid integer argument: {key}")),
        None => Ok(None),
    }
}

pub(crate) fn optional_bool(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<bool>, String> {
    match arguments.get(key) {
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("invalid boolean argument: {key}")),
        None => Ok(None),
    }
}

pub(crate) fn nonempty_tool_string(tool: &str, key: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{tool} refused: {key} must not be empty"));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn validate_relative_path(tool_name: &str, raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(format!(
            "{tool_name} refused: path must not be empty or contain NUL"
        ));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!("{tool_name} refused: path must be relative"));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "{tool_name} refused: path must be a normalized relative path"
                ))
            }
        }
    }
    Ok(path.to_path_buf())
}

pub(crate) fn normalize_repo_relative_paths(
    tool_name: &str,
    paths: &[String],
) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| normalize_repo_relative_path(tool_name, path))
        .collect()
}

pub(crate) fn normalize_repo_relative_path(tool_name: &str, raw: &str) -> Result<String, String> {
    let path = validate_relative_path(tool_name, raw)?;
    let parts = path
        .components()
        .map(|component| match component {
            std::path::Component::Normal(value) => Ok(value.to_string_lossy().into_owned()),
            _ => Err(format!(
                "{tool_name} refused: path must be a normalized relative path"
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

pub(crate) fn matches_any_prefix(path: &str, prefixes: &[String]) -> bool {
    prefixes
        .iter()
        .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
}

pub(crate) fn guarded_output_succeeded(output: &str) -> bool {
    output.lines().any(|line| line.trim() == "exit_code: 0")
}

pub(crate) fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut values = Vec::new();
    let mut padding_started = false;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding_started = true;
                64
            }
            b'\r' | b'\n' | b'\t' | b' ' => continue,
            _ => return Err("content_base64 contains invalid characters".to_string()),
        };
        if padding_started && value != 64 {
            return Err("content_base64 has data after padding".to_string());
        }
        values.push(value);
    }
    if values.is_empty() {
        return Ok(Vec::new());
    }
    if values.len() % 4 != 0 {
        return Err("content_base64 length must be a multiple of 4".to_string());
    }

    let mut output = Vec::with_capacity(values.len() / 4 * 3);
    for chunk in values.chunks_exact(4) {
        let pad = chunk.iter().filter(|value| **value == 64).count();
        if pad > 2 || (pad > 0 && chunk[2] != 64 && chunk[3] == 64 && pad != 1) {
            return Err("content_base64 has invalid padding".to_string());
        }
        if chunk[0] == 64 || chunk[1] == 64 || (chunk[2] == 64 && chunk[3] != 64) {
            return Err("content_base64 has invalid padding".to_string());
        }
        let a = u32::from(chunk[0]);
        let b = u32::from(chunk[1]);
        let c = u32::from(if chunk[2] == 64 { 0 } else { chunk[2] });
        let d = u32::from(if chunk[3] == 64 { 0 } else { chunk[3] });
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        output.push(((triple >> 16) & 0xff) as u8);
        if chunk[2] != 64 {
            output.push(((triple >> 8) & 0xff) as u8);
        }
        if chunk[3] != 64 {
            output.push((triple & 0xff) as u8);
        }
    }

    Ok(output)
}
