//! A pure-Rust port of the Elementary Audio runtime.
//!
//! The graph is described by [`InstructionBatch`]es: create nodes, set
//! properties, wire children, activate roots, commit. A [`Runtime`] applies
//! those batches and renders audio block by block.
//!
//! ```
//! use elementary::{InstructionBatch, NodeType, Runtime};
//!
//! let mut rt = Runtime::new(44100.0, 512);
//! let mut batch = InstructionBatch::new();
//! batch.create_node(1, NodeType::Const);
//! batch.set_property(1, "value", 0.5);
//! batch.create_node(2, NodeType::Root);
//! batch.set_property(2, "channel", 0.0);
//! batch.append_child(2, 1, 0);
//! batch.activate_roots(&[2]);
//! batch.commit();
//! rt.apply_instructions(&batch).unwrap();
//!
//! let mut left = vec![0.0f32; 512];
//! let mut right = vec![0.0f32; 512];
//! let mut output: Vec<&mut [f32]> = vec![&mut left, &mut right];
//! rt.process(&[], &mut output, 512);
//! ```

// Sample loops index several parallel buffers by position; iterators would obscure them.

mod node;
mod nodes;
mod reconcile;
mod resources;
mod runtime;
mod value;

use std::fmt;

pub use node::{BlockContext, Node, NodeId, NodeInfo, Props};
pub use nodes::analyzers::MeterReadout;
pub use nodes::{register_node_type, NodeConstructor};
pub use reconcile::{hash_node, mix, Graph, NodeRepr, Renderer, KEY_BIT};
pub use resources::{SharedResource, SharedResourceMap, SharedResourcePtr};
pub use runtime::Runtime;
pub use value::Value;

/// Whether a node of `type_name` would take `value` as its property `key`, by setting it on
/// one made for the purpose: `None` when there is no such type of node.
pub fn takes_property(type_name: &str, key: &str, value: &Value) -> Option<bool> {
    let mut node = nodes::create_node(type_name, NodeInfo { sample_rate: 48_000.0, block_size: 64 })?;
    Some(node.set_property(key, value, &mut SharedResourceMap::default()).is_ok())
}

//==============================================================================
// Error types

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    UnknownNodeType,
    NodeNotFound,
    NodeAlreadyExists,
    NodeTypeAlreadyExists,
    InvalidPropertyType,
    InvalidPropertyValue,
    InvariantViolation,
    InvalidInstructionFormat,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Error::UnknownNodeType => "Node type not recognized",
            Error::NodeNotFound => "Node not found",
            Error::NodeAlreadyExists => "Node already exists",
            Error::NodeTypeAlreadyExists => "Node type already exists",
            Error::InvalidPropertyType => "Invalid property type",
            Error::InvalidPropertyValue => "Invalid property value",
            Error::InvariantViolation => "Invariant violation",
            Error::InvalidInstructionFormat => "Invalid instruction format",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for Error {}

//==============================================================================
// Node types

/// All builtin node types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeType {
    // Core
    Const,
    Phasor,
    Sphasor,
    Sr,
    Root,
    In,

    // Math/Unary
    Sin,
    Cos,
    Tan,
    Tanh,
    Asinh,
    Ln,
    Log,
    Log2,
    Ceil,
    Floor,
    Round,
    Sqrt,
    Exp,
    Abs,

    // Binary
    Le,
    Leq,
    Ge,
    Geq,
    Pow,
    Eq,
    And,
    Or,

    // Reducing
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Min,
    Max,

    // Sequence/Control
    Seq,
    Seq2,
    Sparseq,
    Sparseq2,
    Counter,
    Accum,
    Latch,
    Maxhold,
    Once,

    // Delays
    Delay,
    Sdelay,
    Z,

    // Filters
    Pole,
    Slew,
    Env,
    Biquad,
    Prewarp,
    Mm1p,
    Svf,
    Svfshelf,

    // Feedback
    TapIn,
    TapOut,

    // Sampling/Tables
    Sample,
    Sampleseq,
    Sampleseq2,
    Table,
    McCapture,
    McSample,
    McSampleseq,
    McSampleseq2,
    McTable,

    // Oscillators
    Blepsaw,
    Blepsquare,
    Bleptriangle,

    // Analysis
    Meter,
    Scope,
    Snapshot,
    Capture,

    // Noise
    Rand,
}

