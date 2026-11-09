//! Timeline-driven sample playback: `sampleseq`, `sampleseq2`, `mc.sampleseq`,
//! `mc.sampleseq2`.
//!
//! The input signal is a time in seconds. The `seq` property maps times to
//! gate values; while the most recent event's value is 1 the sample plays,
//! positioned so that it started at that event's time.
//!
//! The `*2` variants accept the `shift` and `stretch` properties for API
//! compatibility, but do not time-stretch: the original implementation used
//! the Signalsmith Stretch library, which is not ported. They play back at
//! the natural rate.

use crate::node::{expect_number, fp_equal, BlockContext, GainFade, Node, NodeInfo};
use crate::resources::{SharedResourceMap, SharedResourcePtr};
use crate::value::Value;
use crate::Error;

use super::sample::parse_path;

/// Sorted, deduplicated `(time, value)` events. Matches `std::map` semantics:
/// the first insert for a given time wins.
pub(crate) type EventSeq = Vec<(f64, f32)>;

pub(crate) fn parse_event_seq(val: &Value, time_key: &str) -> Result<EventSeq, Error> {
    let items = val.as_array().ok_or(Error::InvalidPropertyType)?;
    let mut seq: EventSeq = Vec::with_capacity(items.len());
    for item in items {
        let obj = item.as_object().ok_or(Error::InvalidPropertyType)?;
        let time = obj
            .get(time_key)
            .and_then(Value::as_number)
            .ok_or(Error::InvalidPropertyType)?;
        let value = obj
            .get("value")
            .and_then(Value::as_number)
            .ok_or(Error::InvalidPropertyType)? as f32;
        seq.push((time, value));
    }
    seq.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    seq.dedup_by(|b, a| a.0 == b.0);
    Ok(seq)
}

/// Index of the first event with time > t (`upper_bound`).
pub(crate) fn upper_bound(seq: &EventSeq, t: f64) -> usize {
    seq.partition_point(|(time, _)| *time <= t)
}

/// Simple linear fade used by the mono sample sequencer's readers.
#[derive(Clone, Copy)]
struct StepFade {
    current: f32,
    target: f32,
    step: f32,
}

impl Default for StepFade {
    fn default() -> Self {
        Self {
            current: 0.0,
            target: 0.0,
            step: 0.02,
        }
    }
}

impl StepFade {
    fn set_target(&mut self, g: f32) {
        self.target = g;
        self.step = if self.target < self.current {
            -self.step.abs()
        } else {
            self.step.abs()
        };
    }

    fn tick(&mut self, x: f32) -> f32 {
        if self.current == self.target {
            return self.current * x;
        }
        let y = x * self.current;
        self.current = (self.current + self.step).clamp(0.0, 1.0);
        y
    }

    fn on(&self) -> bool {
        fp_equal(self.target, 1.0)
    }

    fn reset(&mut self) {
        self.current = 0.0;
        self.target = 0.0;
    }
}

fn position_for(start_time: f64, t: f64, sample_duration: f64, buffer_size: usize) -> usize {
    if sample_duration <= 0.0 || buffer_size == 0 {
        return 0;
    }
    let p = ((t - start_time) / sample_duration) * (buffer_size - 1) as f64;
    (p.max(0.0) as usize).min(buffer_size)
}

#[derive(Clone, Default)]
struct BufferReader {
    fade: StepFade,
    source: Option<SharedResourcePtr>,
    buffer_size: usize,
    position: usize,
    sample_duration: f64,
    start_time: f64,
}

impl BufferReader {
    fn engage(&mut self, start: f64, current_time: f64, source: SharedResourcePtr) {
        self.start_time = start;
        self.buffer_size = source.channel(0).map_or(0, |c| c.len());
        self.source = Some(source);
        self.fade.set_target(1.0);
        self.position = position_for(start, current_time, self.sample_duration, self.buffer_size);
    }

    fn disengage(&mut self) {
        self.fade.set_target(0.0);
    }

    fn is_aligned_with_time(&self, t: f64) -> bool {
        if !self.fade.on() {
            return true;
        }
        let new_pos = position_for(self.start_time, t, self.sample_duration, self.buffer_size);
        (self.position as i64 - new_pos as i64).abs() < 16
    }

    fn read_adding(&mut self, out: &mut [f32]) {
        let Some(src) = &self.source else { return };
        let Some(data) = src.channel(0) else { return };
        for o in out.iter_mut() {
            if self.position >= data.len() {
                break;
            }
            *o += self.fade.tick(data[self.position]);
            self.position += 1;
        }
    }

    fn reset(&mut self, sample_duration: f64) {
        self.fade.reset();
        self.sample_duration = sample_duration;
        self.start_time = 0.0;
    }
}

