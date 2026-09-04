//! `tapIn` / `tapOut`: one-block feedback through a named shared buffer.

use crate::node::{expect_string, BlockContext, Node, NodeInfo};
use crate::resources::{SharedResource, SharedResourceMap, SharedResourcePtr};
use crate::value::Value;
use crate::Error;

pub struct TapInNode {
    buffer: Option<SharedResourcePtr>,
    block_size: usize,
}

impl TapInNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            buffer: None,
            block_size: info.block_size,
        }
    }
}

impl Node for TapInNode {
    fn set_property(&mut self, key: &str, val: &Value, resources: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "name" {
            let name = expect_string(val)?;
            self.buffer = Some(resources.get_tap(name, self.block_size));
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        out.fill(0.0);
        let Some(res) = &self.buffer else { return };
        match res.as_ref() {
            SharedResource::Tap(buf) => {
                if let Ok(buf) = buf.lock() {
                    let n = out.len().min(buf.len());
                    out[..n].copy_from_slice(&buf[..n]);
                }
            }
            SharedResource::Audio { .. } => {
                if let Some(ch) = res.channel(0) {
                    let n = out.len().min(ch.len());
                    out[..n].copy_from_slice(&ch[..n]);
                }
            }
        }
    }
}

pub struct TapOutNode {
    delay_buffer: Vec<f32>,
    tap: Option<SharedResourcePtr>,
    block_size: usize,
}

impl TapOutNode {
    pub fn new(info: NodeInfo) -> Self {
        Self {
            delay_buffer: vec![0.0; info.block_size],
            tap: None,
            block_size: info.block_size,
        }
    }

    /// Publish the block captured by `process` into the shared tap buffer.
    pub fn promote_tap_buffers(&mut self, num_samples: usize) {
        let Some(res) = &self.tap else { return };
        if let SharedResource::Tap(buf) = res.as_ref() {
            if let Ok(mut buf) = buf.lock() {
                let n = num_samples.min(buf.len()).min(self.delay_buffer.len());
                buf[..n].copy_from_slice(&self.delay_buffer[..n]);
            }
        }
    }
}

impl Node for TapOutNode {
    fn set_property(&mut self, key: &str, val: &Value, resources: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "name" {
            let name = expect_string(val)?;
            self.tap = Some(resources.get_tap(name, self.block_size));
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        let n = ctx.num_samples;
        if ctx.num_inputs() < 1 || n > self.delay_buffer.len() {
            out.fill(0.0);
            return;
        }
        let input = ctx.input(0);
        self.delay_buffer[..n].copy_from_slice(input);
        out.copy_from_slice(input);
    }

    fn as_tap_out(&mut self) -> Option<&mut TapOutNode> {
        Some(self)
    }
}