impl NodeType {
    pub const ALL: [NodeType; 74] = [
        NodeType::Const,
        NodeType::Phasor,
        NodeType::Sphasor,
        NodeType::Sr,
        NodeType::Root,
        NodeType::In,
        NodeType::Sin,
        NodeType::Cos,
        NodeType::Tan,
        NodeType::Tanh,
        NodeType::Asinh,
        NodeType::Ln,
        NodeType::Log,
        NodeType::Log2,
        NodeType::Ceil,
        NodeType::Floor,
        NodeType::Round,
        NodeType::Sqrt,
        NodeType::Exp,
        NodeType::Abs,
        NodeType::Le,
        NodeType::Leq,
        NodeType::Ge,
        NodeType::Geq,
        NodeType::Pow,
        NodeType::Eq,
        NodeType::And,
        NodeType::Or,
        NodeType::Add,
        NodeType::Sub,
        NodeType::Mul,
        NodeType::Div,
        NodeType::Mod,
        NodeType::Min,
        NodeType::Max,
        NodeType::Seq,
        NodeType::Seq2,
        NodeType::Sparseq,
        NodeType::Sparseq2,
        NodeType::Counter,
        NodeType::Accum,
        NodeType::Latch,
        NodeType::Maxhold,
        NodeType::Once,
        NodeType::Delay,
        NodeType::Sdelay,
        NodeType::Z,
        NodeType::Pole,
        NodeType::Slew,
        NodeType::Env,
        NodeType::Biquad,
        NodeType::Prewarp,
        NodeType::Mm1p,
        NodeType::Svf,
        NodeType::Svfshelf,
        NodeType::TapIn,
        NodeType::TapOut,
        NodeType::Sample,
        NodeType::Sampleseq,
        NodeType::Sampleseq2,
        NodeType::Table,
        NodeType::McCapture,
        NodeType::McSample,
        NodeType::McSampleseq,
        NodeType::McSampleseq2,
        NodeType::McTable,
        NodeType::Blepsaw,
        NodeType::Blepsquare,
        NodeType::Bleptriangle,
        NodeType::Meter,
        NodeType::Scope,
        NodeType::Snapshot,
        NodeType::Capture,
        NodeType::Rand,
    ];

