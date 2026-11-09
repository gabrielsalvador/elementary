//! Meter, snapshot, scope, capture and mc.capture: nodes that pass audio
//! through and relay analysis data as events.

use crate::node::{
    bitceil, expect_number, expect_string, source_name, BlockContext, Change, MultiChannelRingBuffer,
    Node, NodeInfo, Props,
};
use crate::resources::SharedResourceMap;
use crate::value::Value;
use crate::Error;

//==============================================================================

/// One block's worth of `meter` analysis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterReadout {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

/// `meter`: min / max / rms over the blocks since its readout was last
/// taken (rms of the loudest block).
#[derive(Default)]
pub struct MeterNode {
    readout: Option<MeterReadout>,
}

impl MeterNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }

    /// Take the readout covering every block since the last take, if any
    /// ran. Taking it here means `process_events` won't also emit it.
    pub fn take_readout(&mut self) -> Option<MeterReadout> {
        self.readout.take()
    }
}

impl Node for MeterNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 1 {
            out.fill(0.0);
            return;
        }
        let input = ctx.input(0);
        out.copy_from_slice(input);
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut sum_squares = 0.0f32;
        for &s in input {
            min = min.min(s);
            max = max.max(s);
            sum_squares += s * s;
        }
        if input.is_empty() {
            min = 0.0;
            max = 0.0;
        }
        let rms = (sum_squares / input.len().max(1) as f32).sqrt();
        // Fold into an untaken readout so a short peak in an earlier block
        // isn't overwritten; rms keeps the loudest block's.
        self.readout = Some(match self.readout {
            Some(r) => MeterReadout {
                min: r.min.min(min),
                max: r.max.max(max),
                rms: r.rms.max(rms),
            },
            None => MeterReadout { min, max, rms },
        });
    }

    fn process_events(&mut self, props: &Props, emit: &mut dyn FnMut(&str, Value)) {
        if let Some(ro) = self.readout.take() {
            emit(
                "meter",
                Value::object([
                    ("min", ro.min.into()),
                    ("max", ro.max.into()),
                    ("rms", ro.rms.into()),
                    ("source", source_name(props)),
                ]),
            );
        }
    }

    fn as_meter(&mut self) -> Option<&mut MeterNode> {
        Some(self)
    }
}

//==============================================================================

/// `snapshot`: inputs [trigger, signal]; reports the signal value on each rising trigger.
#[derive(Default)]
pub struct SnapshotNode {
    readout: Option<f32>,
    z: f32,
}

impl SnapshotNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for SnapshotNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let trig = ctx.input(0);
        let x = ctx.input(1);
        for i in 0..ctx.num_samples {
            let l = trig[i];
            if self.z.abs() <= f32::EPSILON && l > f32::EPSILON {
                self.readout = Some(x[i]);
            }
            self.z = l;
            out[i] = x[i];
        }
    }

    fn process_events(&mut self, props: &Props, emit: &mut dyn FnMut(&str, Value)) {
        if let Some(v) = self.readout.take() {
            emit(
                "snapshot",
                Value::object([("source", source_name(props)), ("data", v.into())]),
            );
        }
    }
}

//==============================================================================

/// `scope`: buffers up to four input channels and emits `size` samples at a time.
pub struct ScopeNode {
    ring: MultiChannelRingBuffer,
    size: usize,
    channels: usize,
    scratch: Vec<Vec<f32>>,
}

impl ScopeNode {
    pub fn new(_: NodeInfo) -> Self {
        Self {
            ring: MultiChannelRingBuffer::new(4, 8192),
            size: 512,
            channels: 1,
            scratch: Vec::new(),
        }
    }
}

impl Node for ScopeNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        match key {
            "size" => {
                let v = expect_number(val)?;
                if !(256.0..=8192.0).contains(&v) {
                    return Err(Error::InvalidPropertyValue);
                }
                self.size = v as usize;
            }
            "channels" => {
                let v = expect_number(val)?;
                if !(0.0..=4.0).contains(&v) {
                    return Err(Error::InvalidPropertyValue);
                }
                self.channels = v as usize;
            }
            "name" => {
                expect_string(val)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        let n_in = ctx.num_inputs();
        if n_in < 1 {
            out.fill(0.0);
            return;
        }
        out.copy_from_slice(ctx.input(0));
        let inputs: [&[f32]; 4] = [
            ctx.input(0),
            if n_in > 1 { ctx.input(1) } else { &[] },
            if n_in > 2 { ctx.input(2) } else { &[] },
            if n_in > 3 { ctx.input(3) } else { &[] },
        ];
        self.ring.write(&inputs[..n_in.min(4)], ctx.num_samples, 0);
    }

    fn process_events(&mut self, props: &Props, emit: &mut dyn FnMut(&str, Value)) {
        if self.ring.size() <= self.size {
            return;
        }
        self.scratch.resize(self.channels, Vec::new());
        for ch in &mut self.scratch {
            ch.clear();
            ch.resize(self.size, 0.0);
        }
        let mut dest: Vec<&mut [f32]> = self.scratch.iter_mut().map(|c| c.as_mut_slice()).collect();
        if self.ring.read(&mut dest, self.size) {
            let data: Vec<Value> = self
                .scratch
                .iter()
                .map(|c| Value::Float32Array(c.clone()))
                .collect();
            emit(
                "scope",
                Value::object([("source", source_name(props)), ("data", Value::Array(data))]),
            );
        }
    }
}

