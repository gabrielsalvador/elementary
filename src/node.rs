//! The node interface plus the small DSP helpers shared by many builtins.

use std::collections::BTreeMap;

use crate::resources::SharedResourceMap;
use crate::value::Value;
use crate::Error;

pub type NodeId = i32;
pub type Props = BTreeMap<String, Value>;

/// Static per-runtime configuration handed to node constructors.
#[derive(Clone, Copy, Debug)]
pub struct NodeInfo {
    pub sample_rate: f64,
    pub block_size: usize,
}

/// Per-block processing context.
///
/// Input and output buffers live in a flat arena owned by the render sequence.
/// `input(i)`/`output(i)` hand out slices into that arena. Output slots are
/// distinct from input slots for any given op, and every output slot belongs
/// to exactly one op, so handing out `&mut` from `&self` is sound as long as a
/// node does not hold two live references to the same output index at once.
pub struct BlockContext<'a> {
    arena: *mut f32,
    stride: usize,
    host_inputs: &'a [*const f32],
    host_offset: usize,
    /// `None` means "read the host input channels" (leaf nodes).
    inputs: Option<&'a [usize]>,
    outputs: &'a [usize],
    pub num_samples: usize,
    /// Whether the root this op belongs to was active when the sequence was built.
    pub active: bool,
}

impl<'a> BlockContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        arena: *mut f32,
        stride: usize,
        host_inputs: &'a [*const f32],
        host_offset: usize,
        inputs: Option<&'a [usize]>,
        outputs: &'a [usize],
        num_samples: usize,
        active: bool,
    ) -> Self {
        Self {
            arena,
            stride,
            host_inputs,
            host_offset,
            inputs,
            outputs,
            num_samples,
            active,
        }
    }

    pub fn num_inputs(&self) -> usize {
        match self.inputs {
            Some(slots) => slots.len(),
            None => self.host_inputs.len(),
        }
    }

    pub fn num_outputs(&self) -> usize {
        self.outputs.len()
    }

    pub fn input(&self, i: usize) -> &'a [f32] {
        match self.inputs {
            Some(slots) => {
                let slot = slots[i];
                // SAFETY: slot indices are allocated within the arena by the
                // render sequence, and arena size >= (slots * stride).
                unsafe {
                    std::slice::from_raw_parts(self.arena.add(slot * self.stride), self.num_samples)
                }
            }
            None => {
                // SAFETY: host pointers were provided by the caller of
                // `Runtime::process` and are valid for `host_offset + num_samples`.
                unsafe {
                    std::slice::from_raw_parts(
                        self.host_inputs[i].add(self.host_offset),
                        self.num_samples,
                    )
                }
            }
        }
    }

    pub fn output(&self, i: usize) -> &'a mut [f32] {
        let slot = self.outputs[i];
        // SAFETY: see the type-level docs. Each output slot is owned by exactly
        // one op, and ops run strictly sequentially.
        unsafe {
            std::slice::from_raw_parts_mut(self.arena.add(slot * self.stride), self.num_samples)
        }
    }

    /// Write silence to every output channel.
    pub fn zero_outputs(&self) {
        for i in 0..self.num_outputs() {
            self.output(i).fill(0.0);
        }
    }
}

/// A single processing operation in the audio graph.
pub trait Node: Send {
    /// Called on property changes. The runtime stores the property in the
    /// node's property map only when this returns `Ok`.
    fn set_property(
        &mut self,
        _key: &str,
        _val: &Value,
        _resources: &mut SharedResourceMap,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>);

    /// Relay any pending analysis events.
    fn process_events(&mut self, _props: &Props, _emit: &mut dyn FnMut(&str, Value)) {}

    /// Reset internal state (delay lines, sample readers, ...).
    fn reset(&mut self) {}

    fn as_root(&self) -> Option<&crate::nodes::core::RootNode> {
        None
    }

    fn as_tap_out(&mut self) -> Option<&mut crate::nodes::feedback::TapOutNode> {
        None
    }

    fn as_meter(&mut self) -> Option<&mut crate::nodes::analyzers::MeterNode> {
        None
    }
}

/// Look up the `name` property, used as the `source` of analysis events.
pub fn source_name(props: &Props) -> Value {
    props.get("name").cloned().unwrap_or(Value::Undefined)
}

pub fn expect_number(val: &Value) -> Result<f64, Error> {
    val.as_number().ok_or(Error::InvalidPropertyType)
}

pub fn expect_bool(val: &Value) -> Result<bool, Error> {
    val.as_bool().ok_or(Error::InvalidPropertyType)
}

pub fn expect_string(val: &Value) -> Result<&str, Error> {
    val.as_str().ok_or(Error::InvalidPropertyType)
}

//==============================================================================
// Helpers

/// Returns +1 on a rising input, -1 on a falling input, 0 when steady.
#[derive(Clone, Copy, Debug, Default)]
pub struct Change {
    last: f32,
}

impl Change {
    pub fn tick(&mut self, x: f32) -> f32 {
        let dt = x - self.last;
        self.last = x;
        if dt > 0.0 {
            1.0
        } else if dt < 0.0 {
            -1.0
        } else {
            0.0
        }
    }
}

pub fn fp_equal(x: f32, y: f32) -> bool {
    (x - y).abs() <= 1e-6
}

pub fn lerp(alpha: f32, x: f32, y: f32) -> f32 {
    x + alpha * (y - x)
}

fn ms_to_step(sample_rate: f64, ms: f64) -> f32 {
    if ms > 1e-6 {
        (1.0 / (sample_rate * ms / 1000.0)) as f32
    } else {
        1.0
    }
}

