//! v2 free-canvas layout: grid-snapped drag & drop positions.
//!
//! Each panel section lives in an `egui::Area` at a stored position.
//! Edit mode shows drag handles; positions snap live to `grid` px and
//! persist to `layout-v2.json` next to the v1 state files.

use std::collections::{HashMap, HashSet};

/// A user divider line living on the sheet.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LineSpec {
    pub name: String,
    pub vertical: bool,
    /// Length in px (8..1200).
    pub len: f32,
    /// Thickness in px (1..24).
    pub thick: f32,
}

impl Default for LineSpec {
    fn default() -> Self {
        Self {
            name: "Line".to_string(),
            vertical: true,
            len: 120.0,
            thick: 3.0,
        }
    }
}

/// A user text label living on the sheet.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextSpec {
    pub content: String,
    pub size: f32,
    /// Wrap width in px (0 = natural single flow, manual breaks only).
    #[serde(default)]
    pub wrap: f32,
}

impl Default for TextSpec {
    fn default() -> Self {
        Self {
            content: "Text".to_string(),
            size: 13.0,
            wrap: 0.0,
        }
    }
}

/// A patch-bay jack living on the sheet.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JackSpec {
    pub label: String,
    pub is_input: bool,
    /// For inputs: id of the connected output (empty = unconnected).
    #[serde(default)]
    pub output_id: String,
    /// For MAIN: selected computer output device name.
    #[serde(default)]
    pub device_name: String,
}

