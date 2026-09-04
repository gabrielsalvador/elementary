//! PolyBLEP band-limited saw, square and triangle oscillators.

use crate::node::{BlockContext, Node, NodeInfo};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BlepMode {
    Saw,
    Square,
    Triangle,
}

pub struct PolyBlepNode {
    mode: BlepMode,
    phase: f32,
    acc: f32,
    sr: f32,
}

impl PolyBlepNode {
    pub fn new(info: NodeInfo, mode: BlepMode) -> Self {
        Self {
            mode,
            phase: 0.0,
            acc: 0.0,
            sr: info.sample_rate as f32,
        }
    }

    #[inline]
    fn blep(phase: f32, increment: f32) -> f32 {
        if phase < increment {
            let p = phase / increment;
            (2.0 - p) * p - 1.0
        } else if phase > 1.0 - increment {
            let p = (phase - 1.0) / increment;
            (p + 2.0) * p + 1.0
        } else {
            0.0
        }
    }

    #[inline]
    fn tick(&mut self, phase: f32, increment: f32) -> f32 {
        match self.mode {
            BlepMode::Saw => 2.0 * phase - 1.0 - Self::blep(phase, increment),
            BlepMode::Square | BlepMode::Triangle => {
                let naive = if phase < 0.5 { 1.0 } else { -1.0 };
                let half_phase = (phase + 0.5) % 1.0;
                let square = naive + Self::blep(phase, increment) - Self::blep(half_phase, increment);
                if self.mode == BlepMode::Square {
                    square
                } else {
                    self.acc += 4.0 * increment * square;
                    self.acc
                }
            }
        }
    }
}

impl Node for PolyBlepNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 1 {
            out.fill(0.0);
            return;
        }
        let freq = ctx.input(0);
        for i in 0..ctx.num_samples {
            let increment = freq[i] / self.sr;
            let y = self.tick(self.phase, increment);
            self.phase += increment;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            out[i] = y;
        }
    }
}
