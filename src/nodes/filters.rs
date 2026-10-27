//! One-pole, slew limiter, envelope follower, biquad, prewarp, multimode one-pole, SVF, SVF shelf.

use crate::node::{expect_string, BlockContext, Node, NodeInfo};
use crate::resources::SharedResourceMap;
use crate::value::Value;
use crate::Error;

/// `pole`: inputs [coefficient, signal].
#[derive(Default)]
pub struct OnePoleNode {
    z: f32,
}

impl OnePoleNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for OnePoleNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let p = ctx.input(0);
        let x = ctx.input(1);
        for i in 0..ctx.num_samples {
            self.z = x[i] + p[i] * self.z;
            out[i] = self.z;
        }
    }
}

/// `slew`: inputs [max step, signal]. Follows the signal but moves at most
/// `max step` per sample, so a jump becomes a ramp. Starts at the signal's
/// first value rather than ramping up from zero.
#[derive(Default)]
pub struct SlewNode {
    z: Option<f32>,
}

impl SlewNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for SlewNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let step = ctx.input(0);
        let x = ctx.input(1);
        for i in 0..ctx.num_samples {
            let z = self.z.unwrap_or(x[i]);
            let max = step[i].abs();
            let y = z + (x[i] - z).clamp(-max, max);
            self.z = Some(y);
            out[i] = y;
        }
    }

    fn reset(&mut self) {
        self.z = None;
    }
}

/// `env`: inputs [attack pole, release pole, signal].
#[derive(Default)]
pub struct EnvelopeNode {
    z: f32,
}

impl EnvelopeNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for EnvelopeNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 3 {
            out.fill(0.0);
            return;
        }
        let ap = ctx.input(0);
        let rp = ctx.input(1);
        let x = ctx.input(2);
        for i in 0..ctx.num_samples {
            let vn = x[i].abs();
            self.z = if vn > self.z {
                ap[i] * (self.z - vn) + vn
            } else {
                rp[i] * (self.z - vn) + vn
            };
            out[i] = self.z;
        }
    }
}

/// `biquad`: inputs [b0, b1, b2, a1, a2, signal], transposed direct form II.
#[derive(Default)]
pub struct BiquadNode {
    z1: f32,
    z2: f32,
}

impl BiquadNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for BiquadNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 6 {
            out.fill(0.0);
            return;
        }
        let (b0, b1, b2) = (ctx.input(0), ctx.input(1), ctx.input(2));
        let (a1, a2) = (ctx.input(3), ctx.input(4));
        let x = ctx.input(5);
        for i in 0..ctx.num_samples {
            let xn = x[i];
            let y = b0[i] * xn + self.z1;
            self.z1 = b1[i] * xn - a1[i] * y + self.z2;
            self.z2 = b2[i] * xn - a2[i] * y;
            out[i] = y;
        }
    }
}

/// `prewarp`: bilinear-transform cutoff prewarping, tan(pi * fc / sr).
pub struct PrewarpNode {
    period: f64,
}

impl PrewarpNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            period: 1.0 / info.sample_rate,
        }
    }
}

impl Node for PrewarpNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 1 {
            out.fill(0.0);
            return;
        }
        let fc = ctx.input(0);
        for i in 0..ctx.num_samples {
            let wd = std::f64::consts::TAU * fc[i] as f64;
            out[i] = (wd * self.period / 2.0).tan() as f32;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OnePoleMode {
    Low,
    High,
    All,
}

/// `mm1p`: multimode one-pole; inputs [prewarped g, signal].
pub struct MultiMode1pNode {
    mode: OnePoleMode,
    z: f64,
}

impl MultiMode1pNode {
    pub fn new(_: NodeInfo) -> Self {
        Self {
            mode: OnePoleMode::Low,
            z: 0.0,
        }
    }
}

impl Node for MultiMode1pNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "mode" {
            match expect_string(val)? {
                "lowpass" => self.mode = OnePoleMode::Low,
                "highpass" => self.mode = OnePoleMode::High,
                "allpass" => self.mode = OnePoleMode::All,
                _ => {}
            }
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let g_in = ctx.input(0);
        let x = ctx.input(1);
        for i in 0..ctx.num_samples {
            let g = (g_in[i] as f64).clamp(0.0, 0.9999);
            let xn = x[i];
            let big_g = g / (1.0 + g);
            let v = (xn as f64 - self.z) * big_g;
            let lp = v + self.z;
            self.z = lp + v;
            out[i] = match self.mode {
                OnePoleMode::Low => lp as f32,
                OnePoleMode::High => xn - lp as f32,
                OnePoleMode::All => (lp + lp - xn as f64) as f32,
            };
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SvfMode {
    Low,
    Band,
    High,
    Notch,
    All,
}

/// `svf`: state variable filter; inputs [cutoff Hz, Q, signal].
pub struct SvfNode {
    mode: SvfMode,
    sr: f64,
    g: f64,
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    ic1eq: f64,
    ic2eq: f64,
}

impl SvfNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            mode: SvfMode::Low,
            sr: info.sample_rate,
            g: 0.0,
            k: 0.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            ic1eq: 0.0,
            ic2eq: 0.0,
        }
    }

    #[inline]
    fn update_coeffs(&mut self, fc: f64, q: f64) {
        self.g = (std::f64::consts::PI * fc.clamp(20.0, self.sr / 2.0001) / self.sr).tan();
        self.k = 1.0 / q.clamp(0.25, 20.0);
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
    }

    #[inline]
    fn tick(&mut self, v0: f64) -> f32 {
        let v3 = v0 - self.ic2eq;
        let v1 = self.ic1eq * self.a1 + v3 * self.a2;
        let v2 = self.ic2eq + self.ic1eq * self.a2 + v3 * self.a3;
        self.ic1eq = v1 * 2.0 - self.ic1eq;
        self.ic2eq = v2 * 2.0 - self.ic2eq;
        let y = match self.mode {
            SvfMode::Low => v2,
            SvfMode::Band => v1,
            SvfMode::High => v0 - self.k * v1 - v2,
            SvfMode::Notch => v0 - self.k * v1,
            SvfMode::All => v0 - 2.0 * self.k * v1,
        };
        y as f32
    }
}

