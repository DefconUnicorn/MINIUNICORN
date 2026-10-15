//! Last-state + named preset persistence (JSON files, no extra deps).
//!
//! Layout:
//! - `<dir>/last.json` — auto-saved working state, reloaded at startup.
//! - `<dir>/presets/<name>.json` — named user presets.
//!
//! Dir: `%APPDATA%/minimoog-d-standalone` on Windows, else
//! `$HOME/.minimoog-d-standalone` (fallback: OS temp dir).

use crate::params::Params;
use std::path::PathBuf;

fn state_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let p = PathBuf::from(appdata).join("minimoog-d-standalone");
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join(".minimoog-d-standalone");
        let _ = std::fs::create_dir_all(&p);
        return p;
    }
    let p = std::env::temp_dir().join("minimoog-d-standalone");
    let _ = std::fs::create_dir_all(&p);
    p
}

fn presets_dir() -> PathBuf {
    let p = state_dir().join("presets");
    let _ = std::fs::create_dir_all(&p);
    p
}

fn last_path() -> PathBuf {
    state_dir().join("last.json")
}

/// Keep names filesystem-safe: letters, digits, space, - _ . (max 48).
pub fn sanitize_name(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_' || *c == '.')
        .collect::<String>()
        .trim()
        .to_string();
    while s.contains("  ") {
        s = s.replace("  ", " ");
    }
    if s.len() > 48 {
        s.truncate(48);
        s = s.trim().to_string();
    }
    s
}

fn preset_path(name: &str) -> Option<PathBuf> {
    let clean = sanitize_name(name);
    if clean.is_empty() || clean == "." || clean == ".." {
        return None;
    }
    Some(presets_dir().join(format!("{clean}.json")))
}

// --- last state ---

pub fn load_last() -> Option<Params> {
    let data = std::fs::read_to_string(last_path()).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save_last(p: &Params) {
    if let Ok(data) = serde_json::to_string_pretty(p) {
        let _ = std::fs::write(last_path(), data);
    }
}

// --- named presets ---

pub fn list_presets() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(presets_dir()) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|x| x.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort_by_key(|s| s.to_lowercase());
    out
}

pub fn load_preset(name: &str) -> Option<Params> {
    let data = std::fs::read_to_string(preset_path(name)?).ok()?;
    serde_json::from_str(&data).ok()
}

/// Returns the sanitized name actually used, or None on empty/invalid.
pub fn save_preset(name: &str, p: &Params) -> Option<String> {
    let path = preset_path(name)?;
    let clean = sanitize_name(name);
    if let Ok(data) = serde_json::to_string_pretty(p) {
        if std::fs::write(&path, data).is_ok() {
            return Some(clean);
        }
    }
    None
}

pub fn delete_preset(name: &str) -> bool {
    if let Some(path) = preset_path(name) {
        return std::fs::remove_file(path).is_ok();
    }
    false
}
