//! Uniform white noise in [0, 1).

use crate::node::{expect_number, BlockContext, Node, NodeInfo};
use crate::resources::SharedResourceMap;
use crate::value::Value;
use crate::Error;

pub struct RandNode {
    seed: u32,
}

impl RandNode {
    pub fn new(_: NodeInfo) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0x1234_5678);
        Self { seed: nanos | 1 }
    }

    #[inline]
    fn fast_rand(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(214013).wrapping_add(2531011);
        (self.seed >> 16) & 0x7FFF
    }
}

impl Node for RandNode {
    fn set_property(&mut self, key: &str, val: &Value, _: &mut SharedResourceMap) -> Result<(), Error> {
        if key == "seed" {
            self.seed = expect_number(val)? as u32;
        }
        Ok(())
    }

    fn process(&mut self, ctx: &mut BlockContext<'_>) {
        let out = ctx.output(0);
        for v in out.iter_mut() {
            *v = self.fast_rand() as f32 / 0x7FFF as f32;
        }
    }
}