    /// Parse a runtime type name (e.g. `"mc.sample"`).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        NodeType::ALL.iter().copied().find(|t| t.as_str() == s)
    }

    /// The runtime type name.
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeType::Const => "const",
            NodeType::Phasor => "phasor",
            NodeType::Sphasor => "sphasor",
            NodeType::Sr => "sr",
            NodeType::Root => "root",
            NodeType::In => "in",

            NodeType::Sin => "sin",
            NodeType::Cos => "cos",
            NodeType::Tan => "tan",
            NodeType::Tanh => "tanh",
            NodeType::Asinh => "asinh",
            NodeType::Ln => "ln",
            NodeType::Log => "log",
            NodeType::Log2 => "log2",
            NodeType::Ceil => "ceil",
            NodeType::Floor => "floor",
            NodeType::Round => "round",
            NodeType::Sqrt => "sqrt",
            NodeType::Exp => "exp",
            NodeType::Abs => "abs",

            NodeType::Le => "le",
            NodeType::Leq => "leq",
            NodeType::Ge => "ge",
            NodeType::Geq => "geq",
            NodeType::Pow => "pow",
            NodeType::Eq => "eq",
            NodeType::And => "and",
            NodeType::Or => "or",

            NodeType::Add => "add",
            NodeType::Sub => "sub",
            NodeType::Mul => "mul",
            NodeType::Div => "div",
            NodeType::Mod => "mod",
            NodeType::Min => "min",
            NodeType::Max => "max",

            NodeType::Seq => "seq",
            NodeType::Seq2 => "seq2",
            NodeType::Sparseq => "sparseq",
            NodeType::Sparseq2 => "sparseq2",
            NodeType::Counter => "counter",
            NodeType::Accum => "accum",
            NodeType::Latch => "latch",
            NodeType::Maxhold => "maxhold",
            NodeType::Once => "once",

            NodeType::Delay => "delay",
            NodeType::Sdelay => "sdelay",
            NodeType::Z => "z",

            NodeType::Pole => "pole",
            NodeType::Slew => "slew",
            NodeType::Env => "env",
            NodeType::Biquad => "biquad",
            NodeType::Prewarp => "prewarp",
            NodeType::Mm1p => "mm1p",
            NodeType::Svf => "svf",
            NodeType::Svfshelf => "svfshelf",

            NodeType::TapIn => "tapIn",
            NodeType::TapOut => "tapOut",

            NodeType::Sample => "sample",
            NodeType::Sampleseq => "sampleseq",
            NodeType::Sampleseq2 => "sampleseq2",
            NodeType::Table => "table",
            NodeType::McCapture => "mc.capture",
            NodeType::McSample => "mc.sample",
            NodeType::McSampleseq => "mc.sampleseq",
            NodeType::McSampleseq2 => "mc.sampleseq2",
            NodeType::McTable => "mc.table",

            NodeType::Blepsaw => "blepsaw",
            NodeType::Blepsquare => "blepsquare",
            NodeType::Bleptriangle => "bleptriangle",

            NodeType::Meter => "meter",
            NodeType::Scope => "scope",
            NodeType::Snapshot => "snapshot",
            NodeType::Capture => "capture",

            NodeType::Rand => "rand",
        }
    }
}

//==============================================================================
// Property values

/// A value that can be set as a node property.
pub enum PropertyValue {
    Number(f64),
    String(String),
    Bool(bool),
    FloatArray(Vec<f32>),
    /// Any other shape (arrays of numbers, `{time, value}` objects, ...).
    Raw(Value),
}

impl From<f64> for PropertyValue {
    fn from(v: f64) -> Self {
        PropertyValue::Number(v)
    }
}
impl From<f32> for PropertyValue {
    fn from(v: f32) -> Self {
        PropertyValue::Number(v as f64)
    }
}
impl From<i32> for PropertyValue {
    fn from(v: i32) -> Self {
        PropertyValue::Number(v as f64)
    }
}
impl From<bool> for PropertyValue {
    fn from(v: bool) -> Self {
        PropertyValue::Bool(v)
    }
}
impl From<&str> for PropertyValue {
    fn from(v: &str) -> Self {
        PropertyValue::String(v.to_owned())
    }
}
impl From<String> for PropertyValue {
    fn from(v: String) -> Self {
        PropertyValue::String(v)
    }
}
impl From<Vec<f32>> for PropertyValue {
    fn from(v: Vec<f32>) -> Self {
        PropertyValue::FloatArray(v)
    }
}
impl From<&[f32]> for PropertyValue {
    fn from(v: &[f32]) -> Self {
        PropertyValue::FloatArray(v.to_vec())
    }
}
impl From<Vec<f64>> for PropertyValue {
    fn from(v: Vec<f64>) -> Self {
        PropertyValue::Raw(Value::Array(v.into_iter().map(Value::Number).collect()))
    }
}
impl From<Value> for PropertyValue {
    fn from(v: Value) -> Self {
        PropertyValue::Raw(v)
    }
}

