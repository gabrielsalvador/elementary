//! Unary, binary and reducing arithmetic, plus the `in` identity node.

use crate::node::{expect_number, BlockContext, Node, NodeInfo};
use crate::resources::SharedResourceMap;
use crate::value::Value;
use crate::Error;

pub struct UnaryNode {
    op: fn(f32) -> f32,
}

impl UnaryNode {
    pub fn new(op: fn(f32) -> f32) -> Self {
        Self { op }
    }
}

impl Node for UnaryNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 1 {
            out.fill(0.0);
            return;
        }
        let input = ctx.input(0);
        for i in 0..ctx.num_samples {
            out[i] = (self.op)(input[i]);
        }
    }
}

/// Exactly two inputs; silence otherwise.
pub struct BinaryNode {
    op: fn(f32, f32) -> f32,
}

impl BinaryNode {
    pub fn new(op: fn(f32, f32) -> f32) -> Self {
        Self { op }
    }
}

impl Node for BinaryNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if ctx.num_inputs() < 2 {
            out.fill(0.0);
            return;
        }
        let a = ctx.input(0);
        let b = ctx.input(1);
        for i in 0..ctx.num_samples {
            out[i] = (self.op)(a[i], b[i]);
        }
    }
}

/// Folds the operator across all inputs, left to right.
pub struct ReducingNode {
    op: fn(f32, f32) -> f32,
}

impl ReducingNode {
    pub fn new(op: fn(f32, f32) -> f32) -> Self {
        Self { op }
    }
}

impl Node for ReducingNode {
    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        let n_in = ctx.num_inputs();
        if n_in < 1 {
            out.fill(0.0);
            return;
        }
        out.copy_from_slice(ctx.input(0));
        for ch in 1..n_in {
            let input = ctx.input(ch);
            for j in 0..ctx.num_samples {
                out[j] = (self.op)(out[j], input[j]);
            }
        }
    }
}

/// `in`: copies one host input channel.
pub struct IdentityNode {
    channel: i32,
}

impl IdentityNode {
    pub fn new(_: NodeInfo) -> Self {
        Self { channel: 0 }
    }
}

impl Node for IdentityNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "channel" {
            self.channel = expect_number(val)? as i32;
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        if self.channel < 0 || self.channel as usize >= ctx.num_inputs() {
            out.fill(0.0);
            return;
        }
        out.copy_from_slice(ctx.input(self.channel as usize));
    }
}

//==============================================================================
// Operators

pub fn safe_div(x: f32, y: f32) -> f32 {
    if y == 0.0 {
        0.0
    } else {
        x / y
    }
}

pub fn modulus(x: f32, y: f32) -> f32 {
    x % y
}

pub fn eq(x: f32, y: f32) -> f32 {
    ((x - y).abs() <= f32::EPSILON) as u8 as f32
}

fn is_one(x: f32) -> bool {
    (1.0 - x).abs() <= f32::EPSILON
}

pub fn and(x: f32, y: f32) -> f32 {
    (is_one(x) && is_one(y)) as u8 as f32
}

pub fn or(x: f32, y: f32) -> f32 {
    (is_one(x) || is_one(y)) as u8 as f32
}

pub fn safe_pow(x: f32, y: f32) -> f32 {
    if x < 0.0 && y != y.floor() {
        0.0
    } else {
        x.powf(y)
    }
}

pub fn le(x: f32, y: f32) -> f32 {
    (x < y) as u8 as f32
}
pub fn leq(x: f32, y: f32) -> f32 {
    (x <= y) as u8 as f32
}
pub fn ge(x: f32, y: f32) -> f32 {
    (x > y) as u8 as f32
}
pub fn geq(x: f32, y: f32) -> f32 {
    (x >= y) as u8 as f32
}
pub fn add(x: f32, y: f32) -> f32 {
    x + y
}
pub fn sub(x: f32, y: f32) -> f32 {
    x - y
}
pub fn mul(x: f32, y: f32) -> f32 {
    x * y
}
pub fn min(x: f32, y: f32) -> f32 {
    x.min(y)
}
pub fn max(x: f32, y: f32) -> f32 {
    x.max(y)
}
