//! Root, const, phasor, sample rate, counters, latches and the basic sequencers.

use crate::node::{expect_bool, expect_number, BlockContext, Change, GainFade, Node, NodeInfo};
use crate::resources::SharedResourceMap;
use crate::value::Value;
use crate::Error;

//==============================================================================

pub struct RootNode {
    fade: GainFade,
    channel: i32,
    sample_rate: f64,
}

impl RootNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            fade: GainFade::new(info.sample_rate, 20.0, 20.0, 0.0, 1.0),
            channel: -1,
            sample_rate: info.sample_rate,
        }
    }

    pub fn channel(&self) -> i32 {
        self.channel
    }

    pub fn active(&self) -> bool {
        self.fade.on()
    }

    pub fn still_running(&self) -> bool {
        self.active() || !self.fade.settled()
    }
}

impl Node for RootNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        match key {
            "active" => {
                if expect_bool(val)? {
                    self.fade.fade_in();
                } else {
                    self.fade.fade_out();
                }
            }
            "channel" => self.channel = expect_number(val)? as i32,
            "fadeInMs" => self.fade.set_fade_in_ms(self.sample_rate, expect_number(val)?),
            "fadeOutMs" => self.fade.set_fade_out_ms(self.sample_rate, expect_number(val)?),
            _ => {}
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 1 {
            out.fill(0.0);
            return;
        }
        self.fade.process(ctx.input(0), out);
    }

    fn as_root(&self) -> Option<&RootNode> {
        Some(self)
    }
}

//==============================================================================

pub struct PhasorNode<const WITH_RESET: bool> {
    phase: f32,
    change: Change,
    inv_sr: f32,
}

impl<const WITH_RESET: bool> PhasorNode<WITH_RESET> {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            phase: 0.0,
            change: Change::default(),
            inv_sr: 1.0 / info.sample_rate as f32,
        }
    }

    fn tick(&mut self, freq: f32) -> f32 {
        let step = freq * self.inv_sr;
        let y = self.phase;
        let next = self.phase + step;
        self.phase = next - next.floor();
        y
    }
}

impl<const WITH_RESET: bool> Node for PhasorNode<WITH_RESET> {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        let required = if WITH_RESET { 2 } else { 1 };
        if ctx.num_inputs() < required {
            out.fill(0.0);
            return;
        }
        let freq = ctx.input(0);
        if WITH_RESET {
            let reset = ctx.input(1);
            for i in 0..ctx.num_samples {
                if self.change.tick(reset[i]) > 0.5 {
                    self.phase = 0.0;
                }
                out[i] = self.tick(freq[i]);
            }
        } else {
            for i in 0..ctx.num_samples {
                out[i] = self.tick(freq[i]);
            }
        }
    }
}

//==============================================================================

pub struct ConstNode {
    value: f32,
}

impl ConstNode {
    pub fn new(_: NodeInfo) -> Self {
        Self { value: 1.0 }
    }
}

impl Node for ConstNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "value" {
            self.value = expect_number(val)? as f32;
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        ctx.output(0).fill(self.value);
    }
}

//==============================================================================

pub struct SampleRateNode {
    sr: f32,
}

impl SampleRateNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            sr: info.sample_rate as f32,
        }
    }
}

impl Node for SampleRateNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        ctx.output(0).fill(self.sr);
    }
}

//==============================================================================

#[derive(Default)]
pub struct CounterNode {
    count: f32,
}

impl CounterNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for CounterNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 1 {
            out.fill(0.0);
            return;
        }
        let input = ctx.input(0);
        for i in 0..ctx.num_samples {
            if (1.0 - input[i]) <= f32::EPSILON {
                out[i] = self.count;
                self.count += 1.0;
            } else {
                self.count = 0.0;
                out[i] = 0.0;
            }
        }
    }
}

//==============================================================================

#[derive(Default)]
pub struct AccumNode {
    change: Change,
    total: f32,
}