/// `sampleseq` / `sampleseq2`: input [time in seconds].
pub struct SampleSeqNode {
    with_stretch: bool,
    pending_buffer: Option<SharedResourcePtr>,
    buffer: Option<SharedResourcePtr>,
    pending_seq: Option<EventSeq>,
    seq: Option<EventSeq>,
    prev_event: Option<usize>,
    next_event: Option<usize>,
    readers: [BufferReader; 2],
    active_reader: usize,
    sample_duration: f64,
    rt_sample_duration: f64,
}

impl SampleSeqNode {
    pub fn new(_: NodeInfo, with_stretch: bool) -> Self {
        Self {
            with_stretch,
            pending_buffer: None,
            buffer: None,
            pending_seq: None,
            seq: None,
            prev_event: None,
            next_event: None,
            readers: [BufferReader::default(), BufferReader::default()],
            active_reader: 0,
            sample_duration: 0.0,
            rt_sample_duration: 0.0,
        }
    }

    fn update_event_boundaries(&mut self, t: f64) {
        let (Some(seq), Some(buffer)) = (&self.seq, &self.buffer) else { return };
        let next = upper_bound(seq, t);
        if next == 0 {
            self.prev_event = None;
            self.next_event = if seq.is_empty() { None } else { Some(0) };
            self.readers[0].disengage();
            self.readers[1].disengage();
            return;
        }
        self.next_event = if next < seq.len() { Some(next) } else { None };
        self.prev_event = Some(next - 1);
        self.readers[self.active_reader].disengage();
        self.active_reader = (self.active_reader + 1) & 1;
        let (start, value) = seq[next - 1];
        if fp_equal(value, 1.0) {
            self.readers[self.active_reader].engage(start, t, buffer.clone());
        }
    }
}