//==============================================================================

/// `capture`: inputs [gate, signal]; records the signal while the gate is high
/// and emits the recording on the gate's falling edge.
pub struct CaptureNode {
    change: Change,
    ring: MultiChannelRingBuffer,
    scratch: [f32; 128],
    scratch_size: usize,
    relay: Vec<f32>,
    relay_ready: bool,
}

impl CaptureNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            change: Change::default(),
            ring: MultiChannelRingBuffer::new(1, bitceil(info.sample_rate as usize)),
            scratch: [0.0; 128],
            scratch_size: 0,
            relay: Vec::new(),
            relay_ready: false,
        }
    }

    fn flush_scratch(&mut self) {
        let data: [&[f32]; 1] = [&self.scratch[..self.scratch_size]];
        self.ring.write(&data, self.scratch_size, 0);
        self.scratch_size = 0;
    }
}

impl Node for CaptureNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let gate = ctx.input(0);
        let signal = ctx.input(1);
        out.copy_from_slice(signal);
        for i in 0..ctx.num_samples {
            let g = gate[i] != 0.0;
            let falling = self.change.tick(gate[i]) < -0.5;
            if falling || self.scratch_size >= self.scratch.len() {
                self.flush_scratch();
                if falling {
                    self.relay_ready = true;
                }
            }
            if g {
                self.scratch[self.scratch_size] = signal[i];
                self.scratch_size += 1;
            }
        }
    }

    fn process_events(&mut self, props: &Props, emit: &mut dyn FnMut(&str, Value)) {
        let available = self.ring.size();
        if available > 0 {
            let start = self.relay.len();
            self.relay.resize(start + available, 0.0);
            let mut dest: [&mut [f32]; 1] = [&mut self.relay[start..]];
            if !self.ring.read(&mut dest, available) {
                self.relay.truncate(start);
                return;
            }
        }
        if std::mem::take(&mut self.relay_ready) {
            let data = std::mem::take(&mut self.relay);
            emit(
                "capture",
                Value::object([("source", source_name(props)), ("data", Value::Float32Array(data))]),
            );
        }
    }
}

//==============================================================================

/// `mc.capture`: inputs [gate, ch0, ch1, ...]; multi-channel `capture`.
pub struct McCaptureNode {
    ring: Option<MultiChannelRingBuffer>,
    relay: Vec<Vec<f32>>,
    pending: Vec<Vec<f32>>,
    relay_ready: bool,
    change: Change,
    sample_rate: f64,
}

impl McCaptureNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            ring: None,
            relay: Vec::new(),
            pending: Vec::new(),
            relay_ready: false,
            change: Change::default(),
            sample_rate: info.sample_rate,
        }
    }
}

impl Node for McCaptureNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "_internal:numChildren" {
            let n = expect_number(val)?;
            if n > 1.0 {
                let chans = n as usize - 1;
                self.ring = Some(MultiChannelRingBuffer::new(chans, bitceil(self.sample_rate as usize)));
                self.relay = (0..chans).map(|_| vec![0.0; 8192]).collect();
            }
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let n_in = ctx.num_inputs();
        let n_out = ctx.num_outputs();
        let n = ctx.num_samples;
        if n_in < 2 {
            ctx.zero_outputs();
            return;
        }
        for i in 0..n_out {
            let out = ctx.output(i);
            if i + 1 < n_in {
                out.copy_from_slice(ctx.input(i + 1));
            } else {
                out.fill(0.0);
            }
        }
        let gate = ctx.input(0);
        let signals: Vec<&[f32]> = (1..n_in).map(|i| ctx.input(i)).collect();
        let mut write_start = 0usize;
        let mut write_stop = 0usize;
        for (i, &level) in gate[..n].iter().enumerate() {
            let g = level != 0.0;
            let dg = self.change.tick(level);
            let falling = dg < -0.5;
            let rising = dg > 0.5;
            if falling {
                if let Some(ring) = &mut self.ring {
                    ring.write(&signals, write_stop - write_start, write_start);
                }
                self.relay_ready = true;
                write_start = 0;
                write_stop = 0;
            }
            if g {
                write_stop = i + 1;
                if rising {
                    write_start = i;
                }
            }
        }
        if let Some(ring) = &mut self.ring {
            if write_stop > write_start {
                ring.write(&signals, write_stop - write_start, write_start);
            }
        }
    }

    fn process_events(&mut self, props: &Props, emit: &mut dyn FnMut(&str, Value)) {
        let Some(ring) = &mut self.ring else { return };
        let mut available = ring.size();
        let chans = ring.num_channels().min(self.relay.len());
        while available > 0 {
            let to_read = available.min(8192);
            let mut dest: Vec<&mut [f32]> = self.relay.iter_mut().map(|c| c.as_mut_slice()).collect();
            if !ring.read(&mut dest, to_read) {
                return;
            }
            available -= to_read;
            self.pending.resize(chans, Vec::new());
            for i in 0..chans {
                self.pending[i].extend_from_slice(&self.relay[i][..to_read]);
            }
        }
        if std::mem::take(&mut self.relay_ready) {
            let data: Vec<Value> = self
                .pending
                .iter_mut()
                .map(|c| Value::Float32Array(std::mem::take(c)))
                .collect();
            emit(
                "mc.capture",
                Value::object([("source", source_name(props)), ("data", Value::Array(data))]),
            );
        }
    }
}
