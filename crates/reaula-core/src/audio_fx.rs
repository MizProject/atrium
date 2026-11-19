//! Real-time audio FFT spectrum analysis and WIN 60 HE per-key RGB visualizers.
//!
//! All signal processing uses pure Rust `std` (Hann-windowed 1024-point radix-2 FFT,
//! log-spaced perceptual bands, adaptive auto-gain, and low-band onset detection) and
//! maps audio energy onto the 61 physical keys from [`crate::keymap::WIN60HE_KEYS`].

use std::f32::consts::PI;

use serde::{Deserialize, Serialize};

use crate::keymap::{KEYBOARD_CANVAS_HEIGHT, KEYBOARD_CANVAS_WIDTH, WIN60HE_KEYS};
use crate::protocol::MATRIX_SLOTS;

pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
pub const FFT_SIZE: usize = 1024;
pub const NUM_BANDS: usize = 14;

const F_LO_HZ: f32 = 40.0;
const F_HI_HZ: f32 = 16_000.0;
const NOISE_FLOOR_DBFS: f32 = -64.0;
const REF_FLOOR_DBFS: f32 = -46.0;
const DISPLAY_RANGE_DB: f32 = 36.0;
const REF_RELEASE_DB_PER_S: f32 = 5.0;
const LEVEL_RELEASE_TAU_S: f32 = 0.11;
const BEAT_BANDS: usize = 6;
const BEAT_K: f32 = 1.5;
const BEAT_MIN_FLUX_DB: f32 = 5.5;
const BEAT_COOLDOWN_S: f32 = 0.22;
const BEAT_STATS_TAU_S: f32 = 0.9;
const BEAT_ENV_TAU_S: f32 = 0.14;
const PEAK_HOLD_DROP_PER_S: f32 = 0.85;
const EPS: f32 = 1e-12;

/// Effect modes rendered onto the 61 physical WIN 60 HE keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioEffectStyle {
    /// 14 log-frequency vertical bars rising bottom-to-top across the 5 keyboard rows.
    Spectrum,
    /// Radial bass shockwave and mid/treble ring shimmer centered on the keyboard.
    Pulse,
    /// Left-to-right scrolling spectrogram colored by spectral centroid and beat brightness.
    Wave,
    /// Horizontal full-board loudness VU meter with decaying peak-hold cap.
    Meter,
}

impl AudioEffectStyle {
    pub const ALL: [Self; 4] = [Self::Spectrum, Self::Pulse, Self::Wave, Self::Meter];

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "spectrum" | "bars" | "eq" => Some(Self::Spectrum),
            "pulse" | "radial" | "beat" => Some(Self::Pulse),
            "wave" | "waterfall" | "scroll" => Some(Self::Wave),
            "meter" | "vu" | "level" => Some(Self::Meter),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Spectrum => "Spectrum Bars",
            Self::Pulse => "Radial Beat Pulse",
            Self::Wave => "Spectral Wave",
            Self::Meter => "VU Level Meter",
        }
    }
}

/// Curated LED color palettes for audio visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioPalette {
    /// Deep teal -> cyan -> violet -> warm coral.
    Aurora,
    /// Regal navy -> steel ice blue -> crisp cyan -> white-gold peaks.
    Regal,
    /// Electric magenta -> ultraviolet -> laser cyan -> neon mint.
    Neon,
    /// Crimson ember -> molten orange -> solar amber -> white-hot peaks.
    Inferno,
    /// Caller-supplied `fg` gradient over dimmed `bg`.
    Custom,
}

