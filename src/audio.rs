//! cpal audio backend: mono voice -> stereo out.

use crate::dsp::Voice;
use crate::params::Shared;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

pub fn start_stream(
    shared: Arc<Mutex<Shared>>,
    sample_rate_hint: Option<f32>,
    device_name: Option<&str>,
) -> anyhow_like::Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = if let Some(wanted) = device_name.filter(|name| !name.is_empty()) {
        host.output_devices()
            .map_err(|e| format!("listing output devices failed: {e}"))?
            .find(|candidate| candidate.name().map(|name| name == wanted).unwrap_or(false))
            .or_else(|| host.default_output_device())
            .ok_or_else(|| "no output audio device found".to_string())?
    } else {
        host.default_output_device()
            .ok_or_else(|| "no output audio device found".to_string())?
    };
    let mut supported: cpal::SupportedStreamConfig = device
        .default_output_config()
        .map_err(|e| format!("default output config failed: {e}"))?;

    if let Some(sr) = sample_rate_hint {
        // Keep device rate; Voice resamples state by sr, so just note it.
        let _ = sr;
    }
    // Force f32 stereo where possible for simplicity
    supported = cpal::SupportedStreamConfig::new(
        supported.channels().max(2),
        supported.sample_rate(),
        *supported.buffer_size(),
        supported.sample_format(),
    );
    let config: cpal::StreamConfig = supported.config().clone();
    let sr = config.sample_rate.0 as f32;
    let channels = config.channels as usize;

    let mut voice = Voice::new(sr);

    let err_fn = |err| eprintln!("audio stream error: {err}");

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                // Copy shared state once per buffer (cheap)
                let (params, runtime) = match shared.lock() {
                    Ok(s) => (s.params, s.runtime),
                    Err(_) => return,
                };
                if (voice.sample_rate() - sr).abs() > 1.0 {
                    voice.set_sample_rate(sr);
                }
                let mut i = 0;
                while i < data.len() {
                    let mono = voice.render(&params, &runtime);
                    // stereo duplicate, soft clip guard
                    let out = mono.clamp(-1.2, 1.2);
                    for ch in 0..channels {
                        if i + ch < data.len() {
                            data[i + ch] = out;
                        }
                    }
                    i += channels;
                }
                // Write back overload lamp + sequencer step (best-effort)
                if let Ok(mut s) = shared.try_lock() {
                    s.runtime.overload = voice.last_mix_peak;
                    s.runtime.seq_step = voice.last_seq_step;
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("build_output_stream failed: {e}"))?;

    stream.play().map_err(|e| format!("stream.play failed: {e}"))?;
    Ok(stream)
}

// Minimal Result alias to avoid adding anyhow dependency.
pub mod anyhow_like {
    pub type Result<T, E> = std::result::Result<T, E>;
}