pub(crate) fn validate_stretch_props(key: &str, val: &Value) -> Result<(), Error> {
    match key {
        "shift" => {
            expect_number(val)?;
        }
        "stretch" => {
            let v = expect_number(val)?;
            if !(0.25..=4.0).contains(&v) {
                return Err(Error::InvalidPropertyValue);
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn parse_duration(val: &Value) -> Result<f64, Error> {
    let dur = expect_number(val)?;
    if dur <= 0.0 {
        return Err(Error::InvalidPropertyValue);
    }
    Ok(dur)
}

impl Node for SampleSeqNode {
    fn set_property(&mut self, key: &str, val: &Value, resources: &mut SharedResourceMap) -> Result<(), Error> {
        if self.with_stretch {
            validate_stretch_props(key, val)?;
        }
        match key {
            "duration" => self.sample_duration = parse_duration(val)?,
            "path" => self.pending_buffer = Some(parse_path(val, resources)?),
            "seq" => self.pending_seq = Some(parse_event_seq(val, "time")?),
            _ => {}
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        let dur = self.sample_duration;
        if dur != self.rt_sample_duration {
            self.readers[0].reset(dur);
            self.readers[1].reset(dur);
            self.rt_sample_duration = dur;
        }
        if let Some(buf) = self.pending_buffer.take() {
            self.buffer = Some(buf);
            self.readers[0].reset(dur);
            self.readers[1].reset(dur);
        }
        if let Some(seq) = self.pending_seq.take() {
            self.seq = Some(seq);
            self.prev_event = None;
            self.next_event = None;
        }
        let valid = ctx.num_inputs() >= 1
            && self.seq.as_ref().is_some_and(|s| !s.is_empty())
            && self.buffer.is_some()
            && dur > 0.0;
        if !valid {
            out.fill(0.0);
            return;
        }
        let t = ctx.input(0)[0] as f64;
        let seq = self.seq.as_ref().unwrap();
        let should_update = (self.prev_event.is_none() && self.next_event.is_none())
            || self.prev_event.is_some_and(|p| t <= seq[p].0 + 1e-6)
            || self.next_event.is_some_and(|n| t >= seq[n].0 - 1e-6);
        if should_update || !self.readers[self.active_reader].is_aligned_with_time(t) {
            self.update_event_boundaries(t);
        }
        out.fill(0.0);
        self.readers[0].read_adding(out);
        self.readers[1].read_adding(out);
    }
}

//==============================================================================

#[derive(Clone)]
struct McBufferReader {
    fade: GainFade,
    buffer_size: usize,
    position: usize,
    sample_duration: f64,
    start_time: f64,
}

impl McBufferReader {
    fn new(sample_rate: f64, fade_ms: f64) -> Self {
        Self {
            fade: GainFade::new(sample_rate, fade_ms, fade_ms, 0.0, 0.0),
            buffer_size: 0,
            position: 0,
            sample_duration: 0.0,
            start_time: 0.0,
        }
    }

    fn engage(&mut self, start: f64, current_time: f64, buffer_size: usize) {
        self.start_time = start;
        self.buffer_size = buffer_size;
        self.fade.fade_in();
        self.position = position_for(start, current_time, self.sample_duration, buffer_size);
    }

    fn disengage(&mut self) {
        self.fade.fade_out();
    }

    fn is_aligned_with_time(&self, t: f64) -> bool {
        if !self.fade.on() {
            return true;
        }
        let new_pos = position_for(self.start_time, t, self.sample_duration, self.buffer_size);
        (self.position as i64 - new_pos as i64).abs() < 16
    }

    fn read_adding(&mut self, source: &SharedResourcePtr, ctx: &BlockContext<'_>, num_samples: usize) {
        let mut local_fade = self.fade;
        let n_ch = ctx.num_outputs().min(source.num_channels());
        for ch in 0..n_ch {
            let Some(data) = source.channel(ch) else { continue };
            local_fade = self.fade;
            let out = ctx.output(ch);
            let from = self.position.min(data.len());
            for (o, &x) in out[..num_samples].iter_mut().zip(&data[from..]) {
                *o += local_fade.tick(x);
            }
        }
        self.fade = local_fade;
        self.position += num_samples;
    }

    fn reset(&mut self, sample_duration: f64) {
        self.fade.reset();
        self.sample_duration = sample_duration;
        self.start_time = 0.0;
    }
}

/// `mc.sampleseq` / `mc.sampleseq2`: multi-channel `sampleseq`.
pub struct McSampleSeqNode {
    with_stretch: bool,
    pending_buffer: Option<SharedResourcePtr>,
    buffer: Option<SharedResourcePtr>,
    pending_seq: Option<EventSeq>,
    seq: Option<EventSeq>,
    prev_event: Option<usize>,
    next_event: Option<usize>,
    readers: [McBufferReader; 2],
    active_reader: usize,
    sample_duration: f64,
    rt_sample_duration: f64,
}

impl McSampleSeqNode {
    pub fn new(info: NodeInfo, with_stretch: bool) -> Self {
        Self {
            with_stretch,
            pending_buffer: None,
            buffer: None,
            pending_seq: None,
            seq: None,
            prev_event: None,
            next_event: None,
            readers: [
                McBufferReader::new(info.sample_rate, 8.0),
                McBufferReader::new(info.sample_rate, 8.0),
            ],
            active_reader: 0,
            sample_duration: 0.0,
            rt_sample_duration: 0.0,
        }
    }

    fn update_event_boundaries(&mut self, t: f64) {
        let (Some(seq), Some(buffer)) = (&self.seq, &self.buffer) else { return };
        let next = upper_bound(seq, t);
        if next == 0 {
            self.prev_event = None;
            self.next_event = if seq.is_empty() { None } else { Some(0) };
            self.readers[0].disengage();
            self.readers[1].disengage();
            return;
        }
        self.next_event = if next < seq.len() { Some(next) } else { None };
        self.prev_event = Some(next - 1);
        self.readers[self.active_reader].disengage();
        self.active_reader = (self.active_reader + 1) & 1;
        let (start, value) = seq[next - 1];
        if fp_equal(value, 1.0) {
            let size = buffer.num_samples();
            self.readers[self.active_reader].engage(start, t, size);
        }
    }
}

impl Node for McSampleSeqNode {
    fn set_property(&mut self, key: &str, val: &Value, resources: &mut SharedResourceMap) -> Result<(), Error> {
        if self.with_stretch {
            validate_stretch_props(key, val)?;
        }
        match key {
            "duration" => self.sample_duration = parse_duration(val)?,
            "path" => self.pending_buffer = Some(parse_path(val, resources)?),
            "seq" => {
                let seq = parse_event_seq(val, "time")?;
                if seq.is_empty() {
                    return Err(Error::InvalidPropertyValue);
                }
                self.pending_seq = Some(seq);
            }
            _ => {}
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let dur = self.sample_duration;
        if dur != self.rt_sample_duration {
            self.readers[0].reset(dur);
            self.readers[1].reset(dur);
            self.rt_sample_duration = dur;
        }
        if let Some(buf) = self.pending_buffer.take() {
            self.buffer = Some(buf);
            self.readers[0].reset(dur);
            self.readers[1].reset(dur);
        }
        if let Some(seq) = self.pending_seq.take() {
            self.seq = Some(seq);
            self.prev_event = None;
            self.next_event = None;
        }
        ctx.zero_outputs();
        let valid = ctx.num_inputs() >= 1 && self.seq.is_some() && self.buffer.is_some() && dur > 0.0;
        if !valid {
            return;
        }
        let t = ctx.input(0)[0] as f64;
        let seq = self.seq.as_ref().unwrap();
        let should_update = (self.prev_event.is_none() && self.next_event.is_none())
            || self.prev_event.is_some_and(|p| t <= seq[p].0 + 1e-6)
            || self.next_event.is_some_and(|n| t >= seq[n].0 - 1e-6);
        if should_update || !self.readers[self.active_reader].is_aligned_with_time(t) {
            self.update_event_boundaries(t);
        }
        let buffer = self.buffer.clone().unwrap();
        let n = ctx.num_samples;
        self.readers[0].read_adding(&buffer, ctx, n);
        self.readers[1].read_adding(&buffer, ctx, n);
    }
}