/// Linear gain ramp between 0 and 1 with independent fade-in / fade-out times.
#[derive(Clone, Copy, Debug)]
pub struct GainFade {
    current: f32,
    target: f32,
    step: f32,
    in_step: f32,
    out_step: f32,
}

impl GainFade {
    pub fn new(sample_rate: f64, fade_in_ms: f64, fade_out_ms: f64, current: f32, target: f32) -> Self {
        let mut fade = Self {
            current,
            target,
            step: 0.0,
            in_step: 0.0,
            out_step: 0.0,
        };
        fade.set_fade_in_ms(sample_rate, fade_in_ms);
        fade.set_fade_out_ms(sample_rate, fade_out_ms);
        fade
    }

    pub fn tick(&mut self, x: f32) -> f32 {
        if fp_equal(self.current, self.target) {
            return self.target * x;
        }
        let y = x * self.current;
        self.current = (self.current + self.step).clamp(0.0, 1.0);
        y
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let n = input.len().min(output.len());
        if self.current == self.target {
            for i in 0..n {
                output[i] = input[i] * self.target;
            }
            return;
        }
        let step = self.step;
        for i in 0..n {
            output[i] = input[i] * (self.current + step * i as f32).clamp(0.0, 1.0);
        }
        self.current = (self.current + step * n as f32).clamp(0.0, 1.0);
    }

    pub fn set_fade_in_ms(&mut self, sample_rate: f64, ms: f64) {
        self.in_step = ms_to_step(sample_rate, ms);
        self.update_step();
    }

    pub fn set_fade_out_ms(&mut self, sample_rate: f64, ms: f64) {
        self.out_step = -ms_to_step(sample_rate, ms);
        self.update_step();
    }

    pub fn fade_in(&mut self) {
        self.target = 1.0;
        self.update_step();
    }

    pub fn fade_out(&mut self) {
        self.target = 0.0;
        self.update_step();
    }

    pub fn on(&self) -> bool {
        self.target > 0.5
    }

    pub fn settled(&self) -> bool {
        fp_equal(self.target, self.current)
    }

    pub fn reset(&mut self) {
        self.current = 0.0;
        self.target = 0.0;
    }

    fn update_step(&mut self) {
        self.step = if self.current > self.target {
            self.out_step
        } else {
            self.in_step
        };
    }
}

/// Smallest power of two >= n (n >= 1).
pub fn bitceil(n: usize) -> usize {
    n.max(1).next_power_of_two()
}

/// Multi-channel ring buffer used by the analysis nodes. Capacity must be a
/// power of two. Writing past the free space advances the read position.
pub struct MultiChannelRingBuffer {
    buffers: Vec<Vec<f32>>,
    capacity: usize,
    mask: usize,
    read_pos: usize,
    write_pos: usize,
}

impl MultiChannelRingBuffer {
    pub fn new(num_channels: usize, capacity: usize) -> Self {
        let capacity = bitceil(capacity);
        Self {
            buffers: (0..num_channels).map(|_| vec![0.0; capacity]).collect(),
            capacity,
            mask: capacity - 1,
            read_pos: 0,
            write_pos: 0,
        }
    }

    pub fn num_channels(&self) -> usize {
        self.buffers.len()
    }

    /// Write `num_samples` from each channel of `data`, starting at `read_offset`.
    pub fn write(&mut self, data: &[&[f32]], num_samples: usize, read_offset: usize) {
        let w = self.write_pos;
        let r = self.read_pos;
        let should_move_read = num_samples >= self.num_free_slots(r, w);
        let desired_write = (w + num_samples) & self.mask;
        let desired_read = if should_move_read {
            (desired_write + 1) & self.mask
        } else {
            r
        };
        let n = self.buffers.len().min(data.len());
        for i in 0..n {
            let src = &data[i][read_offset..read_offset + num_samples];
            let buf = &mut self.buffers[i];
            if w + num_samples >= self.capacity {
                let s1 = self.capacity - w;
                buf[w..].copy_from_slice(&src[..s1]);
                buf[..num_samples - s1].copy_from_slice(&src[s1..]);
            } else {
                buf[w..w + num_samples].copy_from_slice(src);
            }
        }
        self.write_pos = desired_write;
        self.read_pos = desired_read;
    }

    /// Read `num_samples` into each channel of `dest`. Returns false (and reads
    /// nothing) if fewer samples are available.
    pub fn read(&mut self, dest: &mut [&mut [f32]], num_samples: usize) -> bool {
        let r = self.read_pos;
        let w = self.write_pos;
        if self.num_full_slots(r, w) < num_samples {
            return false;
        }
        let n = self.buffers.len().min(dest.len());
        for i in 0..n {
            let buf = &self.buffers[i];
            let out = &mut dest[i][..num_samples];
            if r + num_samples >= self.capacity {
                let s1 = self.capacity - r;
                out[..s1].copy_from_slice(&buf[r..]);
                out[s1..].copy_from_slice(&buf[..num_samples - s1]);
            } else {
                out.copy_from_slice(&buf[r..r + num_samples]);
            }
        }
        self.read_pos = (r + num_samples) & self.mask;
        true
    }

    pub fn size(&self) -> usize {
        self.num_full_slots(self.read_pos, self.write_pos)
    }

    fn num_full_slots(&self, r: usize, w: usize) -> usize {
        if w > r {
            w - r
        } else {
            (self.capacity - (r - w)) & self.mask
        }
    }

    fn num_free_slots(&self, r: usize, w: usize) -> usize {
        if r > w {
            r - w
        } else {
            self.capacity - (w - r)
        }
    }
}
