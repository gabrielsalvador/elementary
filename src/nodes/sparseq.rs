//! Sparse sequencers: `sparseq` (tick-indexed, clock driven) and `sparseq2`
//! (time-indexed, driven by a time signal).

use std::collections::BTreeMap;

use crate::node::{expect_bool, expect_number, BlockContext, Change, Node, NodeInfo};
use crate::resources::SharedResourceMap;
use crate::value::Value;
use crate::Error;

use super::core::parse_offset;
use super::sampleseq::{parse_event_seq, upper_bound, EventSeq};

//==============================================================================

type TickSeq = BTreeMap<i32, f32>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct LoopPoints {
    start: i32,
    end: i32,
}

impl LoopPoints {
    const NONE: LoopPoints = LoopPoints { start: -1, end: -1 };

    fn is_none(&self) -> bool {
        self.start == -1 && self.end == -1
    }
}

enum ChangeEvent {
    NewSequence(TickSeq),
    NewLoopPoints(LoopPoints),
}

fn parse_tick_seq(val: &Value) -> Result<TickSeq, Error> {
    let items = val.as_array().ok_or(Error::InvalidPropertyType)?;
    let mut seq = TickSeq::new();
    for item in items {
        let obj = item.as_object().ok_or(Error::InvalidPropertyType)?;
        let time = obj
            .get("tickTime")
            .or_else(|| obj.get("time"))
            .and_then(Value::as_number)
            .ok_or(Error::InvalidPropertyType)? as i32;
        let value = obj
            .get("value")
            .and_then(Value::as_number)
            .ok_or(Error::InvalidPropertyType)? as f32;
        seq.entry(time).or_insert(value);
    }
    Ok(seq)
}

/// `sparseq`: inputs [clock, (reset)]; `seq` is a list of `{tickTime, value}`.
pub struct SparSeqNode {
    pending: Vec<ChangeEvent>,
    seq: Option<TickSeq>,
    change: Change,
    reset_change: Change,
    loop_points: LoopPoints,
    pending_loop_points: Option<LoopPoints>,
    offset: usize,
    follow: bool,
    edge_count: i32,
    samples_since_edge: usize,
    /// Key of the event currently held, if any.
    hold_key: Option<i32>,
    hold_order: i32,
    tick_interval: f64,
    sample_rate: f64,
}

impl SparSeqNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            pending: Vec::new(),
            seq: None,
            change: Change::default(),
            reset_change: Change::default(),
            loop_points: LoopPoints::NONE,
            pending_loop_points: None,
            offset: 0,
            follow: false,
            edge_count: -1,
            samples_since_edge: 0,
            hold_key: None,
            hold_order: 0,
            tick_interval: 0.0,
            sample_rate: info.sample_rate,
        }
    }

    fn find_tick_value(&self, tick_time: i32) -> Option<i32> {
        let seq = self.seq.as_ref()?;
        let first = *seq.keys().next()?;
        let upper = seq.range(tick_time + 1..).next().map(|(k, _)| *k);
        match upper {
            Some(k) if k == first => {
                if k == 0 {
                    Some(0)
                } else {
                    None
                }
            }
            _ => seq.range(..=tick_time).next_back().map(|(k, _)| *k),
        }
    }

    fn get_tick_time(&mut self, offset: usize) -> i32 {
        let mut tick_time = offset as i32 + self.edge_count;
        let ls = self.loop_points.start;
        let le = self.loop_points.end;
        if ls > -1 && le > -1 && tick_time >= le {
            let loop_duration = le - ls;
            if loop_duration > 0 {
                if let Some(next) = self.pending_loop_points.take() {
                    self.loop_points = next;
                    if next.is_none() {
                        return tick_time;
                    }
                    let span = next.end - next.start;
                    if span > 0 {
                        tick_time = next.start + ((tick_time - le) % span);
                    }
                } else {
                    tick_time = ls + ((tick_time - le) % loop_duration);
                }
                self.edge_count = tick_time - offset as i32;
            }
        }
        tick_time
    }
}

impl Node for SparSeqNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        match key {
            "offset" => self.offset = parse_offset(val)?,
            "loop" => {
                if val.is_null() || val.as_bool() == Some(false) {
                    self.pending.push(ChangeEvent::NewLoopPoints(LoopPoints::NONE));
                } else {
                    let points = val.as_array().ok_or(Error::InvalidPropertyType)?;
                    let start = points.first().and_then(Value::as_number).ok_or(Error::InvalidPropertyType)?;
                    let end = points.get(1).and_then(Value::as_number).ok_or(Error::InvalidPropertyType)?;
                    self.pending.push(ChangeEvent::NewLoopPoints(LoopPoints {
                        start: start as i32,
                        end: end as i32,
                    }));
                }
            }
            "follow" => self.follow = expect_bool(val)?,
            "interpolate" => self.hold_order = expect_number(val)? as i32,
            "tickInterval" => {
                let ti = expect_number(val)?;
                if ti < 0.0 {
                    return Err(Error::InvalidPropertyValue);
                }
                self.tick_interval = self.sample_rate * ti;
            }
            "seq" => self.pending.push(ChangeEvent::NewSequence(parse_tick_seq(val)?)),
            _ => {}
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        let has_reset = ctx.num_inputs() > 1;
        let offset = self.offset;
        let follow = self.follow;
        let hold_order = self.hold_order;
        let samples_per_clock = self.tick_interval;
        let mut tick_time = self.get_tick_time(offset);