impl AccumNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for AccumNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let input = ctx.input(0);
        let reset = ctx.input(1);
        for i in 0..ctx.num_samples {
            if self.change.tick(reset[i]) > 0.5 {
                self.total = 0.0;
            }
            self.total += input[i];
            out[i] = self.total;
        }
    }
}

//==============================================================================

#[derive(Default)]
pub struct LatchNode {
    z: f32,
    hold: f32,
}

impl LatchNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for LatchNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let gate = ctx.input(0);
        let x = ctx.input(1);
        for i in 0..ctx.num_samples {
            let l = gate[i];
            if self.z.abs() <= f32::EPSILON && l > f32::EPSILON {
                self.hold = x[i];
            }
            self.z = l;
            out[i] = self.hold;
        }
    }
}

//==============================================================================

pub struct MaxHoldNode {
    change: Change,
    hold_samples: u32,
    samples_at_max: u32,
    max: f32,
    sample_rate: f64,
}

impl MaxHoldNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            change: Change::default(),
            hold_samples: u32::MAX,
            samples_at_max: 0,
            max: 0.0,
            sample_rate: info.sample_rate,
        }
    }
}

impl Node for MaxHoldNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "hold" {
            let h = self.sample_rate * 0.001 * expect_number(val)?;
            self.hold_samples = h.max(0.0) as u32;
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let input = ctx.input(0);
        let reset = ctx.input(1);
        let hts = self.hold_samples;
        for i in 0..ctx.num_samples {
            let x = input[i];
            let reset_edge = self.change.tick(reset[i]) > 0.5;
            let expired = if reset_edge {
                false
            } else {
                self.samples_at_max = self.samples_at_max.wrapping_add(1);
                self.samples_at_max >= hts
            };
            if reset_edge || expired {
                self.max = x;
                self.samples_at_max = 0;
            } else if x > self.max {
                self.samples_at_max = 0;
                self.max = x;
            }
            out[i] = self.max;
        }
    }
}

//==============================================================================

#[derive(Default)]
pub struct OnceNode {
    armed: bool,
    gain: f32,
    change: Change,
}

impl OnceNode {
    pub fn new(_: NodeInfo) -> Self {
        Self::default()
    }
}

impl Node for OnceNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "arm" {
            let v = expect_bool(val)?;
            if !self.armed {
                self.armed = v;
            }
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 1 {
            out.fill(0.0);
            return;
        }
        let input = ctx.input(0);
        let is_armed = self.armed;
        for i in 0..ctx.num_samples {
            let delta = self.change.tick(input[i]);
            let rising = delta > 0.5;
            let falling = delta < -0.5;
            if is_armed && rising {
                self.gain = 1.0;
                self.armed = false;
            }
            if falling {
                self.gain = 0.0;
            }
            out[i] = input[i] * self.gain;
        }
    }
}

//==============================================================================

/// `seq`: steps through a list of values on each rising edge of the trigger input.
pub struct SequenceNode {
    pending: Option<Vec<f32>>,
    active: Option<Vec<f32>>,
    change: Change,
    reset_change: Change,
    hold: bool,
    looping: bool,
    offset: usize,
    hold_value: f32,
    index: usize,
    received_first_pulse: bool,
}

impl SequenceNode {
    pub fn new(_: NodeInfo) -> Self {
        Self {
            pending: None,
            active: None,
            change: Change::default(),
            reset_change: Change::default(),
            hold: false,
            looping: true,
            offset: 0,
            hold_value: 0.0,
            index: 0,
            received_first_pulse: false,
        }
    }
}

pub(crate) fn parse_offset(val: &Value) -> Result<usize, Error> {
    let v = expect_number(val)?;
    if v < 0.0 {
        return Err(Error::InvalidPropertyValue);
    }
    Ok(v as usize)
}

