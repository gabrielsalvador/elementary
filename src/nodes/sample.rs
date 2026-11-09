//! Triggered sample playback (`sample`, `mc.sample`) and wavetable lookup
//! (`table`, `mc.table`) from shared buffer resources.

use crate::node::{expect_number, expect_string, lerp, BlockContext, Change, GainFade, Node, NodeInfo};
use crate::resources::{SharedResourceMap, SharedResourcePtr};
use crate::value::Value;
use crate::Error;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlayMode {
    Trigger,
    Gate,
    Loop,
}

pub(crate) fn parse_path(val: &Value, resources: &SharedResourceMap) -> Result<SharedResourcePtr, Error> {
    let path = expect_string(val)?;
    resources.get(path).ok_or(Error::InvalidPropertyValue)
}

fn parse_offset(val: &Value) -> Result<usize, Error> {
    let v = expect_number(val)? as i64;
    if v < 0 {
        return Err(Error::InvalidPropertyValue);
    }
    Ok(v as usize)
}

fn parse_mode(val: &Value, mode: &mut PlayMode) -> Result<(), Error> {
    match expect_string(val)? {
        "trigger" => *mode = PlayMode::Trigger,
        "gate" => *mode = PlayMode::Gate,
        "loop" => *mode = PlayMode::Loop,
        _ => {}
    }
    Ok(())
}

//==============================================================================

/// Linear-interpolating mono reader with a smoothed gain envelope.
#[derive(Clone)]
struct VariablePitchLerpReader {
    source: Option<SharedResourcePtr>,
    gain_smooth_alpha: f32,
    target_gain: f32,
    gain: f32,
    pos: f64,
}

impl VariablePitchLerpReader {
    fn new(sample_rate: f64, source: Option<SharedResourcePtr>) -> Self {
        Self {
            source,
            gain_smooth_alpha: (1.0 - (-1.0 / (0.01 * sample_rate)).exp()) as f32,
            target_gain: 0.0,
            gain: 0.0,
            pos: 0.0,
        }
    }

    fn note_on(&mut self, start_offset: usize) {
        self.target_gain = 1.0;
        self.pos = start_offset as f64;
    }

    fn note_off(&mut self) {
        self.target_gain = 0.0;
    }

    fn tick(&mut self, start_offset: usize, stop_offset: usize, step: f32, wants_loop: bool) -> f32 {
        let Some(src) = &self.source else { return 0.0 };
        if self.pos < 0.0 || (self.gain == 0.0 && self.target_gain == 0.0) {
            return 0.0;
        }
        let Some(data) = src.channel(0) else { return 0.0 };
        let len = data.len();
        if len == 0 {
            return 0.0;
        }
        if self.pos >= len.saturating_sub(stop_offset) as f64 {
            if !wants_loop {
                return 0.0;
            }
            self.pos = start_offset as f64;
        }
        let mut left = self.pos as usize;
        let mut right = left + 1;
        let frac = (self.pos - left as f64) as f32;
        if left >= len {
            left -= len;
        }
        if right >= len {
            right -= len;
        }
        let l = data[left.min(len - 1)];
        let r = data[right.min(len - 1)];
        let out = self.gain * (l + frac * (r - l));
        let settled = (self.target_gain - self.gain).abs() <= f32::EPSILON;
        self.pos += step as f64;
        self.gain = if settled {
            self.target_gain
        } else {
            self.gain + self.gain_smooth_alpha * (self.target_gain - self.gain)
        };
        self.gain = self.gain.clamp(0.0, 1.0);
        out
    }
}

/// `sample`: inputs [trigger, (playback rate)].
pub struct SampleNode {
    pending: Option<SharedResourcePtr>,
    buffer: Option<SharedResourcePtr>,
    change: Change,
    readers: [VariablePitchLerpReader; 2],
    current_reader: usize,
    mode: PlayMode,
    start_offset: usize,
    stop_offset: usize,
    sample_rate: f64,
}

impl SampleNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            pending: None,
            buffer: None,
            change: Change::default(),
            readers: [
                VariablePitchLerpReader::new(info.sample_rate, None),
                VariablePitchLerpReader::new(info.sample_rate, None),
            ],
            current_reader: 0,
            mode: PlayMode::Trigger,
            start_offset: 0,
            stop_offset: 0,
            sample_rate: info.sample_rate,
        }
    }
}