impl AudioPalette {
    pub const ALL: [Self; 5] = [
        Self::Aurora,
        Self::Regal,
        Self::Neon,
        Self::Inferno,
        Self::Custom,
    ];

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "aurora" => Some(Self::Aurora),
            "regal" | "navy" => Some(Self::Regal),
            "neon" | "cyber" => Some(Self::Neon),
            "inferno" | "fire" => Some(Self::Inferno),
            "custom" | "fg" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Aurora => "Aurora",
            Self::Regal => "Regal",
            Self::Neon => "Neon",
            Self::Inferno => "Inferno",
            Self::Custom => "Custom",
        }
    }

    fn sample(self, t: f32, fg: [u8; 3]) -> [u8; 3] {
        let stops: &[[u8; 3]] = match self {
            Self::Aurora => &[[14, 165, 233], [34, 211, 238], [168, 85, 247], [244, 114, 182]],
            Self::Regal => &[[32, 60, 98], [56, 132, 224], [56, 189, 248], [226, 232, 240]],
            Self::Neon => &[[236, 72, 153], [139, 92, 246], [6, 182, 212], [52, 211, 153]],
            Self::Inferno => &[[185, 28, 28], [249, 115, 22], [250, 204, 21], [255, 255, 230]],
            Self::Custom => {
                let mid = fg;
                let peak = [
                    fg[0].saturating_add(70),
                    fg[1].saturating_add(70),
                    fg[2].saturating_add(70),
                ];
                return lerp_rgb(scale_rgb(fg, 0.35), lerp_rgb(mid, peak, t), t);
            }
        };
        sample_gradient(stops, t)
    }
}

/// Instantaneous audio analysis metrics extracted from a PCM window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrameMetrics {
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
    pub signal_active: bool,
    pub beat: bool,
    pub beat_env: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub centroid: f32,
    pub bands: [f32; NUM_BANDS],
}