pub(crate) fn parse_seq_numbers(val: &Value) -> Result<Vec<f32>, Error> {
    match val {
        Value::Array(_) | Value::Float32Array(_) => val.as_f32_list().ok_or(Error::InvalidPropertyType),
        _ => Err(Error::InvalidPropertyType),
    }
}

impl Node for SequenceNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        match key {
            "hold" => self.hold = expect_bool(val)?,
            "loop" => self.looping = expect_bool(val)?,
            "offset" => self.offset = parse_offset(val)?,
            "seq" => self.pending = Some(parse_seq_numbers(val)?),
            _ => {}
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if let Some(seq) = self.pending.take() {
            self.index = if seq.is_empty() { 0 } else { self.index % seq.len() };
            if self.received_first_pulse {
                if let Some(&v) = seq.get(self.index) {
                    self.hold_value = v;
                }
            }
            self.active = Some(seq);
        }
        let seq = match &self.active {
            Some(seq) if ctx.num_inputs() >= 1 && !seq.is_empty() => seq,
            _ => {
                out.fill(0.0);
                return;
            }
        };
        let input = ctx.input(0);
        let reset = if ctx.num_inputs() > 1 {
            Some(ctx.input(1))
        } else {
            None
        };
        let len = seq.len();
        for i in 0..ctx.num_samples {
            let x = input[i];
            let r = reset.map_or(0.0, |r| r[i]);
            if self.reset_change.tick(r) > 0.5 {
                self.index = self.offset;
            }
            if self.change.tick(x) > 0.5 {
                self.hold_value = seq[self.index.min(len - 1)];
                self.received_first_pulse = true;
                self.index += 1;
                if self.index >= len && self.looping {
                    self.index = 0;
                }
            }
            out[i] = if self.index < len {
                if self.hold {
                    self.hold_value
                } else {
                    self.hold_value * x
                }
            } else if self.hold {
                self.hold_value
            } else {
                0.0
            };
        }
    }
}

//==============================================================================

/// `seq2`: like `seq`, but indexes by counting trigger edges since the last reset.
pub struct Seq2Node {
    pending: Option<Vec<f32>>,
    active: Option<Vec<f32>>,
    change: Change,
    reset_change: Change,
    hold: bool,
    looping: bool,
    offset: usize,
    edge_count: usize,
}

impl Seq2Node {
    pub fn new(_: NodeInfo) -> Self {
        Self {
            pending: None,
            active: None,
            change: Change::default(),
            reset_change: Change::default(),
            hold: false,
            looping: true,
            offset: 0,
            edge_count: 0,
        }
    }
}

impl Node for Seq2Node {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        match key {
            "hold" => self.hold = expect_bool(val)?,
            "loop" => self.looping = expect_bool(val)?,
            "offset" => self.offset = parse_offset(val)?,
            "seq" => self.pending = Some(parse_seq_numbers(val)?),
            _ => {}
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if let Some(seq) = self.pending.take() {
            self.active = Some(seq);
        }
        let seq = match &self.active {
            Some(seq) if ctx.num_inputs() >= 1 && !seq.is_empty() => seq,
            _ => {
                out.fill(0.0);
                return;
            }
        };
        let input = ctx.input(0);
        let reset = if ctx.num_inputs() > 1 {
            Some(ctx.input(1))
        } else {
            None
        };
        let len = seq.len();
        for i in 0..ctx.num_samples {
            let x = input[i];
            let r = reset.map_or(0.0, |r| r[i]);
            if self.change.tick(x) > 0.5 {
                self.edge_count += 1;
            }
            if self.reset_change.tick(r) > 0.5 {
                self.edge_count = 0;
            }
            let idx = self.offset + self.edge_count;
            let next = if idx < len {
                seq[idx]
            } else if self.looping {
                seq[idx % len]
            } else if self.hold {
                seq[len - 1]
            } else {
                0.0
            };
            out[i] = if self.hold { next } else { next * x };
        }
    }
}