impl Default for JackSpec {
    fn default() -> Self {
        Self {
            label: "JACK".to_string(),
            is_input: false,
            output_id: String::new(),
            device_name: String::new(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanvasLayout {
    /// Snap step in logical px.
    pub grid: f32,
    /// Section id -> [x, y] (screen space, top-left of the area).
    pub pos: HashMap<String, [f32; 2]>,
    /// Item ids rotated 90° (rockers/selectors with both orientations).
    #[serde(default)]
    pub rot90: HashSet<String>,
    /// Item ids hidden in play mode (ghosts in edit mode).
    #[serde(default)]
    pub hidden: HashSet<String>,
    /// User divider lines: id ("lineN") -> spec (position in `pos`).
    #[serde(default)]
    pub lines: HashMap<String, LineSpec>,
    /// Counter for fresh line ids/names.
    #[serde(default)]
    pub line_seq: u32,
    /// User text labels: id ("textN") -> spec (position in `pos`).
    #[serde(default)]
    pub texts: HashMap<String, TextSpec>,
    /// Counter for fresh text ids/names.
    #[serde(default)]
    pub text_seq: u32,
    /// Patch-bay jacks: id ("jackN") -> spec (position in `pos`).
    #[serde(default)]
    pub jacks: HashMap<String, JackSpec>,
    /// Counter for fresh jack ids.
    #[serde(default)]
    pub jack_seq: u32,
}

impl Default for CanvasLayout {
    fn default() -> Self {
        Self {
            grid: 8.0,
            pos: default_positions(),
            rot90: HashSet::new(),
            hidden: HashSet::new(),
            lines: HashMap::new(),
            line_seq: 0,
            texts: HashMap::new(),
            text_seq: 0,
            jacks: default_jacks(),
            jack_seq: 0,
        }
    }
}

/// Fresh factory arrangement: every knob/toggle flows row-major on a
/// 14-column grid (122 x 124 cells) below the patch strip; the wide
/// keyboard + sequencer groups park at the bottom. Everything is
/// user-movable afterwards.
pub fn default_positions() -> HashMap<String, [f32; 2]> {
    // Signal-flow order: controllers, osc bank, mixer, modifiers,
    // mod switches, output.
    let flow: &[&str] = &[
        "c_tune", "c_glide", "c_modmix", "c_aba", "c_abb", "c_moddepth", "c_lfoshape",
        "c_lforate", "c_oscmod", "o_name1", "o_range1", "o_wave1",
        "o_ref1", "o_name2", "o_range2", "o_freq2", "o_wave2", "o_name3", "o_range3",
        "o_freq3", "o_wave3", "o_ctrl3", "o_drone", "m_vol1", "m_sw1", "m_vol2", "m_sw2",
        "m_vol3", "m_sw3", "m_swext", "m_ext", "m_swnoise", "m_noise", "m_overload",
        "m_wpink", "f_cutoff", "f_emph", "f_contour", "f_flabel", "f_fa", "f_fd", "f_fs",
        "f_llabel", "f_aa", "f_ad", "f_as", "f_dhelp", "s_fmod", "s_mode", "s_key1",
        "s_key2", "s_fdec", "s_ldec", "s_khelp", "u_vol", "u_main", "u_a440", "u_phones",
        "u_power",
    ];
    const COLS: usize = 14;
    const DX: f32 = 122.0;
    const DY: f32 = 112.0;
    const X0: f32 = 10.0;
    const Y0: f32 = 90.0;
    let mut map = HashMap::new();
    for (i, id) in flow.iter().enumerate() {
        map.insert(
            id.to_string(),
            [X0 + (i % COLS) as f32 * DX, Y0 + (i / COLS) as f32 * DY],
        );
    }
    // Full-width strips / groups with explicit spots.
    map.insert("keys".to_string(), [10.0, 652.0]);
    map.insert("seq".to_string(), [10.0, 732.0]);
    // Patch-bay jacks in a row (standard Minimoog Model D assignments).
    let jacks: &[(&str, &str, bool)] = &[
        ("jack_mods", "MOD SOURCE", true),
        ("jack_osc1v", "OSC1V/OCT", true),
        ("jack_lfocv", "LFO CV", true),
        ("jack_lfosaw", "LFO SAW", false),
        ("jack_lfosqr", "LFO SQR", false),
        ("jack_ext", "EXT", true),
        ("jack_mix", "MIX", false),
        ("jack_cutcv", "CUT CV", true),
        ("jack_fcgate", "FC GATE", true),
        ("jack_filtcont", "FILT CONT", false),
        ("jack_lcgate", "LC GATE", true),
        ("jack_loudcont", "LOUD CONT", false),
        ("jack_loudcv", "LOUD CV", true),
        ("jack_phones", "PHONES", false),
        ("jack_main", "MAIN", false),
        ("jack_midiin", "MIDI IN", true),
    ];
    const JACK_DX: f32 = 120.0;
    for (i, (id, _, _)) in jacks.iter().enumerate() {
        let position = if *id == "jack_main" {
            // Keep MAIN visible on narrower windows; it remains independently
            // movable and saved positions still override this default.
            [8.0, 160.0]
        } else if *id == "jack_midiin" {
            [128.0, 160.0]
        } else {
            [8.0 + i as f32 * JACK_DX, 40.0]
        };
        map.insert(id.to_string(), position);
    }
    map
}

/// Factory patch-bay definitions. Positions remain independently editable in
/// `CanvasLayout::pos`; this function only defines the jack identities.
pub fn default_jacks() -> HashMap<String, JackSpec> {
    let defs: &[(&str, &str, bool)] = &[
        ("jack_mods", "MOD SOURCE", true),
        ("jack_osc1v", "OSC1V/OCT", true),
        ("jack_lfocv", "LFO CV", true),
        ("jack_lfosaw", "LFO SAW", false),
        ("jack_lfosqr", "LFO SQR", false),
        ("jack_ext", "EXT", true),
        ("jack_mix", "MIX", false),
        ("jack_cutcv", "CUT CV", true),
        ("jack_fcgate", "FC GATE", true),
        ("jack_filtcont", "FILT CONT", false),
        ("jack_lcgate", "LC GATE", true),
        ("jack_loudcont", "LOUD CONT", false),
        ("jack_loudcv", "LOUD CV", true),
        ("jack_phones", "PHONES", false),
        ("jack_main", "MAIN", false),
        ("jack_midiin", "MIDI IN", true),
    ];
    defs.iter()
        .map(|(id, label, is_input)| {
            (
                (*id).to_string(),
                JackSpec {
                    label: (*label).to_string(),
                    is_input: *is_input,
                    output_id: String::new(),
                    device_name: String::new(),
                },
            )
        })
        .collect()
}

fn layout_path() -> std::path::PathBuf {
    layouts_dir().join("layout-v2.json")
}

fn layouts_dir() -> std::path::PathBuf {
    #[cfg(windows)]
    let mut dir = std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    #[cfg(not(windows))]
    let mut dir = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    #[cfg(windows)]
    dir.push("minimoog-d-standalone");
    #[cfg(not(windows))]
    dir.push(".minimoog-d-standalone");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn layout_name_path(name: &str) -> Option<std::path::PathBuf> {
    let clean: String = name
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .collect();
    if clean.is_empty() {
        return None;
    }
    Some(layouts_dir().join(format!("layout-{clean}.json")))
}

pub fn named_names() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(layouts_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(name) = name.strip_prefix("layout-").and_then(|n| n.strip_suffix(".json")) {
                if name != "v2" {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

pub fn save_named(name: &str, layout: &CanvasLayout) -> bool {
    let Some(path) = layout_name_path(name) else { return false };
    serde_json::to_string_pretty(layout)
        .ok()
        .and_then(|data| std::fs::write(path, data).ok())
        .is_some()
}

pub fn load_named(name: &str) -> Option<CanvasLayout> {
    let path = layout_name_path(name)?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn load() -> CanvasLayout {
    let mut layout = CanvasLayout::default();
    if let Ok(data) = std::fs::read_to_string(layout_path()) {
        if let Ok(saved) = serde_json::from_str::<CanvasLayout>(&data) {
            if saved.grid >= 1.0 && saved.grid <= 64.0 {
                layout.grid = saved.grid;
            }
            // User lines + texts + jacks load whole (spec + position).
            layout.lines = saved.lines;
            layout.line_seq = saved.line_seq;
            layout.texts = saved.texts;
            layout.text_seq = saved.text_seq;
            layout.jack_seq = saved.jack_seq;
            // Keep factory jack definitions when loading older layouts, while
            // preserving any saved routing for known jacks.
            for (id, spec) in saved.jacks {
                if let Some(factory) = layout.jacks.get(&id) {
                    let mut spec = spec;
                    // Directions are factory-defined; old saved layouts may
                    // predate the OSC1V/OCT input correction.
                    spec.is_input = factory.is_input;
                    layout.jacks.insert(id, spec);
                }
            }
            // Saved positions win; unknown ids are ignored, missing ids
            // keep defaults so new controls still appear.
            for (k, v) in saved.pos {
                if layout.pos.contains_key(&k)
                    || layout.lines.contains_key(&k)
                    || layout.texts.contains_key(&k)
                    || layout.jacks.contains_key(&k)
                {
                    layout.pos.insert(k, v);
                }
            }
            // Styling sets only for ids we still know (factory or lines).
            for k in saved.rot90 {
                if layout.pos.contains_key(&k) {
                    layout.rot90.insert(k);
                }
            }
            for k in saved.hidden {
                if layout.pos.contains_key(&k) {
                    layout.hidden.insert(k);
                }
            }
        }
    }
    layout
}

pub fn save(layout: &CanvasLayout) {
    if let Ok(data) = serde_json::to_string_pretty(layout) {
        let _ = std::fs::write(layout_path(), data);
    }
}

pub fn snap(v: f32, grid: f32) -> f32 {
    if grid < 1.0 {
        v
    } else {
        (v / grid).round() * grid
    }
}