impl Default for AudioFrameMetrics {
    fn default() -> Self {
        Self {
            rms_dbfs: -90.0,
            peak_dbfs: -90.0,
            signal_active: false,
            beat: false,
            beat_env: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            centroid: 0.25,
            bands: [0.0; NUM_BANDS],
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BandBins {
    lo: usize,
    hi: usize,
    density_weight: f32,
}

#[derive(Debug, Clone, Copy)]
struct KeyGeometry {
    slot: usize,
    band_lo: usize,
    band_hi: usize,
    row_from_bottom: f32,
    x_norm: f32,
    radial_norm: f32,
}

/// Rolling FFT + perceptual band + beat-flux analyzer.
pub struct AudioAnalyzer {
    sample_rate: u32,
    gain: f32,
    ring: [f32; FFT_SIZE],
    write_pos: usize,
    hann: [f32; FFT_SIZE],
    coherent_norm: f32,
    bands: [BandBins; NUM_BANDS],
    auto_ref_db: f32,
    prev_low_db: Option<[f32; BEAT_BANDS]>,
    flux_mean: f32,
    flux_dev: f32,
    since_beat_s: f32,
    beat_env: f32,
    smooth_bands: [f32; NUM_BANDS],
    fft_re: [f32; FFT_SIZE],
    fft_im: [f32; FFT_SIZE],
}

impl AudioAnalyzer {
    pub fn new(sample_rate: u32, gain: f32) -> Self {
        let rate = sample_rate.max(8000);
        let mut hann = [0.0f32; FFT_SIZE];
        let mut w_sum = 0.0f32;
        for (i, w) in hann.iter_mut().enumerate() {
            let val = 0.5 * (1.0 - (2.0 * PI * (i as f32) / (FFT_SIZE as f32)).cos());
            *w = val;
            w_sum += val;
        }
        let coherent_norm = 2.0 / w_sum.max(1.0);
        let bands = compute_band_bins(rate);

        Self {
            sample_rate: rate,
            gain: gain.max(0.05),
            ring: [0.0; FFT_SIZE],
            write_pos: 0,
            hann,
            coherent_norm,
            bands,
            auto_ref_db: REF_FLOOR_DBFS,
            prev_low_db: None,
            flux_mean: 0.0,
            flux_dev: 0.0,
            since_beat_s: 10.0,
            beat_env: 0.0,
            smooth_bands: [0.0; NUM_BANDS],
            fft_re: [0.0; FFT_SIZE],
            fft_im: [0.0; FFT_SIZE],
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.max(0.05);
    }

    /// Push incoming mono normalized PCM samples (`-1.0..=1.0`) into the rolling analysis window.
    pub fn push_samples(&mut self, samples: &[f32]) {
        for &s in samples {
            let clean = if s.is_finite() { s } else { 0.0 };
            self.ring[self.write_pos] = clean;
            self.write_pos = (self.write_pos + 1) % FFT_SIZE;
        }
    }

    /// Analyze the latest `FFT_SIZE` window and advance envelopes by `dt_s`.
    pub fn analyze(&mut self, dt_s: f32) -> AudioFrameMetrics {
        let dt = dt_s.clamp(0.0, 0.25);

        let mut sum_sq = 0.0f32;
        let mut peak_amp = 0.0f32;
        for i in 0..FFT_SIZE {
            let idx = (self.write_pos + i) % FFT_SIZE;
            let raw = (self.ring[idx] * self.gain).clamp(-4.0, 4.0);
            sum_sq += raw * raw;
            peak_amp = peak_amp.max(raw.abs());
            self.fft_re[i] = raw * self.hann[i];
            self.fft_im[i] = 0.0;
        }

        let rms = (sum_sq / (FFT_SIZE as f32)).sqrt();
        let rms_dbfs = amp_to_db(rms);
        let peak_dbfs = amp_to_db(peak_amp);
        let signal_active = rms_dbfs > NOISE_FLOOR_DBFS;

        radix2_fft_in_place(&mut self.fft_re, &mut self.fft_im);

        let mut band_db = [NOISE_FLOOR_DBFS - 12.0; NUM_BANDS];
        let mut max_band_db = REF_FLOOR_DBFS;
        let scale_sq = self.coherent_norm * self.coherent_norm;

        for (b_idx, spec) in self.bands.iter().enumerate() {
            let mut sum_power = 0.0f32;
            for bin in spec.lo..spec.hi {
                let re = self.fft_re[bin];
                let im = self.fft_im[bin];
                sum_power += (re * re + im * im) * scale_sq;
            }
            let power = sum_power * spec.density_weight;
            let db = power_to_db(power);
            band_db[b_idx] = db;
            if db > max_band_db {
                max_band_db = db;
            }
        }

        self.auto_ref_db = (self.auto_ref_db - REF_RELEASE_DB_PER_S * dt)
            .max(max_band_db)
            .max(REF_FLOOR_DBFS);
        let lo_db = self.auto_ref_db - DISPLAY_RANGE_DB;

        let decay = if dt > 0.0 {
            (-dt / LEVEL_RELEASE_TAU_S).exp()
        } else {
            1.0
        };

        let mut weighted_band_sum = 0.0f32;
        let mut band_energy_sum = 0.0f32;
        for (b_idx, &db) in band_db.iter().enumerate() {
            let target = if signal_active {
                ((db - lo_db) / DISPLAY_RANGE_DB).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let val = if target >= self.smooth_bands[b_idx] {
                target
            } else {
                (self.smooth_bands[b_idx] * decay).max(target)
            };
            self.smooth_bands[b_idx] = val;
            weighted_band_sum += val * (b_idx as f32) / ((NUM_BANDS - 1) as f32);
            band_energy_sum += val;
        }

        // Low-band energy flux onset detection on absolute dB clamped above noise floor.
        let mut low_now = [lo_db; BEAT_BANDS];
        let mut flux = 0.0f32;
        for b_idx in 0..BEAT_BANDS {
            let clamped = if signal_active {
                band_db[b_idx].max(lo_db)
            } else {
                lo_db
            };
            low_now[b_idx] = clamped;
            if let Some(prev) = self.prev_low_db {
                let diff = clamped - prev[b_idx];
                if diff > 0.0 {
                    flux += diff;
                }
            }
        }
        self.prev_low_db = Some(low_now);

        self.since_beat_s += dt;
        let beat = signal_active
            && flux >= BEAT_MIN_FLUX_DB
            && flux > self.flux_mean + BEAT_K * self.flux_dev
            && self.since_beat_s >= BEAT_COOLDOWN_S;

        if beat {
            self.since_beat_s = 0.0;
            self.beat_env = 1.0;
        } else if dt > 0.0 {
            self.beat_env *= (-dt / BEAT_ENV_TAU_S).exp();
        }

        let alpha = if dt > 0.0 {
            1.0 - (-dt / BEAT_STATS_TAU_S).exp()
        } else {
            0.0
        };
        self.flux_mean += alpha * (flux - self.flux_mean);
        self.flux_dev += alpha * ((flux - self.flux_mean).abs() - self.flux_dev);

        let bass = average_slice(&self.smooth_bands[0..4]);
        let mid = average_slice(&self.smooth_bands[4..9]);
        let treble = average_slice(&self.smooth_bands[9..NUM_BANDS]);
        let centroid = if band_energy_sum > 1e-4 {
            (weighted_band_sum / band_energy_sum).clamp(0.0, 1.0)
        } else {
            0.25
        };

        AudioFrameMetrics {
            rms_dbfs,
            peak_dbfs,
            signal_active,
            beat,
            beat_env: self.beat_env,
            bass,
            mid,
            treble,
            centroid,
            bands: self.smooth_bands,
        }
    }
}

/// Complete 61-key WIN 60 HE audio-reactive lighting renderer.
pub struct AudioVisualizer {
    analyzer: AudioAnalyzer,
    style: AudioEffectStyle,
    palette: AudioPalette,
    custom_fg: [u8; 3],
    custom_bg: [u8; 3],
    geometry: Vec<KeyGeometry>,
    wave_history: [[f32; 2]; NUM_BANDS],
    wave_shift_accum: f32,
    meter_peak: f32,
    gamma_lut: [u8; 256],
}

impl AudioVisualizer {
    pub fn new(
        sample_rate: u32,
        style: AudioEffectStyle,
        palette: AudioPalette,
        custom_fg: [u8; 3],
        custom_bg: [u8; 3],
        gain: f32,
    ) -> Self {
        let mut gamma_lut = [0u8; 256];
        for (i, slot) in gamma_lut.iter_mut().enumerate() {
            let v = (f32::from(i as u8) / 255.0).powf(1.6) * 255.0;
            *slot = v.round().clamp(0.0, 255.0) as u8;
        }

        Self {
            analyzer: AudioAnalyzer::new(sample_rate, gain),
            style,
            palette,
            custom_fg,
            custom_bg,
            geometry: build_key_geometry(),
            wave_history: [[0.0; 2]; NUM_BANDS],
            wave_shift_accum: 0.0,
            meter_peak: 0.0,
            gamma_lut,
        }
    }

    pub fn push_samples(&mut self, samples: &[f32]) {
        self.analyzer.push_samples(samples);
    }

    /// Update visual rendering style, palette, colors, and input gain live without resetting FFT state.
    pub fn update_render_params(
        &mut self,
        style: AudioEffectStyle,
        palette: AudioPalette,
        custom_fg: [u8; 3],
        custom_bg: [u8; 3],
        gain: f32,
    ) {
        self.style = style;
        self.palette = palette;
        self.custom_fg = custom_fg;
        self.custom_bg = custom_bg;
        self.analyzer.set_gain(gain);
    }

    /// Analyze latest samples and render a full 132-slot WIN 60 HE RGB frame.
    pub fn step(&mut self, dt_s: f32) -> (AudioFrameMetrics, [[u8; 3]; MATRIX_SLOTS]) {
        let metrics = self.analyzer.analyze(dt_s);
        let frame = self.render_frame(&metrics, dt_s);
        (metrics, frame)
    }

    fn render_frame(&mut self, m: &AudioFrameMetrics, dt_s: f32) -> [[u8; 3]; MATRIX_SLOTS] {
        let mut out = [[0u8; 3]; MATRIX_SLOTS];
        let dt = dt_s.clamp(0.0, 0.25);

        match self.style {
            AudioEffectStyle::Spectrum => {
                let bg_pulse = scale_rgb(self.palette.sample(0.15, self.custom_fg), m.beat_env * 0.22);
                let bg_base = max_rgb(self.custom_bg, bg_pulse);
                for g in &self.geometry {
                    let level = average_slice(&m.bands[g.band_lo..g.band_hi]);
                    let bar_rows = level * 5.0;
                    let fill = (bar_rows - g.row_from_bottom).clamp(0.0, 1.0);
                    let lit = if fill > 0.02 {
                        let color = self.palette.sample(g.row_from_bottom / 4.0, self.custom_fg);
                        let boost = 1.0 + m.beat_env * 0.25;
                        lerp_rgb(bg_base, scale_rgb(color, boost), fill)
                    } else {
                        bg_base
                    };
                    out[g.slot] = self.apply_gamma(lit);
                }
            }
            AudioEffectStyle::Pulse => {
                let ring_pos = 1.0 - (m.bass * 0.85 + m.beat_env * 0.35).clamp(0.0, 1.0);
                let shimmer = m.treble * 0.55;
                for g in &self.geometry {
                    let wave = (1.0 - ((g.radial_norm - ring_pos).abs() * 2.1)).clamp(0.0, 1.0);
                    let bass_glow = ((1.0 - g.radial_norm * 0.7) * (m.bass * 0.8 + m.beat_env * 0.5)).clamp(0.0, 1.0);
                    let energy = wave.max(bass_glow).max(shimmer * g.radial_norm);
                    let hue = (g.radial_norm * 0.65 + m.centroid * 0.35).clamp(0.0, 1.0);
                    let fg = self.palette.sample(hue, self.custom_fg);
                    let lit = lerp_rgb(self.custom_bg, fg, energy);
                    out[g.slot] = self.apply_gamma(lit);
                }
            }
            AudioEffectStyle::Wave => {
                self.wave_shift_accum += dt * 16.0;
                if self.wave_shift_accum >= 1.0 || m.beat {
                    self.wave_shift_accum = 0.0;
                    self.wave_history.rotate_right(1);
                }
                let amp = (m.bass * 0.55 + m.mid * 0.35 + m.beat_env * 0.4).clamp(0.0, 1.0);
                self.wave_history[0] = [amp, m.centroid];
                for g in &self.geometry {
                    let col = g.band_lo.min(NUM_BANDS - 1);
                    let [amp, centroid] = self.wave_history[col];
                    let row_weight = 1.0 - ((g.row_from_bottom - 2.0).abs() * 0.22);
                    let fg = self.palette.sample(centroid, self.custom_fg);
                    let lit = lerp_rgb(self.custom_bg, fg, (amp * row_weight).clamp(0.0, 1.0));
                    out[g.slot] = self.apply_gamma(lit);
                }
            }
            AudioEffectStyle::Meter => {
                let overall = (m.bass * 0.45 + m.mid * 0.35 + m.treble * 0.2 + m.beat_env * 0.18).clamp(0.0, 1.0);
                if overall >= self.meter_peak {
                    self.meter_peak = overall;
                } else {
                    self.meter_peak = (self.meter_peak - PEAK_HOLD_DROP_PER_S * dt).max(overall);
                }
                for g in &self.geometry {
                    let fill = ((overall - g.x_norm) * 10.0 + 1.0).clamp(0.0, 1.0);
                    let is_peak = m.signal_active && (g.x_norm - self.meter_peak).abs() < 0.045;
                    let lit = if is_peak {
                        [255, 255, 255]
                    } else if fill > 0.02 {
                        let c = self.palette.sample(g.x_norm, self.custom_fg);
                        lerp_rgb(self.custom_bg, c, fill)
                    } else {
                        self.custom_bg
                    };
                    out[g.slot] = self.apply_gamma(lit);
                }
            }
        }

        out
    }

    fn apply_gamma(&self, rgb: [u8; 3]) -> [u8; 3] {
        [
            self.gamma_lut[usize::from(rgb[0])],
            self.gamma_lut[usize::from(rgb[1])],
            self.gamma_lut[usize::from(rgb[2])],
        ]
    }
}

/// Compact 1-line Unicode bar meter for live CLI feedback.
pub fn render_ascii_meter(m: &AudioFrameMetrics) -> String {
    const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let bars: String = m
        .bands
        .iter()
        .map(|&v| {
            let idx = (v.clamp(0.0, 1.0) * 8.0).round() as usize;
            BLOCKS[idx.min(8)]
        })
        .collect();
    let beat_mark = if m.beat { "BEAT" } else if m.beat_env > 0.35 { " *  " } else { "    " };
    let db = m.rms_dbfs.max(-90.0);
    format!("[{bars}] {db:5.1} dBFS {beat_mark}")
}

/// Deterministic musical groove generator for `--source demo` testing without live playback.
pub struct DemoAudioSynth {
    sample_rate: u32,
    sample_idx: u64,
}

impl DemoAudioSynth {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(8000),
            sample_idx: 0,
        }
    }

    pub fn generate(&mut self, out: &mut [f32]) {
        let sr = self.sample_rate as f32;
        let beat_dur = 0.50f32; // 120 BPM quarter note
        let arp_freqs = [220.0f32, 261.63, 329.63, 440.0, 523.25, 659.25, 880.0, 523.25];

        for s in out.iter_mut() {
            let t = (self.sample_idx as f32) / sr;
            let beat_phase = (t % beat_dur) / beat_dur;
            let step = ((t / 0.25) as usize) % arp_freqs.len();

            // Punchy pitch-dropped kick drum on every beat
            let kick_env = (-beat_phase * 9.0).exp();
            let kick_freq = 48.0 + 95.0 * (-beat_phase * 22.0).exp();
            let kick = (2.0 * PI * kick_freq * (t % beat_dur)).sin() * kick_env * 0.65;

            // Warm saw-like bass + melodic arp lead + crisp offbeat hi-hat bursts
            let arp_f = arp_freqs[step];
            let arp_env = (-((t % 0.25) / 0.25) * 4.5).exp();
            let lead = ((2.0 * PI * arp_f * t).sin() + 0.4 * (2.0 * PI * arp_f * 2.0 * t).sin())
                * arp_env
                * 0.22;

            let hat_phase = ((t + 0.25) % beat_dur) / beat_dur;
            let hat_env = (-hat_phase * 28.0).exp();
            let hat_noise = (((self.sample_idx.wrapping_mul(1664525).wrapping_add(1013904223)) >> 16)
                as u16 as f32
                / 32768.0)
                - 1.0;
            let hat = hat_noise * hat_env * 0.25;

            *s = (kick + lead + hat).clamp(-0.95, 0.95);
            self.sample_idx += 1;
        }
    }
}

fn compute_band_bins(sample_rate: u32) -> [BandBins; NUM_BANDS] {
    let df = (sample_rate as f32) / (FFT_SIZE as f32);
    let max_bin = FFT_SIZE / 2;
    let ratio = F_HI_HZ / F_LO_HZ;
    let mut out = [BandBins {
        lo: 1,
        hi: 2,
        density_weight: 1.0,
    }; NUM_BANDS];

    for (k, slot) in out.iter_mut().enumerate() {
        let f_lo = F_LO_HZ * ratio.powf((k as f32) / (NUM_BANDS as f32));
        let f_hi = F_LO_HZ * ratio.powf(((k + 1) as f32) / (NUM_BANDS as f32));
        let mut lo = (f_lo / df).ceil().clamp(1.0, (max_bin - 1) as f32) as usize;
        let mut hi = (f_hi / df).ceil().clamp(2.0, max_bin as f32) as usize;
        if hi <= lo {
            let center_hz = (f_lo * f_hi).sqrt();
            lo = (center_hz / df).round().clamp(1.0, (max_bin - 1) as f32) as usize;
            hi = (lo + 1).min(max_bin);
        }
        let covered_hz = df * ((hi - lo) as f32);
        let weight = ((f_hi - f_lo) / covered_hz.max(df)).max(0.1);
        *slot = BandBins {
            lo,
            hi,
            density_weight: weight,
        };
    }
    out
}

fn build_key_geometry() -> Vec<KeyGeometry> {
    let mut rows: Vec<u16> = WIN60HE_KEYS.iter().map(|k| k.y).collect();
    rows.sort_unstable();
    rows.dedup();
    // Bottom row (`y = 167`) is row 0; top number row (`y = 15`) is row 4.
    rows.reverse();

    let min_x = f32::from(WIN60HE_KEYS.iter().map(|k| k.x).min().unwrap_or(10));
    let max_right = f32::from(
        WIN60HE_KEYS
            .iter()
            .map(|k| k.x + k.width)
            .max()
            .unwrap_or(KEYBOARD_CANVAS_WIDTH),
    );
    let span_x = (max_right - min_x).max(1.0);
    let cx = f32::from(KEYBOARD_CANVAS_WIDTH) * 0.5;
    let cy = f32::from(KEYBOARD_CANVAS_HEIGHT) * 0.5;
    let max_r = (cx * cx + cy * cy).sqrt().max(1.0);

    WIN60HE_KEYS
        .iter()
        .map(|k| {
            let x0 = ((f32::from(k.x) - min_x) / span_x).clamp(0.0, 0.999);
            let x1 = ((f32::from(k.x + k.width) - min_x) / span_x).clamp(x0 + 0.01, 1.0);
            let b0 = ((x0 * (NUM_BANDS as f32)).floor() as usize).min(NUM_BANDS - 1);
            let b1 = ((x1 * (NUM_BANDS as f32)).ceil() as usize)
                .clamp(b0 + 1, NUM_BANDS);
            let row_idx = rows.iter().position(|&ry| ry == k.y).unwrap_or(0) as f32;
            let kx = f32::from(k.x) + f32::from(k.width) * 0.5;
            let ky = f32::from(k.y) + f32::from(k.height) * 0.5;
            let dx = kx - cx;
            let dy = ky - cy;
            let radial = ((dx * dx + dy * dy).sqrt() / max_r).clamp(0.0, 1.0);
            KeyGeometry {
                slot: usize::from(k.index),
                band_lo: b0,
                band_hi: b1,
                row_from_bottom: row_idx,
                x_norm: ((kx - min_x) / span_x).clamp(0.0, 1.0),
                radial_norm: radial,
            }
        })
        .collect()
}

/// Iterative radix-2 Cooley-Tukey FFT in-place for `N = FFT_SIZE`.
fn radix2_fft_in_place(re: &mut [f32; FFT_SIZE], im: &mut [f32; FFT_SIZE]) {
    let n = FFT_SIZE;
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while (j & bit) != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let half = len / 2;
        let angle = -2.0 * PI / (len as f32);
        let wlen_re = angle.cos();
        let wlen_im = angle.sin();
        for start in (0..n).step_by(len) {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;
            for k in 0..half {
                let u_idx = start + k;
                let v_idx = u_idx + half;
                let v_re = re[v_idx] * w_re - im[v_idx] * w_im;
                let v_im = re[v_idx] * w_im + im[v_idx] * w_re;
                let u_re = re[u_idx];
                let u_im = im[u_idx];
                re[u_idx] = u_re + v_re;
                im[u_idx] = u_im + v_im;
                re[v_idx] = u_re - v_re;
                im[v_idx] = u_im - v_im;
                let next_re = w_re * wlen_re - w_im * wlen_im;
                w_im = w_re * wlen_im + w_im * wlen_re;
                w_re = next_re;
            }
        }
        len <<= 1;
    }
}

fn amp_to_db(amp: f32) -> f32 {
    if !amp.is_finite() || amp <= 1e-6 {
        -120.0
    } else {
        20.0 * amp.log10()
    }
}

fn power_to_db(power: f32) -> f32 {
    if !power.is_finite() || power <= EPS {
        10.0 * EPS.log10()
    } else {
        10.0 * power.log10()
    }
}

fn average_slice(vals: &[f32]) -> f32 {
    if vals.is_empty() {
        0.0
    } else {
        vals.iter().sum::<f32>() / (vals.len() as f32)
    }
}

fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (f32::from(a[0]) + (f32::from(b[0]) - f32::from(a[0])) * t).round() as u8,
        (f32::from(a[1]) + (f32::from(b[1]) - f32::from(a[1])) * t).round() as u8,
        (f32::from(a[2]) + (f32::from(b[2]) - f32::from(a[2])) * t).round() as u8,
    ]
}

