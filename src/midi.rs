//! MIDI input via midir: note on/off, pitch bend, mod wheel -> cutoff.
//!
//! Keeps the connection alive by returning it; caller must hold it.

use crate::params::Shared;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub struct MidiKeepAlive {
    _conn: midir::MidiInputConnection<()>,
    _client: midir::MidiInput,
}

pub fn start_manager(
    shared: Arc<Mutex<Shared>>,
    requested: Arc<Mutex<Option<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut active_name: Option<String> = None;
        let mut connection: Option<MidiKeepAlive> = None;
        loop {
            let desired = requested.lock().ok().and_then(|value| value.clone());
            if desired != active_name {
                connection = None;
                active_name = None;
                if let Some(name) = desired.clone() {
                    let requested_name = if name.is_empty() { None } else { Some(name.as_str()) };
                    connection = connect_first_available(shared.clone(), requested_name);
                    if connection.is_some() {
                        active_name = Some(name);
                    }
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    })
}

pub fn connect_first_available(
    shared: Arc<Mutex<Shared>>,
    requested_name: Option<&str>,
) -> Option<MidiKeepAlive> {
    let client = midir::MidiInput::new("miniunicorn").ok()?;
    let ports = client.ports();
    if ports.is_empty() {
        eprintln!("MIDI: no input ports found (virtual keyboard in UI still works)");
        return None;
    }
    eprintln!("MIDI: available inputs:");
    for (i, p) in ports.iter().enumerate() {
        let name = client.port_name(p).unwrap_or_else(|_| "?".into());
        eprintln!("  [{i}] {name}");
    }
    let port = requested_name
        .filter(|name| !name.is_empty())
        .and_then(|wanted| {
            ports.iter().find(|port| {
                client.port_name(port).map(|name| name == wanted).unwrap_or(false)
            }).cloned()
        })
        .or_else(|| ports.into_iter().next())?;
    let name = client
        .port_name(&port)
        .unwrap_or_else(|_| "midi-in".into());
    eprintln!("MIDI: connecting to {name}");

    // midir::connect consumes client; keep a fresh one for the struct is
    // awkward, so create the connection and store a dummy client.
    let conn = client
        .connect(
            &port,
            "miniunicorn",
            move |_stamp, msg: &[u8], _| {
                if let Ok(mut s) = shared.lock() {
                    handle_midi(&mut s, msg);
                }
            },
            (),
        )
        .ok()?;

    // Re-create a client handle just to keep type happy (unused).
    let dummy = midir::MidiInput::new("miniunicorn-holder").ok()?;
    Some(MidiKeepAlive {
        _conn: conn,
        _client: dummy,
    })
}

fn handle_midi(shared: &mut crate::params::Shared, msg: &[u8]) {
    let runtime = &mut shared.runtime;
    if msg.is_empty() {
        return;
    }
    let status = msg[0] & 0xF0;
    match status {
        0x90 => {
            // note on (vel 0 = note off)
            if msg.len() >= 3 {
                let note = msg[1];
                let vel = msg[2];
                if vel == 0 {
                    if runtime.note == Some(note) {
                        runtime.gate = false;
                        runtime.note = None;
                    }
                } else {
                    runtime.note = Some(note);
                    runtime.velocity = vel as f32 / 127.0;
                    runtime.gate = true;
                }
            }
        }
        0x80 => {
            if msg.len() >= 3 {
                let note = msg[1];
                if runtime.note == Some(note) {
                    runtime.gate = false;
                    runtime.note = None;
                }
            }
        }
        0xB0 => {
            // Manual §4.8: mod wheel adjusts MOD DEPTH.
            if msg.len() >= 3 && msg[1] == 1 {
                runtime.mod_wheel = (msg[2] as f32 / 127.0).clamp(0.0, 1.0);
            }
        }
        0xE0 => {
            // pitch bend
            if msg.len() >= 3 {
                let lsb = msg[1] as i32;
                let msb = msg[2] as i32;
                let raw = (msb << 7) | lsb; // 0..16383
                let centered = (raw - 8192) as f32 / 8192.0;
                runtime.bend_semi = centered * 2.0;
            }
        }
        _ => {}
    }
}
