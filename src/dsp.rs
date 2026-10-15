//! Mono Model-D-style DSP core.
//!
//! - 3x PolyBLEP oscillators (triangle / saw / square) with drift + range + LO mode
//! - White/pink noise
//! - Saturating mixer (tanh) with OVERLOAD level
//! - Huovilainen-inspired nonlinear transistor ladder filter, 2x oversampled
//! - Separate FILTER CONTOUR and LOUDNESS CONTOUR ADS+R, exponential glide
//!
//! Dependency-free, no allocation in `render()`.

use crate::params::{
    self, midi_note_to_freq, OscWave, Params, Runtime,
};
use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn poly_blep(phase: f32, dt: f32) -> f32 {
    if phase < dt {
        let t = phase / dt;
        2.0 * t - t * t - 1.0
    } else if phase > 1.0 - dt {
        let t = (phase - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

fn pulse_sample(phase: f32, dt: f32, pw: f32) -> f32 {
    let pw = pw.clamp(0.03, 0.97);
    let mut naive = if phase < pw { 1.0 } else { -1.0 };
    naive += poly_blep(phase, dt);
    let phase2 = (phase + 1.0 - pw) % 1.0;
    naive -= poly_blep(phase2, dt);
    naive * 0.9
}

fn tri_sample(phase: f32) -> f32 {
    4.0 * (phase - (phase + 0.25).floor() - 0.25).abs() - 1.0
}

fn osc_sample(wave: OscWave, phase: f32, dt: f32) -> f32 {
    match wave {
        OscWave::Triangle => tri_sample(phase),
        OscWave::TriSaw => {
            // Manual §2: triangular/saw blend (OSC 1+2)
            let saw = 2.0 * phase - 1.0 - poly_blep(phase, dt);
            0.5 * (tri_sample(phase) + saw)
        }
        OscWave::Saw => {
            let naive = 2.0 * phase - 1.0;
            naive - poly_blep(phase, dt)
        }
        OscWave::ReverseSaw => {
            // Manual §2: reverse saw (OSC 3)
            let naive = 1.0 - 2.0 * phase;
            naive + poly_blep(phase, dt)
        }
        OscWave::Square => pulse_sample(phase, dt, 0.5),
        OscWave::WidePulse => pulse_sample(phase, dt, 0.3),
        OscWave::NarrowPulse => pulse_sample(phase, dt, 0.1),
    }
}

// ---------------------------------------------------------------------------
// ADS+R envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Debug, Clone)]
pub struct AdsEnv {
    stage: EnvStage,
    level: f32,
    sample_rate: f32,
}

impl AdsEnv {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            stage: EnvStage::Idle,
            level: 0.0,
            sample_rate,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
    }

    pub fn gate_on(&mut self) {
        self.stage = EnvStage::Attack;
    }

    pub fn gate_off(&mut self) {
        if self.stage != EnvStage::Idle {
            self.stage = EnvStage::Release;
        }
    }

    fn coeff(&self, secs: f32) -> f32 {
        let t = secs.max(0.001);
        (-1.0 / (t * self.sample_rate)).exp()
    }

    pub fn process(&mut self, attack: f32, decay: f32, sustain: f32, release: f32) -> f32 {
        let a = attack.max(0.001);
        let d = decay.max(0.005);
        let s = clamp01(sustain);
        let r = release.max(0.005);

        match self.stage {
            EnvStage::Idle => self.level = 0.0,
            EnvStage::Attack => {
                let step = 1.0 / (a * self.sample_rate);
                self.level += step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
            }
            EnvStage::Decay => {
                let c = self.coeff(d);
                self.level = s + (self.level - s) * c;
                if (self.level - s).abs() < 0.0008 {
                    self.level = s;
                    self.stage = EnvStage::Sustain;
                }
            }
            EnvStage::Sustain => self.level = s,
            EnvStage::Release => {
                let c = self.coeff(r);
                self.level *= c;
                if self.level < 0.0004 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
            }
        }
        self.level
    }

    pub fn is_active(&self) -> bool {
        self.stage != EnvStage::Idle
    }
}

// ---------------------------------------------------------------------------
// Nonlinear ladder filter (Huovilainen-inspired)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LadderFilter {
    s: [f32; 4],
    g: f32,
    k: f32,
    sample_rate_os: f32,
}

impl LadderFilter {
    pub fn new(sample_rate_os: f32) -> Self {
        Self {
            s: [0.0; 4],
            g: 0.1,
            k: 1.0,
            sample_rate_os,
        }
    }

    pub fn set_sample_rate_os(&mut self, sr_os: f32) {
        self.sample_rate_os = sr_os;
    }

    pub fn reset(&mut self) {
        self.s = [0.0; 4];
    }

    fn update_coeffs(&mut self, cutoff: f32, resonance: f32) {
        let fc = cutoff.clamp(20.0, 18000.0);
        let g = 1.0 - (-2.0 * PI * fc / self.sample_rate_os).exp();
        self.g = g.clamp(0.01, 0.9);
        self.k = (clamp01(resonance) * 4.0).clamp(0.0, 3.95);
    }

    #[inline]
    pub fn process_one_os(&mut self, x: f32) -> f32 {
        let g = self.g;
        let k = self.k;
        let u = (x - k * self.s[3]).tanh();
        self.s[0] += g * (u - self.s[0].tanh());
        self.s[1] += g * (self.s[0].tanh() - self.s[1].tanh());
        self.s[2] += g * (self.s[1].tanh() - self.s[2].tanh());
        self.s[3] += g * (self.s[2].tanh() - self.s[3].tanh());
        self.s[3]
    }

    pub fn process(&mut self, x: f32, cutoff: f32, resonance: f32) -> f32 {
        self.update_coeffs(cutoff, resonance);
        let y1 = self.process_one_os(x);
        let y2 = self.process_one_os(x);
        0.5 * (y1 + y2)
    }
}

// Simple deterministic white noise (xorshift) + pink-ish lowpassed version
#[derive(Debug, Clone)]
struct NoiseGen {
    state: u32,
    pink_b: [f32; 6],
}

impl NoiseGen {
    fn new() -> Self {
        Self {
            state: 0x12345678,
            pink_b: [0.0; 6],
        }
    }
    fn white(&mut self) -> f32 {
        // xorshift32 -> -1..1
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
    fn pink(&mut self) -> f32 {
        // Paul Kellet-ish pink approximation
        let w = self.white();
        let b = &mut self.pink_b;
        b[0] = 0.99886 * b[0] + w * 0.0555179;
        b[1] = 0.99332 * b[1] + w * 0.0750759;
        b[2] = 0.96900 * b[2] + w * 0.1538520;
        b[3] = 0.86650 * b[3] + w * 0.3104856;
        b[4] = 0.55000 * b[4] + w * 0.5329522;
        b[5] = -0.7616 * b[5] - w * 0.0168980;
        (b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[5] + w * 0.5362) * 0.11
    }
}

// ---------------------------------------------------------------------------
// Mono voice
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Voice {
    sample_rate: f32,
    phase: [f32; 3],
    drift_phase: [f32; 3],
    glide_freq: f32,
    last_gate: bool,
    filter: LadderFilter,
    amp_env: AdsEnv,
    filt_env: AdsEnv,
    smooth_cutoff: f32,
    noise: NoiseGen,
    lfo_phase: f32,
    mod_noise_hold: f32,
    prev_out: f32,
    a440_phase: f32,
    seq_pos: usize,
    seq_counter: u32,
    pub last_mix_peak: f32,
    pub last_seq_step: u8,
}

impl Voice {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phase: [0.0, 0.33, 0.66],
            drift_phase: [0.0, 0.37, 0.71],
            glide_freq: 55.0,
            last_gate: false,
            filter: LadderFilter::new(sample_rate * 2.0),
            amp_env: AdsEnv::new(sample_rate),
            filt_env: AdsEnv::new(sample_rate),
            smooth_cutoff: 1200.0,
            noise: NoiseGen::new(),
            lfo_phase: 0.0,
            mod_noise_hold: 0.0,
            prev_out: 0.0,
            a440_phase: 0.0,
            seq_pos: 0,
            seq_counter: 0,
            last_mix_peak: 0.0,
            last_seq_step: 0,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        self.filter.set_sample_rate_os(sr * 2.0);
        self.amp_env.set_sample_rate(sr);
        self.filt_env.set_sample_rate(sr);
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn reset(&mut self) {
        self.phase = [0.0, 0.33, 0.66];
        self.filter.reset();
    }

    fn glide_coeff(&self, secs: f32) -> f32 {
        if secs <= 0.0005 {
            return 1.0;
        }
        (-1.0 / (secs * self.sample_rate)).exp()
    }

    pub fn render(&mut self, p: &Params, r: &Runtime) -> f32 {
        if !p.power_on {
            return 0.0;
        }
        // The phones jack is an internal post-output bus. It is deliberately
        // independent of MAIN volume, and can be patched back into inputs.
        let phone_routes = p.phones_route_mask;
        let phone_bus = (self.prev_out * params::knob_0_10_to_volume(p.phones_0_10))
            .clamp(-1.2, 1.2);
        let phones_to = |index: usize| {
            p.jack_routes[index] == 6
                || (p.jack_routes[index] == 0 && phone_routes & (1 << index) != 0)
        };
        let phone_loud_gain = if p.jack_routes[7] == 0 && phones_to(7) {
            (1.0 + phone_bus * 0.5).clamp(0.0, 2.0)
        } else {
            1.0
        };
        // --- 16-step sequencer overrides keys when ON ---
        // 16th notes at seq BPM. Rest (note 0) = gate off.
        let mut seq_note: Option<u8> = None;
        let mut seq_gate = false;
        if p.seq_on {
            let bpm = params::knob_0_10_to_bpm(p.seq_bpm_0_10).clamp(30.0, 300.0);
            let step_samples = ((self.sample_rate * 60.0 / bpm / 4.0) as u32).max(64);
            self.seq_counter += 1;
            if self.seq_counter >= step_samples {
                self.seq_counter = 0;
                self.seq_pos = (self.seq_pos + 1) % 16;
            }
            let n = p.seq_notes[self.seq_pos];
            if n > 0 {
                seq_note = Some(n);
                seq_gate = true;
            }
            self.last_seq_step = self.seq_pos as u8;
        } else {
            self.seq_pos = 0;
            self.seq_counter = 0;
            self.last_seq_step = 0;
        }
        let eff_note = seq_note.or(r.note);
        let phone_gate = (phones_to(5) || phones_to(6))
            && phone_bus.abs() > 0.05;
        let eff_gate = (if p.seq_on { seq_gate } else { r.gate }) || phone_gate;
        if eff_gate && !self.last_gate {
            self.amp_env.gate_on();
            self.filt_env.gate_on();
        } else if !eff_gate && self.last_gate {
            self.amp_env.gate_off();
            self.filt_env.gate_off();
        }
        self.last_gate = eff_gate;

        // --- note + tune (A-440 no longer hijacks the voice; it is a
        // separate solid reference drone mixed at the output below) ---
        let note = eff_note.unwrap_or(48) as f32;
        let mut target = midi_note_to_freq(note as u8);
        let master_tune = params::knob_0_10_to_tune_semi(p.tune_0_10);
        target *= 2.0f32.powf((master_tune + r.bend_semi) / 12.0);

        let glide = params::knob_0_10_to_glide(p.glide_0_10);
        let c = self.glide_coeff(glide);
        if c >= 1.0 {
            self.glide_freq = target;
        } else {
            self.glide_freq += (target - self.glide_freq) * (1.0 - c);
        }
        let mut base = self.glide_freq.max(20.0).min(4000.0);

        // --- Envelopes first (FILTER EG can be a mod source, manual §4.8) ---
        // v2: RELEASE knob removed — fixed middle value (≈25 ms).
        let rel_knob = params::knob_0_10_to_decay(2.0);
        let fa = params::knob_0_10_to_attack(p.filt_attack_0_10);
        let fd = params::knob_0_10_to_decay(p.filt_decay_0_10);
        let fs = p.filt_sustain_0_10 / 10.0;
        let aa = params::knob_0_10_to_attack(p.amp_attack_0_10);
        let ad = params::knob_0_10_to_decay(p.amp_decay_0_10);
        let asus = p.amp_sustain_0_10 / 10.0;
        // Manual (30): DECAY ON = release takes DECAY time; OFF = immediate.
        // Fixed RELEASE trim extends release when switch is ON.
        let rel_filt = if p.filter_decay_on {
            fd.max(rel_knob * 0.5)
        } else {
            0.015
        };
        let rel_amp = if p.loud_decay_on {
            ad.max(rel_knob * 0.5)
        } else {
            0.015
        };
        // Peek envelopes without advancing twice: advance once here, reuse
        // values below (filter contour needed as mod source).
        // NOTE: AdsEnv advances per call; call once per sample here.
        let f_env_peek = self.filt_env.process(fa, fd, fs, rel_filt);
        let a_env_peek = self.amp_env.process(aa, ad, asus, rel_amp);
        let f_env = f_env_peek;
        let a_env = a_env_peek;

        // --- Modulation bus (manual §3.1.2, §4.8) ---
        // Source A (4): OSC3 audio vs Filter Envelope.
        // Source B (9): Noise (or ext) vs dedicated LFO (11: tri/square, 12: rate).
        // (8) MOD MIX blends A <> B. (10) MOD DEPTH (+ mod wheel) = amount.
        // (7) OSC MOD routes bus to osc pitch; (32) FILTER MOD routes to filter.
        let sr_pre = self.sample_rate;
        let lfo_hz = params::knob_0_10_to_lfo_hz(p.lfo_rate_0_10).clamp(0.02, 300.0);
        self.lfo_phase += lfo_hz / sr_pre;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }
        let lfo = if p.lfo_square {
            if self.lfo_phase < 0.5 { 1.0 } else { -1.0 }
        } else {
            // triangle from phase
            if self.lfo_phase < 0.5 {
                4.0 * self.lfo_phase - 1.0
            } else {
                3.0 - 4.0 * self.lfo_phase
            }
        };
        // smooth noise source so vibrato isn't harsh FM
        self.mod_noise_hold = self.mod_noise_hold * 0.998 + self.noise.white() * 0.002;
        // OSC3 audio sample for source A: compute its phase now (without
        // pitch-mod on itself to avoid feedback; manual modulates all three
        // but self-FM at audio rate is unstable in naive DSP).
        let fine3 = params::knob_0_10_to_fine_semi(p.osc3_freq_0_10);
        let range3 = p.osc3_range_oct as f32 * 12.0;
        let mut f3 = base * 2.0f32.powf((range3 + fine3) / 12.0);
        // RANGE = LO (-3): sub-audio drone. Without keyboard control it is a
        // free drone (~6 Hz); there is no separate LO switch on the hardware.
        if p.osc3_range_oct <= -3 {
            f3 = if p.osc3_kb_ctrl {
                (base * 2.0f32.powf((range3 + fine3) / 12.0)).max(0.3)
            } else {
                6.0 * 2.0f32.powf(fine3 / 12.0)
            };
        }
        let dt3 = (f3 / sr_pre).clamp(0.00005, 0.25);
        // advance osc3 phase + drift (drift applied in main loop too; keep
        // simple by advancing here and reusing sample)
        // v2: DRIFT knob removed — fixed gentle amount (was 1.5/10).
        let drift_amt_pre = 0.15;
        self.drift_phase[2] += 0.5 / sr_pre;
        if self.drift_phase[2] >= 1.0 {
            self.drift_phase[2] -= 1.0;
        }
        let wob3 = (2.0 * PI * self.drift_phase[2]).sin();
        let f3d = f3 * 2.0f32.powf(wob3 * 4.0 * drift_amt_pre / 1200.0);
        let dt3d = (f3d / sr_pre).clamp(0.00005, 0.25);
        self.phase[2] += dt3d;
        if self.phase[2] >= 1.0 {
            self.phase[2] -= 1.0;
        }
        let osc3_sample = osc_sample(p.osc3_wave, self.phase[2], dt3d);
        let _ = (dt3, f3);

        let src_a = if p.mod_src_a_is_filter_eg {
            f_env * 2.0 - 1.0 // bipolar envelope
        } else {
            osc3_sample
        };
        let src_b = if p.mod_src_b_is_lfo {
            lfo
        } else {
            self.mod_noise_hold.clamp(-1.0, 1.0) * 3.0
        };
        let mix_t = (p.mod_mix_0_10 / 10.0).clamp(0.0, 1.0);
        let mod_bus = src_a * (1.0 - mix_t) + src_b * mix_t;
        let lfo_saw = self.lfo_phase * 2.0 - 1.0;
        let lfo_sqr = if self.lfo_phase < 0.5 { 1.0 } else { -1.0 };
        let route_signal = |code: u8, mix_signal: f32| match code {
            1 => lfo_saw,
            2 => lfo_sqr,
            3 => mix_signal,
            4 => f_env * 2.0 - 1.0,
            5 => a_env,
            6 => phone_bus,
            7 => self.prev_out,
            _ => 0.0,
        };
        let loud_route_gain = if p.jack_routes[7] != 0 {
            (1.0 + route_signal(p.jack_routes[7], 0.0) * 0.5).clamp(0.0, 2.0)
        } else {
            1.0
        };
        let depth_total = ((p.mod_depth_0_10 / 10.0) + r.mod_wheel).clamp(0.0, 1.5);
        let mut mod_cents = 0.0;
        let mut mod_filter_oct = 0.0;
        if p.osc_mod_on && depth_total > 0.001 {
            // Manual (7): oscillators modulated by mix. ±80 cents max.
            mod_cents = mod_bus * depth_total * 80.0;
        }
        if p.filter_mod_on && depth_total > 0.001 {
            // Manual (32): filter modulated by mix. ±2 oct max.
            mod_filter_oct = mod_bus * depth_total * 2.0;
        }
        if p.jack_routes[1] != 0 {
            target *= 2.0f32.powf(route_signal(p.jack_routes[1], 0.0));
        } else if phones_to(1) {
            target *= 2.0f32.powf(phone_bus);
        }
        base *= 2.0f32.powf(mod_cents / 1200.0);
        let base = base.max(20.0).min(4000.0);

        let fine = [
            0.0, // OSC 1 is the reference: no detune knob on the hardware
            params::knob_0_10_to_fine_semi(p.osc2_freq_0_10),
            params::knob_0_10_to_fine_semi(p.osc3_freq_0_10),
        ];
        let ranges = [
            p.osc1_range_oct as f32 * 12.0,
            p.osc2_range_oct as f32 * 12.0,
            p.osc3_range_oct as f32 * 12.0,
        ];
        let waves = [p.osc1_wave, p.osc2_wave, p.osc3_wave];
        let ons = [p.osc1_on, p.osc2_on, p.osc3_on];
        let levels = [
            params::knob_0_10_to_level(p.osc1_level_0_10),
            params::knob_0_10_to_level(p.osc2_level_0_10),
            params::knob_0_10_to_level(p.osc3_level_0_10),
        ];

        let sr = self.sample_rate;
        // v2: DRIFT knob removed — fixed gentle amount (was 1.5/10).
        let drift_amt = 0.15;
        let mut mix = 0.0;
        for i in 0..2 {
            if !ons[i] {
                continue;
            }
            let f = base * 2.0f32.powf((ranges[i] + fine[i]) / 12.0);
            self.drift_phase[i] += 0.5 / sr;
            if self.drift_phase[i] >= 1.0 {
                self.drift_phase[i] -= 1.0;
            }
            let wob = (2.0 * PI * self.drift_phase[i]).sin()
                + 0.5 * (2.0 * PI * self.drift_phase[i] * 2.7 + 1.3).sin();
            let cents = wob * 4.0 * drift_amt;
            let f_d = f * 2.0f32.powf(cents / 1200.0);

            let dt = (f_d / sr).clamp(0.00005, 0.25);
            self.phase[i] += dt;
            if self.phase[i] >= 1.0 {
                self.phase[i] -= 1.0;
            }
            mix += osc_sample(waves[i], self.phase[i], dt) * levels[i];
        }
        // OSC3: reuse sample computed for mod bus (phase already advanced).
        if p.osc3_on {
            mix += osc3_sample * levels[2];
        }

        // Noise (22)
        if p.noise_on {
                let n = if p.noise_pink {
                    self.noise.pink() * 2.0
                } else {
                    self.noise.white()
                };
            mix += n * params::knob_0_10_to_level(p.noise_level_0_10) * 0.9;
        }

        // EXT feedback (25): nothing patched -> main out feeds back for
        // extra bass/crunch. Gain <1 keeps it stable through the tanh.
        // The manual's EXT selector chooses external input when ON; with no
        // external source selected, the main output feeds back into EXT.
        let ext_has_patch_source = p.ext_on && p.jack_routes[3] != 0;
        if !ext_has_patch_source {
            let fb = (self.prev_out
                * params::knob_0_10_to_level(p.ext_level_0_10)
                * 0.85)
                .clamp(-1.0, 1.0);
            mix += fb;
        }
        if p.ext_on && p.jack_routes[3] != 0 {
            mix += route_signal(p.jack_routes[3], mix)
                * params::knob_0_10_to_level(p.ext_level_0_10)
                * 0.85;
        } else if p.ext_on && phones_to(3) {
            mix += phone_bus * params::knob_0_10_to_level(p.ext_level_0_10) * 0.85;
        }

        // Mixer saturation + overload tracking (24)
        let pre = mix * 0.55 * 1.8;
        self.last_mix_peak = self.last_mix_peak * 0.999 + pre.abs() * 0.001;
        let driven = pre.tanh() * 1.2;

        // (f_env/a_env already advanced once at top of render for mod bus.)

        // Filter cutoff: knob + contour + velocity + kb tracking (manual §4.6)
        let base_cut = params::knob_0_10_to_cutoff(p.cutoff_0_10);
        let env_oct = (p.contour_amt_0_10 / 10.0) * 6.0; // 0..6 octaves
        let mut cutoff_mod = 2.0f32.powf(f_env * env_oct);
        if p.jack_routes[4] != 0 {
            let cv = route_signal(p.jack_routes[4], mix);
            cutoff_mod *= 2.0f32.powf(cv * 2.0);
        } else if phones_to(4) {
            cutoff_mod *= 2.0f32.powf(phone_bus * 2.0);
        }
        if !p.filter_mod_on {
            // (32) OFF keeps contour; mod-bus wobble still applied below
            // only via mod_filter_oct when FILTER MOD is on. Zero it here.
            mod_filter_oct = 0.0;
            let _ = &mut cutoff_mod;
        }
        // (31) Keyboard tracking: OFF/OFF=none, 1=1/3, 2=2/3, both=max(full)
        let track: f32 = match (p.kb_control_1, p.kb_control_2) {
            (false, false) => 0.0,
            (true, false) => 1.0 / 3.0,
            (false, true) => 2.0 / 3.0,
            (true, true) => 1.0,
        };
        let kb_mult = 2.0f32.powf(track * (note - 60.0) / 12.0);
        let mut cutoff = base_cut * cutoff_mod * kb_mult * (0.75 + 0.25 * r.velocity.clamp(0.0, 1.0));
        // (32) filter mod bus (±2 oct pre-scaled in mod section)
        cutoff *= 2.0f32.powf(mod_filter_oct);
        if p.filter_mode_hi {
            cutoff *= 1.5; // HI range lift (hardware filter mode switch)
        }
        cutoff = cutoff.clamp(30.0, 17000.0);

        let smooth_c = (-1.0 / (0.005 * sr)).exp();
        self.smooth_cutoff = self.smooth_cutoff * smooth_c + cutoff * (1.0 - smooth_c);

        let reso = p.reso_0_10 / 10.0;

        // (44)(45)(47) VOLUME + MAIN ON. Standalone has one stereo out;
        // PHONES trim is disabled in the UI (kept in place, no-op).
        let out_gain = params::knob_0_10_to_volume(p.volume_0_10)
            * if p.main_on { 1.0 } else { 0.0 };

        // A-440 reference: solid 440 Hz sine, independent of gate/filter/
        // envelope so oscs can be tuned to it by ear. Scaled by output
        // gain (VOLUME/MAIN) like the hardware output bus.
        let a440 = if p.a440_on {
            self.a440_phase += 440.0 / sr;
            if self.a440_phase >= 1.0 {
                self.a440_phase -= 1.0;
            }
            (2.0 * PI * self.a440_phase).sin() * 0.3 * out_gain * 2.0
        } else {
            0.0
        };

        if !self.amp_env.is_active() && !eff_gate {
            let tail = self.filter.process(0.0, self.smooth_cutoff, reso);
            let o = tail * a_env * out_gain * 2.0 * phone_loud_gain * loud_route_gain + a440;
            self.prev_out = o.clamp(-1.2, 1.2);
            return self.prev_out;
        }

        let lp = self.filter.process(driven, self.smooth_cutoff, reso);

        let vel = if r.velocity <= 0.0 { 0.9 } else { r.velocity.clamp(0.1, 1.0) };
        let vca = (lp * (0.6 + 0.9 * a_env * vel) * 1.4).tanh();
        let o = vca * a_env.max(0.0) * out_gain * 2.4 * phone_loud_gain * loud_route_gain + a440;
        self.prev_out = o.clamp(-1.2, 1.2);
        self.prev_out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{Params, Runtime};

    #[test]
    fn voice_renders_finite_with_gate() {
        let mut v = Voice::new(44100.0);
        let p = Params::default();
        let r = Runtime {
            note: Some(69),
            velocity: 0.9,
            bend_semi: 0.0,
            mod_wheel: 0.0,
            gate: true,
            overload: 0.0,
            seq_step: 0,
        };
        let mut rms = 0.0;
        for _ in 0..44100 {
            let s = v.render(&p, &r);
            assert!(s.is_finite(), "non-finite sample: {s}");
            rms += s * s;
        }
        rms = (rms / 44100.0).sqrt();
        assert!(rms > 0.001, "voice too quiet, rms={rms}");
        assert!(rms < 2.0, "voice blowing up, rms={rms}");
    }

    #[test]
    fn filter_stable_at_high_reso() {
        let mut f = LadderFilter::new(88200.0);
        for _ in 0..10000 {
            let y = f.process(0.8, 8000.0, 0.95);
            assert!(y.is_finite());
            assert!(y.abs() < 5.0, "filter unstable: {y}");
        }
    }
}