fn scale_rgb(c: [u8; 3], factor: f32) -> [u8; 3] {
    let f = factor.clamp(0.0, 1.0);
    [
        (f32::from(c[0]) * f).round() as u8,
        (f32::from(c[1]) * f).round() as u8,
        (f32::from(c[2]) * f).round() as u8,
    ]
}

fn max_rgb(a: [u8; 3], b: [u8; 3]) -> [u8; 3] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]
}

fn sample_gradient(stops: &[[u8; 3]], t: f32) -> [u8; 3] {
    if stops.len() == 1 {
        return stops[0];
    }
    let pos = t.clamp(0.0, 1.0) * ((stops.len() - 1) as f32);
    let idx = (pos.floor() as usize).min(stops.len() - 2);
    lerp_rgb(stops[idx], stops[idx + 1], pos - (idx as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_keeps_bands_zero_and_inactive() {
        let mut analyzer = AudioAnalyzer::new(DEFAULT_SAMPLE_RATE, 1.0);
        let zeros = vec![0.0f32; FFT_SIZE];
        analyzer.push_samples(&zeros);
        let m = analyzer.analyze(0.02);
        assert!(!m.signal_active);
        assert!(!m.beat);
        assert!(m.bands.iter().all(|&b| b == 0.0));
    }

    #[test]
    fn fft_places_low_and_high_sine_waves_in_expected_bands() {
        let sr = DEFAULT_SAMPLE_RATE as f32;
        let low_samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 * (2.0 * PI * 120.0 * (i as f32) / sr).sin())
            .collect();
        let mut low_a = AudioAnalyzer::new(DEFAULT_SAMPLE_RATE, 1.0);
        low_a.push_samples(&low_samples);
        let low_m = low_a.analyze(0.02);
        assert!(low_m.signal_active);
        let low_peak = low_m
            .bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert!(low_peak <= 3, "expected 120 Hz in low bands 0..=3, got {low_peak}");

        let high_samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 * (2.0 * PI * 5500.0 * (i as f32) / sr).sin())
            .collect();
        let mut high_a = AudioAnalyzer::new(DEFAULT_SAMPLE_RATE, 1.0);
        high_a.push_samples(&high_samples);
        let high_m = high_a.analyze(0.02);
        let high_peak = high_m
            .bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert!(high_peak >= 10, "expected 5.5 kHz in treble bands >=10, got {high_peak}");
    }

    #[test]
    fn all_visualizer_styles_render_strictly_physical_slots() {
        let mut synth = DemoAudioSynth::new(DEFAULT_SAMPLE_RATE);
        let mut pcm = [0.0f32; 480];
        synth.generate(&mut pcm);

        let physical_slots: std::collections::BTreeSet<usize> =
            WIN60HE_KEYS.iter().map(|k| usize::from(k.index)).collect();

        for style in [
            AudioEffectStyle::Spectrum,
            AudioEffectStyle::Pulse,
            AudioEffectStyle::Wave,
            AudioEffectStyle::Meter,
        ] {
            let mut vis = AudioVisualizer::new(
                DEFAULT_SAMPLE_RATE,
                style,
                AudioPalette::Aurora,
                [56, 189, 248],
                [0, 0, 0],
                1.0,
            );
            vis.push_samples(&pcm);
            let (metrics, grid) = vis.step(0.02);
            assert!(metrics.signal_active);
            for (slot, rgb) in grid.iter().enumerate() {
                if !physical_slots.contains(&slot) {
                    assert_eq!(*rgb, [0, 0, 0], "non-key slot {slot} should remain black");
                }
            }
            assert!(
                physical_slots.iter().any(|&s| grid[s] != [0, 0, 0]),
                "style {style:?} produced an all-black frame on demo audio"
            );
        }
    }
}