        if !self.pending.is_empty() {
            for evt in std::mem::take(&mut self.pending) {
                match evt {
                    ChangeEvent::NewSequence(seq) => self.seq = Some(seq),
                    ChangeEvent::NewLoopPoints(lp) => self.pending_loop_points = Some(lp),
                }
            }
            self.hold_key = self.find_tick_value(tick_time);
        }

        if let Some(pending) = self.pending_loop_points {
            let take_immediately = self.loop_points.is_none() || !follow;
            if take_immediately {
                self.loop_points = pending;
                self.pending_loop_points = None;
                tick_time = self.get_tick_time(offset);
            }
        }

        if ctx.num_inputs() < 1 || self.seq.is_none() {
            out.fill(0.0);
            return;
        }
        let clock = ctx.input(0);
        let reset = if has_reset { Some(ctx.input(1)) } else { None };

        for i in 0..ctx.num_samples {
            self.samples_since_edge += 1;
            let r = reset.map_or(0.0, |r| r[i]);
            let is_trigger = self.change.tick(clock[i]) > 0.5;
            let is_reset = self.reset_change.tick(r) > 0.5;
            if is_reset {
                self.edge_count = 0;
            }
            if is_trigger {
                self.edge_count = if is_reset { 0 } else { self.edge_count + 1 };
                self.samples_since_edge = 0;
                tick_time = self.get_tick_time(offset);
                self.hold_key = self.find_tick_value(tick_time);
            }
            let seq = self.seq.as_ref().unwrap();
            let Some(hold_key) = self.hold_key else {
                out[i] = 0.0;
                continue;
            };
            let Some(&hold_value) = seq.get(&hold_key) else {
                out[i] = 0.0;
                continue;
            };
            out[i] = match hold_order {
                1 => match seq.range(hold_key + 1..).next() {
                    None => hold_value,
                    Some((&tr, &right_value)) => {
                        let tl = hold_key;
                        let span = (tr - tl) as f64;
                        let mut alpha = (tick_time - tl).max(0) as f64 / span;
                        if samples_per_clock > 0.0 {
                            alpha += ((self.samples_since_edge as f64).min(samples_per_clock) / samples_per_clock) / span;
                        }
                        hold_value + alpha as f32 * (right_value - hold_value)
                    }
                },
                _ => hold_value,
            };
        }
    }
}

//==============================================================================

/// `sparseq2`: input [time]; `seq` is a list of `{time, value}`; optional
/// linear interpolation between events.
pub struct SparSeq2Node {
    pending: Option<EventSeq>,
    seq: Option<EventSeq>,
    prev_event: Option<usize>,
    next_event: Option<usize>,
    interpolate: bool,
}

impl SparSeq2Node {
    pub fn new(_: NodeInfo) -> Self {
        Self {
            pending: None,
            seq: None,
            prev_event: None,
            next_event: None,
            interpolate: false,
        }
    }

    fn update_event_boundaries(&mut self, t: f64) {
        let Some(seq) = &self.seq else { return };
        let next = upper_bound(seq, t);
        self.next_event = if next < seq.len() { Some(next) } else { None };
        self.prev_event = if next == 0 { None } else { Some(next - 1) };
    }
}

impl Node for SparSeq2Node {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        match key {
            "seq" => self.pending = Some(parse_event_seq(val, "time")?),
            "interpolate" => self.interpolate = expect_number(val)? == 1.0,
            _ => {}
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if let Some(seq) = self.pending.take() {
            self.seq = Some(seq);
            self.prev_event = None;
            self.next_event = None;
        }
        let valid = ctx.num_inputs() >= 1 && self.seq.as_ref().is_some_and(|s| !s.is_empty());
        if !valid {
            out.fill(0.0);
            return;
        }
        let time = ctx.input(0);
        for i in 0..ctx.num_samples {
            let t = time[i] as f64;
            let seq = self.seq.as_ref().unwrap();
            let should_update = (self.prev_event.is_none() && self.next_event.is_none())
                || self.prev_event.is_some_and(|p| t <= seq[p].0 + 1e-9)
                || self.next_event.is_some_and(|n| t >= seq[n].0 - 1e-9);
            if should_update {
                self.update_event_boundaries(t);
            }
            let seq = self.seq.as_ref().unwrap();
            let Some(prev) = self.prev_event else {
                out[i] = 0.0;
                continue;
            };
            let (pt, pv) = seq[prev];
            let Some(next) = self.next_event else {
                out[i] = pv;
                continue;
            };
            let (nt, nv) = seq[next];
            let alpha = if self.interpolate { (t - pt) / (nt - pt) } else { 0.0 };
            out[i] = pv + alpha as f32 * (nv - pv);
        }
    }
}
