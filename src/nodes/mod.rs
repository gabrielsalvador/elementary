//! Builtin node types and the factory that maps type names to constructors.

pub mod analyzers;
pub mod core;
pub mod delays;
pub mod feedback;
pub mod filters;
pub mod math;
pub mod noise;
pub mod oscillators;
pub mod sample;
pub mod sampleseq;
pub mod sparseq;

use crate::node::{Node, NodeInfo};

use self::oscillators::BlepMode;

/// Instantiate a builtin node by its type name. Returns `None` for unknown types.
pub fn create_node(type_name: &str, info: NodeInfo) -> Option<Box<dyn Node>> {
    use math::{BinaryNode as Bin, ReducingNode as Red, UnaryNode as Un};

    let node: Box<dyn Node> = match type_name {
        // Core
        "root" => Box::new(core::RootNode::new(info)),
        "const" => Box::new(core::ConstNode::new(info)),
        "phasor" => Box::new(core::PhasorNode::<false>::new(info)),
        "sphasor" => Box::new(core::PhasorNode::<true>::new(info)),
        "sr" => Box::new(core::SampleRateNode::new(info)),
        "in" => Box::new(math::IdentityNode::new(info)),
        "counter" => Box::new(core::CounterNode::new(info)),
        "accum" => Box::new(core::AccumNode::new(info)),
        "latch" => Box::new(core::LatchNode::new(info)),
        "maxhold" => Box::new(core::MaxHoldNode::new(info)),
        "once" => Box::new(core::OnceNode::new(info)),
        "seq" => Box::new(core::SequenceNode::new(info)),
        "seq2" => Box::new(core::Seq2Node::new(info)),
        "sparseq" => Box::new(sparseq::SparSeqNode::new(info)),
        "sparseq2" => Box::new(sparseq::SparSeq2Node::new(info)),

        // Unary math
        "sin" => Box::new(Un::new(f32::sin)),
        "cos" => Box::new(Un::new(f32::cos)),
        "tan" => Box::new(Un::new(f32::tan)),
        "tanh" => Box::new(Un::new(f32::tanh)),
        "asinh" => Box::new(Un::new(f32::asinh)),
        "ln" => Box::new(Un::new(f32::ln)),
        "log" => Box::new(Un::new(f32::log10)),
        "log2" => Box::new(Un::new(f32::log2)),
        "ceil" => Box::new(Un::new(f32::ceil)),
        "floor" => Box::new(Un::new(f32::floor)),
        "round" => Box::new(Un::new(f32::round)),
        "sqrt" => Box::new(Un::new(f32::sqrt)),
        "exp" => Box::new(Un::new(f32::exp)),
        "abs" => Box::new(Un::new(f32::abs)),

        // Binary
        "le" => Box::new(Bin::new(math::le)),
        "leq" => Box::new(Bin::new(math::leq)),
        "ge" => Box::new(Bin::new(math::ge)),
        "geq" => Box::new(Bin::new(math::geq)),
        "pow" => Box::new(Bin::new(math::safe_pow)),
        "eq" => Box::new(Bin::new(math::eq)),
        "and" => Box::new(Bin::new(math::and)),
        "or" => Box::new(Bin::new(math::or)),

        // Reducing
        "add" => Box::new(Red::new(math::add)),
        "sub" => Box::new(Red::new(math::sub)),
        "mul" => Box::new(Red::new(math::mul)),
        "div" => Box::new(Red::new(math::safe_div)),
        "mod" => Box::new(Red::new(math::modulus)),
        "min" => Box::new(Red::new(math::min)),
        "max" => Box::new(Red::new(math::max)),

        // Delays
        "delay" => Box::new(delays::VariableDelayNode::new(info)),
        "sdelay" => Box::new(delays::SampleDelayNode::new(info)),
        "z" => Box::new(delays::SingleSampleDelayNode::new(info)),

        // Filters
        "pole" => Box::new(filters::OnePoleNode::new(info)),
        "slew" => Box::new(filters::SlewNode::new(info)),
        "env" => Box::new(filters::EnvelopeNode::new(info)),
        "biquad" => Box::new(filters::BiquadNode::new(info)),
        "prewarp" => Box::new(filters::PrewarpNode::new(info)),
        "mm1p" => Box::new(filters::MultiMode1pNode::new(info)),
        "svf" => Box::new(filters::SvfNode::new(info)),
        "svfshelf" => Box::new(filters::SvfShelfNode::new(info)),

        // Feedback
        "tapIn" => Box::new(feedback::TapInNode::new(info)),
        "tapOut" => Box::new(feedback::TapOutNode::new(info)),

        // Sampling / tables
        "sample" => Box::new(sample::SampleNode::new(info)),
        "sampleseq" => Box::new(sampleseq::SampleSeqNode::new(info, false)),
        "sampleseq2" => Box::new(sampleseq::SampleSeqNode::new(info, true)),
        "table" => Box::new(sample::TableNode::new(info)),
        "mc.capture" => Box::new(analyzers::McCaptureNode::new(info)),
        "mc.sample" => Box::new(sample::McSampleNode::new(info)),
        "mc.sampleseq" => Box::new(sampleseq::McSampleSeqNode::new(info, false)),
        "mc.sampleseq2" => Box::new(sampleseq::McSampleSeqNode::new(info, true)),
        "mc.table" => Box::new(sample::McTableNode::new(info)),

        // Oscillators
        "blepsaw" => Box::new(oscillators::PolyBlepNode::new(info, BlepMode::Saw)),
        "blepsquare" => Box::new(oscillators::PolyBlepNode::new(info, BlepMode::Square)),
        "bleptriangle" => Box::new(oscillators::PolyBlepNode::new(info, BlepMode::Triangle)),

        // Analysis
        "meter" => Box::new(analyzers::MeterNode::new(info)),
        "scope" => Box::new(analyzers::ScopeNode::new(info)),
        "snapshot" => Box::new(analyzers::SnapshotNode::new(info)),
        "capture" => Box::new(analyzers::CaptureNode::new(info)),

        // Noise
        "rand" => Box::new(noise::RandNode::new(info)),

        _ => return None,
    };
    Some(node)
}