impl Node for SampleNode {
    fn set_property(&mut self, key: &str, val: &Value, resources: &mut SharedResourceMap) -> Result<(), Error> {
        match key {
            "path" => self.pending = Some(parse_path(val, resources)?),
            "mode" => parse_mode(val, &mut self.mode)?,
            "startOffset" => self.start_offset = parse_offset(val)?,
            "stopOffset" => self.stop_offset = parse_offset(val)?,
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.readers[0].note_off();
        self.readers[1].note_off();
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if let Some(buf) = self.pending.take() {
            self.readers[0] = VariablePitchLerpReader::new(self.sample_rate, Some(buf.clone()));
            self.readers[1] = VariablePitchLerpReader::new(self.sample_rate, Some(buf.clone()));
            self.buffer = Some(buf);
        }
        if ctx.num_inputs() < 1 || self.buffer.is_none() {
            out.fill(0.0);
            return;
        }
        let wants_loop = self.mode == PlayMode::Loop;
        let trig = ctx.input(0);
        let rate = if ctx.num_inputs() >= 2 {
            Some(ctx.input(1))
        } else {
            None
        };
        for i in 0..ctx.num_samples {
            let cv = self.change.tick(trig[i]);
            let step = rate.map_or(1.0, |r| r[i]);
            if cv > 0.5 {
                self.readers[self.current_reader & 1].note_off();
                self.current_reader += 1;
                self.readers[self.current_reader & 1].note_on(self.start_offset);
            }
            if cv < -0.5 && self.mode != PlayMode::Trigger {
                self.readers[self.current_reader & 1].note_off();
            }
            out[i] = self.readers[0].tick(self.start_offset, self.stop_offset, step, wants_loop)
                + self.readers[1].tick(self.start_offset, self.stop_offset, step, wants_loop);
        }
    }
}

//==============================================================================

#[derive(Clone)]
struct McVariablePitchReader {
    source: Option<SharedResourcePtr>,
    fade: GainFade,
    should_loop: bool,
    stop_offset: f64,
    start_offset: f64,
    pos: f64,
}

impl McVariablePitchReader {
    fn new(sample_rate: f64, source: Option<SharedResourcePtr>) -> Self {
        Self {
            source,
            fade: GainFade::new(sample_rate, 4.0, 4.0, 0.0, 0.0),
            should_loop: false,
            stop_offset: 0.0,
            start_offset: 0.0,
            pos: 0.0,
        }
    }

    fn note_on(&mut self, start: usize, stop: usize, wants_loop: bool) {
        self.fade.fade_in();
        self.start_offset = start as f64;
        self.stop_offset = stop as f64;
        self.should_loop = wants_loop;
        self.pos = self.start_offset;
    }

    fn note_off(&mut self) {
        self.fade.fade_out();
    }

    fn lerp_read(data: &[f32], pos: f64) -> f32 {
        let left = pos as usize;
        let right = left + 1;
        let alpha = (pos - left as f64) as f32;
        if left >= data.len() {
            0.0
        } else if right >= data.len() {
            data[left]
        } else {
            lerp(alpha, data[left], data[right])
        }
    }

    fn sum_into(&mut self, ctx: &BlockContext<'_>, write_offset: usize, num_samples: usize, rate: f64) {
        let Some(src) = &self.source else { return };
        let mut local_fade = self.fade;
        let read_start = self.start_offset;
        let mut read_stop = 0.0f64;
        let n_ch = ctx.num_outputs().min(src.num_channels());
        for ch in 0..n_ch {
            let Some(data) = src.channel(ch) else { continue };
            read_stop = data.len() as f64 - self.stop_offset;
            local_fade = self.fade;
            let out = ctx.output(ch);
            for j in 0..num_samples {
                let mut read_pos = self.pos + j as f64 * rate;
                if read_pos >= read_stop {
                    if self.should_loop && read_stop > read_start {
                        read_pos = read_start + (read_pos - read_start) % (read_stop - read_start);
                    } else {
                        continue;
                    }
                }
                out[write_offset + j] += local_fade.tick(Self::lerp_read(data, read_pos));
            }
        }
        self.fade = local_fade;
        self.pos += num_samples as f64 * rate;
        if self.pos >= read_stop && self.should_loop && read_stop > read_start {
            self.pos = read_start + (self.pos - read_start) % (read_stop - read_start);
        }
    }
}

/// `mc.sample`: multi-channel triggered sample playback; input [trigger].
pub struct McSampleNode {
    pending: Option<SharedResourcePtr>,
    buffer: Option<SharedResourcePtr>,
    change: Change,
    readers: [McVariablePitchReader; 2],
    current_reader: usize,
    mode: PlayMode,
    start_offset: usize,
    stop_offset: usize,
    playback_rate: f64,
    sample_rate: f64,
}

impl McSampleNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            pending: None,
            buffer: None,
            change: Change::default(),
            readers: [
                McVariablePitchReader::new(info.sample_rate, None),
                McVariablePitchReader::new(info.sample_rate, None),
            ],
            current_reader: 0,
            mode: PlayMode::Trigger,
            start_offset: 0,
            stop_offset: 0,
            playback_rate: 1.0,
            sample_rate: info.sample_rate,
        }
    }

    // Both readers, the one fading out and the one playing, summed into the outputs.
    fn sum_readers(&mut self, ctx: &BlockContext<'_>, write_offset: usize, num_samples: usize, rate: f64) {
        for reader in &mut self.readers {
            reader.sum_into(ctx, write_offset, num_samples, rate);
        }
    }
}