impl From<PropertyValue> for Value {
    fn from(v: PropertyValue) -> Self {
        match v {
            PropertyValue::Number(n) => Value::Number(n),
            PropertyValue::String(s) => Value::String(s),
            PropertyValue::Bool(b) => Value::Bool(b),
            PropertyValue::FloatArray(a) => Value::Float32Array(a),
            PropertyValue::Raw(v) => v,
        }
    }
}

/// A sequence property as an array of `{time, value}` objects, as `sparseq2`, `sampleseq`
/// and friends consume.
pub fn seq_events(times: &[f64], values: &[f64]) -> Value {
    Value::Array(
        times
            .iter()
            .zip(values)
            .map(|(&t, &v)| Value::object([("time", Value::Number(t)), ("value", Value::Number(v))]))
            .collect(),
    )
}

//==============================================================================
// InstructionBatch

#[derive(Clone, Debug, PartialEq)]
pub enum Instruction {
    CreateNode { id: NodeId, type_name: String },
    SetProperty { id: NodeId, key: String, value: Value },
    AppendChild { parent: NodeId, child: NodeId, channel: i32 },
    ActivateRoots(Vec<NodeId>),
    Commit,
}

/// A batch of graph mutations applied together via [`Runtime::apply_instructions`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstructionBatch {
    pub(crate) instructions: Vec<Instruction>,
}

impl InstructionBatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove all instructions from the batch so it can be reused.
    pub fn clear(&mut self) {
        self.instructions.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    /// Keeps the instructions `keep` says to.
    pub fn retain(&mut self, keep: impl FnMut(&Instruction) -> bool) {
        self.instructions.retain(keep);
    }

    /// Create a new node with the given id and type.
    pub fn create_node(&mut self, node_id: NodeId, node_type: NodeType) {
        self.create_node_named(node_id, node_type.as_str());
    }

    /// Create a node by its runtime type name.
    pub fn create_node_named(&mut self, node_id: NodeId, type_name: &str) {
        self.instructions.push(Instruction::CreateNode {
            id: node_id,
            type_name: type_name.to_string(),
        });
    }

    /// Set a property on a node.
    pub fn set_property(&mut self, node_id: NodeId, key: &str, value: impl Into<PropertyValue>) {
        self.instructions.push(Instruction::SetProperty {
            id: node_id,
            key: key.to_string(),
            value: value.into().into(),
        });
    }

    /// Set a sequence property as an array of `{time, value}` objects, as
    /// consumed by `sparseq2`, `sampleseq` and friends.
    pub fn set_property_seq(&mut self, node_id: NodeId, key: &str, times: &[f64], values: &[f64]) {
        self.set_property(node_id, key, seq_events(times, values));
    }

    /// Set a `sparseq` sequence as an array of `{tickTime, value}` objects.
    pub fn set_property_tick_seq(&mut self, node_id: NodeId, key: &str, ticks: &[i32], values: &[f64]) {
        let events: Vec<Value> = ticks
            .iter()
            .zip(values)
            .map(|(&t, &v)| {
                Value::object([("tickTime", Value::Number(t as f64)), ("value", Value::Number(v))])
            })
            .collect();
        self.set_property(node_id, key, Value::Array(events));
    }

    /// Wire a child node's output channel to a parent node's next inlet.
    pub fn append_child(&mut self, parent_id: NodeId, child_id: NodeId, output_channel: i32) {
        self.instructions.push(Instruction::AppendChild {
            parent: parent_id,
            child: child_id,
            channel: output_channel,
        });
    }

    /// Activate the given root node ids, deactivating any prior roots not listed.
    pub fn activate_roots(&mut self, root_ids: &[NodeId]) {
        self.instructions.push(Instruction::ActivateRoots(root_ids.to_vec()));
    }

    /// End the batch. Triggers a render sequence rebuild if roots were activated.
    pub fn commit(&mut self) {
        self.instructions.push(Instruction::Commit);
    }
}

#[cfg(test)]
mod tests;
