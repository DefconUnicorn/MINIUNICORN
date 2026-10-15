mod audio;
mod dsp;
mod layout;
mod midi;
mod params;
mod presets;
mod ui;

use params::Shared;
use std::sync::{Arc, Mutex};

fn main() {
    // Restore the user's last working state if present.
    let mut initial = Shared::default();
    if let Some(last) = presets::load_last() {
        initial.params = last;
        println!("restored last state");
    }
    // Sequencer must never auto-play on startup, even if it was playing on close.
    initial.params.seq_on = false;
    let shared = Arc::new(Mutex::new(initial));
    let saved_layout = layout::load();
    let main_device = saved_layout
        .jacks
        .get("jack_main")
        .map(|jack| jack.device_name.as_str());

    // Audio first (needs to stay alive while UI runs)
    let stream = match audio::start_stream(shared.clone(), None, main_device) {
        Ok(s) => {
            println!("audio: stream started");
            Some(s)
        }
        Err(e) => {
            eprintln!("audio failed: {e}");
            eprintln!("The UI will still open so you can inspect parameters.");
            None
        }
    };

    // MIDI (optional)
    let midi_device = saved_layout
        .jacks
        .get("jack_midiin")
        .map(|jack| jack.device_name.clone())
        .filter(|name| !name.is_empty());
    let midi_selection = Arc::new(Mutex::new(Some(midi_device.unwrap_or_default())));
    let _midi = midi::start_manager(shared.clone(), midi_selection.clone());
    let midi_status = "MIDI input manager ready".to_string();

    // UI blocks until window closes; keep stream + midi alive
    ui::run(shared.clone(), midi_status, midi_selection);

    drop(stream);
}