impl Node for McSampleNode {
    fn set_property(&mut self, key: &str, val: &Value, resources: &mut SharedResourceMap) -> Result<(), Error> {
        match key {
            "path" => self.pending = Some(parse_path(val, resources)?),
            "mode" => parse_mode(val, &mut self.mode)?,
            "startOffset" => self.start_offset = parse_offset(val)?,
            "stopOffset" => self.stop_offset = parse_offset(val)?,
            "playbackRate" => self.playback_rate = expect_number(val)?,
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.readers[0].note_off();
        self.readers[1].note_off();
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        if let Some(buf) = self.pending.take() {
            self.readers[0] = McVariablePitchReader::new(self.sample_rate, Some(buf.clone()));
            self.readers[1] = McVariablePitchReader::new(self.sample_rate, Some(buf.clone()));
            self.buffer = Some(buf);
        }
        ctx.zero_outputs();
        if ctx.num_inputs() < 1 || self.buffer.is_none() {
            return;
        }
        let wants_loop = self.mode == PlayMode::Loop;
        let rate = self.playback_rate;
        let trig = ctx.input(0);
        let mut i = 0usize;
        let n = ctx.num_samples;
        for (j, &t) in trig[..n].iter().enumerate() {
            let cv = self.change.tick(t);
            if cv > 0.5 {
                self.sum_readers(ctx, i, j - i, rate);
                self.readers[self.current_reader & 1].note_off();
                self.current_reader += 1;
                self.readers[self.current_reader & 1].note_on(self.start_offset, self.stop_offset, wants_loop);
                i = j;
            }
            if cv < -0.5 && self.mode != PlayMode::Trigger {
                self.sum_readers(ctx, i, j - i, rate);
                self.readers[self.current_reader & 1].note_off();
                i = j;
            }
        }
        self.sum_readers(ctx, i, n - i, rate);
    }
}

//==============================================================================

fn table_read(data: &[f32], pos01: f32) -> f32 {
    let len = data.len();
    let read_pos = pos01.clamp(0.0, 1.0) * (len - 1) as f32;
    let left = read_pos as usize;
    let right = left + 1;
    let frac = read_pos - read_pos.floor();
    let l = data[left % len];
    let r = data[right % len];
    l + frac * (r - l)
}

/// `table`: input [position 0..1]; reads channel 0 of the buffer.
pub struct TableNode {
    pending: Option<SharedResourcePtr>,
    buffer: Option<SharedResourcePtr>,
}

impl TableNode {
    pub fn new(_: NodeInfo) -> Self {
        Self {
            pending: None,
            buffer: None,
        }
    }
}

impl Node for TableNode {
    fn set_property(&mut self, key: &str, val: &Value, resources: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "path" {
            self.pending = Some(parse_path(val, resources)?);
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if let Some(buf) = self.pending.take() {
            self.buffer = Some(buf);
        }
        let data = match &self.buffer {
            Some(buf) if ctx.num_inputs() > 0 => buf.channel(0),
            _ => None,
        };
        match data {
            Some(data) if !data.is_empty() => {
                let pos = ctx.input(0);
                for i in 0..ctx.num_samples {
                    out[i] = table_read(data, pos[i]);
                }
            }
            _ => out.fill(0.0),
        }
    }
}

/// `mc.table`: like `table`, with one output per buffer channel.
pub struct McTableNode {
    pending: Option<SharedResourcePtr>,
    buffer: Option<SharedResourcePtr>,
}

impl McTableNode {
    pub fn new(_: NodeInfo) -> Self {
        Self {
            pending: None,
            buffer: None,
        }
    }
}

impl Node for McTableNode {
    fn set_property(&mut self, key: &str, val: &Value, resources: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "path" {
            self.pending = Some(parse_path(val, resources)?);
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        if let Some(buf) = self.pending.take() {
            self.buffer = Some(buf);
        }
        let Some(buf) = &self.buffer else {
            ctx.zero_outputs();
            return;
        };
        if ctx.num_inputs() == 0 {
            ctx.zero_outputs();
            return;
        }
        let pos = ctx.input(0);
        for ch in 0..ctx.num_outputs() {
            let out = ctx.output(ch);
            match buf.channel(ch) {
                Some(data) if !data.is_empty() => {
                    for i in 0..ctx.num_samples {
                        out[i] = table_read(data, pos[i]);
                    }
                }
                _ => out.fill(0.0),
            }
        }
    }
}