impl Node for SvfNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "mode" {
            match expect_string(val)? {
                "lowpass" => self.mode = SvfMode::Low,
                "bandpass" => self.mode = SvfMode::Band,
                "highpass" => self.mode = SvfMode::High,
                "notch" => self.mode = SvfMode::Notch,
                "allpass" => self.mode = SvfMode::All,
                _ => {}
            }
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 3 {
            out.fill(0.0);
            return;
        }
        let fc = ctx.input(0);
        let q = ctx.input(1);
        let x = ctx.input(2);
        for i in 0..ctx.num_samples {
            self.update_coeffs(fc[i] as f64, q[i] as f64);
            out[i] = self.tick(x[i] as f64);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShelfMode {
    Lowshelf,
    Highshelf,
    Bell,
}

/// `svfshelf`: inputs [cutoff Hz, Q, gain dB, signal].
pub struct SvfShelfNode {
    mode: ShelfMode,
    sr: f64,
    a: f64,
    g: f64,
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    ic1eq: f64,
    ic2eq: f64,
}

impl SvfShelfNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            mode: ShelfMode::Lowshelf,
            sr: info.sample_rate,
            a: 0.0,
            g: 0.0,
            k: 0.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            ic1eq: 0.0,
            ic2eq: 0.0,
        }
    }

    #[inline]
    fn update_coeffs(&mut self, fc: f64, q: f64, gain_db: f64) {
        self.a = 10f64.powf(gain_db / 40.0);
        self.g = (std::f64::consts::PI * fc.clamp(20.0, self.sr / 2.0001) / self.sr).tan();
        self.k = 1.0 / q.clamp(0.25, 20.0);
        match self.mode {
            ShelfMode::Lowshelf => self.g /= self.a,
            ShelfMode::Highshelf => self.g *= self.a,
            ShelfMode::Bell => self.k /= self.a,
        }
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
    }

    #[inline]
    fn tick(&mut self, v0: f64) -> f32 {
        let v3 = v0 - self.ic2eq;
        let v1 = self.ic1eq * self.a1 + v3 * self.a2;
        let v2 = self.ic2eq + self.ic1eq * self.a2 + v3 * self.a3;
        self.ic1eq = v1 * 2.0 - self.ic1eq;
        self.ic2eq = v2 * 2.0 - self.ic2eq;
        let a = self.a;
        let k = self.k;
        let y = match self.mode {
            ShelfMode::Bell => v0 + k * (a * a - 1.0) * v1,
            ShelfMode::Lowshelf => v0 + k * (a - 1.0) * v1 + (a * a - 1.0) * v2,
            ShelfMode::Highshelf => a * a * v0 + k * (1.0 - a) * a * v1 + (1.0 - a * a) * v2,
        };
        y as f32
    }
}

impl Node for SvfShelfNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "mode" {
            match expect_string(val)? {
                "lowshelf" => self.mode = ShelfMode::Lowshelf,
                "highshelf" => self.mode = ShelfMode::Highshelf,
                "bell" | "peak" => self.mode = ShelfMode::Bell,
                _ => {}
            }
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 4 {
            out.fill(0.0);
            return;
        }
        let fc = ctx.input(0);
        let q = ctx.input(1);
        let gain = ctx.input(2);
        let x = ctx.input(3);
        for i in 0..ctx.num_samples {
            self.update_coeffs(fc[i] as f64, q[i] as f64, gain[i] as f64);
            out[i] = self.tick(x[i] as f64);
        }
    }
}
