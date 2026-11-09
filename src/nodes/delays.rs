//! Single-sample, fixed sample-count, and variable (interpolating) delays.

use crate::node::{bitceil, expect_number, BlockContext, Node, NodeInfo};
use crate::resources::SharedResourceMap;
use crate::value::Value;
use crate::Error;

/// `z`: one sample of delay.
#[derive(Default)]
pub struct SingleSampleDelayNode {
    z: f32,
}

impl SingleSampleDelayNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for SingleSampleDelayNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 1 {
            out.fill(0.0);
            return;
        }
        let input = ctx.input(0);
        for i in 0..ctx.num_samples {
            let x = input[i];
            out[i] = self.z;
            self.z = x;
        }
    }
}

/// `delay`: inputs are [delay length in samples, feedback, signal].
pub struct VariableDelayNode {
    pending: Option<Vec<f32>>,
    buffer: Vec<f32>,
    write_index: usize,
}

impl VariableDelayNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            pending: None,
            buffer: vec![0.0; info.block_size],
            write_index: 0,
        }
    }
}

impl Node for VariableDelayNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "size" {
            let size = expect_number(val)?.max(0.0) as usize;
            self.pending = Some(vec![0.0; size]);
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if let Some(buf) = self.pending.take() {
            self.buffer = buf;
            self.write_index = 0;
        }
        if ctx.num_inputs() < 3 {
            out.fill(0.0);
            return;
        }
        let delay_len = ctx.input(0);
        let feedback = ctx.input(1);
        let signal = ctx.input(2);
        let size = self.buffer.len();
        if size == 0 {
            out.copy_from_slice(signal);
            return;
        }
        let data = &mut self.buffer;
        for i in 0..ctx.num_samples {
            let offset = delay_len[i].clamp(0.0, size as f32);
            if offset <= f32::EPSILON {
                let x = signal[i];
                data[self.write_index] = x;
                out[i] = x;
                self.write_index += 1;
                if self.write_index >= size {
                    self.write_index -= size;
                }
                continue;
            }
            let read_frac = (size + self.write_index) as f32 - offset;
            let read_left = read_frac as usize;
            let read_right = read_left + 1;
            let frac = read_frac - read_frac.floor();
            let left = data[read_left % size];
            let right = data[read_right % size];
            let y = left + frac * (right - left);
            let fb = feedback[i].clamp(-1.0, 1.0);
            data[self.write_index] = signal[i] + fb * y;
            out[i] = y;
            self.write_index += 1;
            if self.write_index >= size {
                self.write_index -= size;
            }
        }
    }
}

/// `sdelay`: fixed delay of `size` samples, block-oriented.
pub struct SampleDelayNode {
    pending: Option<(Vec<f32>, usize)>,
    buffer: Vec<f32>,
    length: usize,
    write_index: usize,
    block_size: usize,
}

impl SampleDelayNode {
    pub fn new(info: NodeInfo) -> Self {
        let mut node = Self {
            pending: None,
            buffer: Vec::new(),
            length: 0,
            write_index: 0,
            block_size: info.block_size,
        };
        node.set_size(info.block_size);
        node
    }

    fn set_size(&mut self, len: usize) {
        let size = bitceil(len + self.block_size);
        self.pending = Some((vec![0.0; size], len));
    }
}

impl Node for SampleDelayNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "size" {
            let len = expect_number(val)?.max(0.0) as usize;
            self.set_size(len);
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if let Some((buf, len)) = self.pending.take() {
            self.buffer = buf;
            self.length = len;
            self.write_index = 0;
        }
        let size = self.buffer.len();
        if ctx.num_inputs() < 1 || size == 0 {
            out.fill(0.0);
            return;
        }
        let mask = size - 1;
        let input = ctx.input(0);
        let read_start = self.write_index as i64 - self.length as i64;
        for &x in &input[..ctx.num_samples] {
            self.buffer[self.write_index] = x;
            self.write_index = (self.write_index + 1) & mask;
        }
        for (i, o) in out[..ctx.num_samples].iter_mut().enumerate() {
            let idx = (size as i64 + read_start + i as i64) as usize & mask;
            *o = self.buffer[idx];
        }
    }
}
