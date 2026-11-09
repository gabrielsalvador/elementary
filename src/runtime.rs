//! The graph host: applies instruction batches, builds the render sequence,
//! and runs the per-block processing loop.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::node::{BlockContext, Node, NodeId, NodeInfo, Props};
use crate::nodes::analyzers::MeterReadout;
use crate::nodes::create_node;
use crate::resources::{SharedResource, SharedResourceMap};
use crate::value::Value;
use crate::{Error, Instruction, InstructionBatch};

#[derive(Clone, Copy, Debug)]
struct Inlet {
    source: NodeId,
    channel: usize,
}

#[derive(Clone, Copy, Debug)]
struct Outlet {
    destination: NodeId,
    channel: usize,
}

struct GraphEntry {
    node: Box<dyn Node>,
    type_name: String,
    props: Props,
    inlets: Vec<Inlet>,
    outlets: Vec<Outlet>,
}

/// One node's processing step within a root's subsequence.
struct RenderOp {
    node: NodeId,
    /// `None`: a leaf node reading the host input channels.
    inputs: Option<Vec<usize>>,
    outputs: Vec<usize>,
}

struct RootSequence {
    root: NodeId,
    root_slot: usize,
    active_at_build: bool,
    ops: Vec<RenderOp>,
}

struct RenderSequence {
    arena: Vec<f32>,
    roots: Vec<RootSequence>,
    live_nodes: HashSet<NodeId>,
}

/// Raw host input pointers, only populated for the duration of a `process` call.
#[derive(Default)]
struct HostInputs(Vec<*const f32>);

// SAFETY: the vector is always empty between `process` calls, so no pointer
// ever crosses a thread boundary.
unsafe impl Send for HostInputs {}

/// A pure-Rust port of the Elementary Audio runtime.
///
/// Create one per audio context, feed it [`InstructionBatch`]es to build the
/// graph, and call [`process`](Self::process) from the audio callback. All
/// methods take `&mut self`; the host serializes access.
pub struct Runtime {
    info: NodeInfo,
    nodes: HashMap<NodeId, GraphEntry>,
    current_roots: BTreeSet<NodeId>,
    resources: SharedResourceMap,
    sequence: Option<RenderSequence>,
    host_inputs: HostInputs,
}

impl Runtime {
    /// Create a runtime. `block_size` is the largest chunk rendered in one
    /// pass; longer `process` calls are split into `block_size` chunks.
    pub fn new(sample_rate: f64, block_size: i32) -> Self {
        Self {
            info: NodeInfo {
                sample_rate,
                block_size: block_size.max(1) as usize,
            },
            nodes: HashMap::new(),
            current_roots: BTreeSet::new(),
            resources: SharedResourceMap::default(),
            sequence: None,
            host_inputs: HostInputs::default(),
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.info.sample_rate
    }

    pub fn block_size(&self) -> usize {
        self.info.block_size
    }

    //==========================================================================
    // Instructions

    /// Apply a batch of graph instructions. Stops at the first failing
    /// instruction; earlier instructions in the batch remain applied.
    pub fn apply_instructions(&mut self, batch: &InstructionBatch) -> Result<(), Error> {
        let mut should_rebuild = false;
        for instr in &batch.instructions {
            match instr {
                Instruction::CreateNode { id, type_name } => self.create_node(*id, type_name)?,
                Instruction::SetProperty { id, key, value } => self.set_property(*id, key, value)?,
                Instruction::AppendChild {
                    parent,
                    child,
                    channel,
                } => self.append_child(*parent, *child, *channel)?,
                Instruction::ActivateRoots(ids) => {
                    self.activate_roots(ids)?;
                    should_rebuild = true;
                }
                Instruction::Commit => {
                    if should_rebuild {
                        self.sequence = Some(self.build_render_sequence());
                    }
                }
            }
        }
        Ok(())
    }

    fn create_node(&mut self, id: NodeId, type_name: &str) -> Result<(), Error> {
        let node = create_node(type_name, self.info).ok_or(Error::UnknownNodeType)?;
        if let Some(entry) = self.nodes.get_mut(&id) {
            if entry.type_name != type_name {
                return Err(Error::NodeAlreadyExists);
            }
            // A renderer creating a node of the same type again, one it forgot while this
            // runtime kept it under a fading root, or one keyed to live on while its inputs
            // move: the node and its state stay, and the batch wires it afresh.
            let inlets = std::mem::take(&mut entry.inlets);
            for inlet in inlets {
                if let Some(child) = self.nodes.get_mut(&inlet.source) {
                    child.outlets.retain(|o| o.destination != id);
                }
            }
            return Ok(());
        }
        self.nodes.insert(
            id,
            GraphEntry {
                node,
                type_name: type_name.to_string(),
                props: Props::new(),
                inlets: Vec::new(),
                outlets: Vec::new(),
            },
        );
        Ok(())
    }

    fn set_property(&mut self, id: NodeId, key: &str, value: &Value) -> Result<(), Error> {
        let entry = self.nodes.get_mut(&id).ok_or(Error::NodeNotFound)?;
        entry.node.set_property(key, value, &mut self.resources)?;
        entry.props.insert(key.to_string(), value.clone());
        Ok(())
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId, channel: i32) -> Result<(), Error> {
        if !self.nodes.contains_key(&parent) || !self.nodes.contains_key(&child) {
            return Err(Error::NodeNotFound);
        }
        let channel = channel.max(0) as usize;
        self.nodes
            .get_mut(&parent)
            .unwrap()
            .inlets
            .push(Inlet { source: child, channel });
        self.nodes.get_mut(&child).unwrap().outlets.push(Outlet {
            destination: parent,
            channel,
        });
        Ok(())
    }

    fn activate_roots(&mut self, ids: &[NodeId]) -> Result<(), Error> {
        let mut active = BTreeSet::new();
        for &id in ids {
            let entry = self.nodes.get_mut(&id).ok_or(Error::NodeNotFound)?;
            if entry.node.as_root().is_some() {
                entry.node.set_property("active", &Value::Bool(true), &mut self.resources)?;
                entry.props.insert("active".into(), Value::Bool(true));
                active.insert(id);
            }
        }
        // Deactivate roots that dropped out of the set, but keep tracking them
        // while they fade out.
        for &id in &self.current_roots {
            let Some(entry) = self.nodes.get_mut(&id) else { continue };
            if entry.node.as_root().is_none() {
                continue;
            }
            if !active.contains(&id) {
                entry.node.set_property("active", &Value::Bool(false), &mut self.resources)?;
                entry.props.insert("active".into(), Value::Bool(false));
            }
            if entry.node.as_root().is_some_and(|r| r.still_running()) {
                active.insert(id);
            }
        }
        self.current_roots = active;
        Ok(())
    }

    //==========================================================================
    // Render sequence

    /// Post-order traversal from `start`, skipping already-visited nodes.
    fn traverse(&self, visited: &mut HashSet<NodeId>, order: &mut Vec<NodeId>, start: NodeId) {
        if visited.contains(&start) || !self.nodes.contains_key(&start) {
            return;
        }
        let mut stack: Vec<(NodeId, usize)> = vec![(start, 0)];
        let mut on_stack: HashSet<NodeId> = HashSet::from([start]);
        while let Some(&mut (id, ref mut next_child)) = stack.last_mut() {
            let inlets = &self.nodes[&id].inlets;
            if *next_child < inlets.len() {
                let child = inlets[*next_child].source;
                *next_child += 1;
                if !visited.contains(&child) && !on_stack.contains(&child) && self.nodes.contains_key(&child) {
                    stack.push((child, 0));
                    on_stack.insert(child);
                }
            } else {
                stack.pop();
                on_stack.remove(&id);
                visited.insert(id);
                order.push(id);
            }
        }
    }

    fn build_render_sequence(&mut self) -> RenderSequence {
        // Active roots first so that shared nodes are owned by a live root's
        // subsequence, then inactive (fading) roots.
        let mut sorted_roots: Vec<NodeId> = Vec::new();
        for &id in &self.current_roots {
            let Some(entry) = self.nodes.get(&id) else { continue };
            let Some(root) = entry.node.as_root() else { continue };
            if root.active() {
                sorted_roots.insert(0, id);
            } else {
                sorted_roots.push(id);
            }
        }

        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut buffer_map: HashMap<(NodeId, usize), usize> = HashMap::new();
        let mut next_slot = 0usize;
        let mut roots = Vec::new();
        let mut live_nodes = HashSet::new();

        for root_id in sorted_roots {
            let active = self.nodes[&root_id].node.as_root().is_some_and(|r| r.active());
            let mut order = Vec::new();
            self.traverse(&mut visited, &mut order, root_id);

            let mut ops = Vec::with_capacity(order.len());
            for id in order {
                live_nodes.insert(id);
                let num_outs = self.nodes[&id]
                    .outlets
                    .iter()
                    .map(|o| o.channel + 1)
                    .max()
                    .unwrap_or(1)
                    .max(1);
                let outputs: Vec<usize> = (0..num_outs)
                    .map(|ch| {
                        *buffer_map.entry((id, ch)).or_insert_with(|| {
                            next_slot += 1;
                            next_slot - 1
                        })
                    })
                    .collect();

                let inlets = self.nodes[&id].inlets.clone();
                let inputs = if inlets.is_empty() {
                    None
                } else {
                    let count = inlets.len();
                    let entry = self.nodes.get_mut(&id).unwrap();
                    let v = Value::Number(count as f64);
                    if entry.node.set_property("_internal:numChildren", &v, &mut self.resources).is_ok() {
                        entry.props.insert("_internal:numChildren".into(), v);
                    }
                    Some(
                        inlets
                            .iter()
                            .map(|inlet| {
                                *buffer_map.entry((inlet.source, inlet.channel)).or_insert_with(|| {
                                    next_slot += 1;
                                    next_slot - 1
                                })
                            })
                            .collect(),
                    )
                };
                ops.push(RenderOp { node: id, inputs, outputs });
            }

            let root_slot = buffer_map[&(root_id, 0)];
            roots.push(RootSequence {
                root: root_id,
                root_slot,
                active_at_build: active,
                ops,
            });
        }

        RenderSequence {
            arena: vec![0.0; next_slot.max(1) * self.info.block_size],
            roots,
            live_nodes,
        }
    }

    //==========================================================================
    // Processing

    /// Render `num_samples` frames. Output channels are cleared first and each
    /// active root sums into the channel it targets.
    pub fn process(&mut self, input: &[&[f32]], output: &mut [&mut [f32]], num_samples: usize) {
        self.host_inputs.0.clear();
        self.host_inputs.0.extend(input.iter().map(|ch| ch.as_ptr()));
        let mut out_ptrs: Vec<*mut f32> = output.iter_mut().map(|ch| ch.as_mut_ptr()).collect();
        // SAFETY: pointers derive from live slices at least `num_samples` long.
        unsafe { self.process_ptrs(out_ptrs.as_mut_slice(), num_samples) };
        self.host_inputs.0.clear();
    }

    /// Render from raw channel pointers, for audio callbacks that already hold them.
    ///
    /// # Safety
    /// `input` must point to `num_inputs` valid channel pointers and `output`
    /// to `num_outputs` valid channel pointers, each at least `num_samples` long.
    pub unsafe fn process_raw(
        &mut self,
        input: *const *const f32,
        num_inputs: usize,
        output: *mut *mut f32,
        num_outputs: usize,
        num_samples: usize,
        _user_data: *mut std::ffi::c_void,
    ) {
        self.host_inputs.0.clear();
        if num_inputs > 0 && !input.is_null() {
            self.host_inputs
                .0
                .extend_from_slice(std::slice::from_raw_parts(input, num_inputs));
        }
        let outputs: &mut [*mut f32] = if num_outputs > 0 && !output.is_null() {
            std::slice::from_raw_parts_mut(output, num_outputs)
        } else {
            &mut []
        };
        self.process_ptrs(outputs, num_samples);
        self.host_inputs.0.clear();
    }

    unsafe fn process_ptrs(&mut self, outputs: &mut [*mut f32], num_samples: usize) {
        for ch in outputs.iter() {
            std::ptr::write_bytes(*ch, 0, num_samples);
        }
        let Some(seq) = &mut self.sequence else { return };
        let block = self.info.block_size;
        let mut offset = 0;
        while offset < num_samples {
            let n = (num_samples - offset).min(block);
            Self::process_block(&mut self.nodes, seq, &self.host_inputs.0, outputs, offset, n, block);
            offset += n;
        }
    }

    unsafe fn process_block(
        nodes: &mut HashMap<NodeId, GraphEntry>,
        seq: &mut RenderSequence,
        host_inputs: &[*const f32],
        outputs: &mut [*mut f32],
        offset: usize,
        n: usize,
        block: usize,
    ) {
        let arena = seq.arena.as_mut_ptr();
        for rs in &seq.roots {
            let Some(entry) = nodes.get(&rs.root) else { continue };
            let Some(root) = entry.node.as_root() else { continue };
            if !root.still_running() {
                continue;
            }
            let chan = root.channel();
            if chan < 0 || chan as usize >= outputs.len() {
                continue;
            }
            for op in &rs.ops {
                let Some(entry) = nodes.get_mut(&op.node) else { continue };
                let mut ctx = BlockContext::new(
                    arena,
                    block,
                    host_inputs,
                    offset,
                    op.inputs.as_deref(),
                    &op.outputs,
                    n,
                    rs.active_at_build,
                );
                entry.node.process(&mut ctx);
            }
            let src = std::slice::from_raw_parts(arena.add(rs.root_slot * block), n);
            let dst = std::slice::from_raw_parts_mut(outputs[chan as usize].add(offset), n);
            for (d, s) in dst.iter_mut().zip(src) {
                *d += *s;
            }
        }
        // Publish tap buffers after all roots ran, so feedback reads happen
        // before writes within a block.
        for rs in &seq.roots {
            let active = nodes
                .get(&rs.root)
                .and_then(|e| e.node.as_root())
                .is_some_and(|r| r.active());
            if !active {
                continue;
            }
            for op in &rs.ops {
                if let Some(entry) = nodes.get_mut(&op.node) {
                    if let Some(tap) = entry.node.as_tap_out() {
                        tap.promote_tap_buffers(n);
                    }
                }
            }
        }
    }

    //==========================================================================
    // Events, reset, gc

    /// Relay pending analysis events (`meter`, `scope`, `snapshot`, `capture`...).
    /// The callback receives the event type and the event payload as JSON.
    pub fn process_queued_events<F: FnMut(&str, &str)>(&mut self, mut callback: F) {
        let Some(seq) = &self.sequence else { return };
        let mut json = String::new();
        for rs in &seq.roots {
            let active = self
                .nodes
                .get(&rs.root)
                .and_then(|e| e.node.as_root())
                .is_some_and(|r| r.active());
            if !active {
                continue;
            }
            for op in &rs.ops {
                let Some(entry) = self.nodes.get_mut(&op.node) else { continue };
                let props = &entry.props;
                entry.node.process_events(props, &mut |event_type, value| {
                    json.clear();
                    value.write_json(&mut json);
                    callback(event_type, &json);
                });
            }
        }
    }

    /// Hand every `meter` node's latest readout to `f`, with the node's `name`
    /// property. Unlike [`process_queued_events`](Self::process_queued_events)
    /// this never allocates, so it is safe in the audio callback; readouts
    /// taken here are not emitted as events.
    pub fn read_meters<F: FnMut(Option<&Value>, MeterReadout)>(&mut self, mut f: F) {
        let Some(seq) = &self.sequence else { return };
        for rs in &seq.roots {
            for op in &rs.ops {
                let Some(entry) = self.nodes.get_mut(&op.node) else { continue };
                let Some(meter) = entry.node.as_meter() else { continue };
                // Nodes shared between roots appear in each root's ops; the
                // first visit takes the readout.
                if let Some(readout) = meter.take_readout() {
                    f(entry.props.get("name"), readout);
                }
            }
        }
    }

    /// Reset all graph node internal state (delay lines, sample readers, etc.).
    pub fn reset(&mut self) {
        for entry in self.nodes.values_mut() {
            entry.node.reset();
        }
    }

    /// Drop nodes that are not part of the current render sequence. Returns
    /// the pruned node ids in ascending order.
    pub fn gc(&mut self) -> Vec<NodeId> {
        let live: HashSet<NodeId> = self
            .sequence
            .as_ref()
            .map(|s| s.live_nodes.clone())
            .unwrap_or_default();
        let mut pruned: Vec<NodeId> = self.nodes.keys().filter(|id| !live.contains(id)).copied().collect();
        pruned.sort_unstable();
        for id in &pruned {
            let Some(entry) = self.nodes.remove(id) else { continue };
            for inlet in entry.inlets {
                if let Some(child) = self.nodes.get_mut(&inlet.source) {
                    child.outlets.retain(|o| o.destination != *id);
                }
            }
        }
        self.current_roots.retain(|id| self.nodes.contains_key(id));
        pruned
    }

    //==========================================================================
    // Shared resources

    /// Load a mono buffer under `name`. Returns false if the name is taken.
    pub fn add_shared_resource(&mut self, name: &str, data: &[f32]) -> bool {
        self.resources.add(name, SharedResource::mono(data))
    }

    /// Load a multi-channel buffer under `name`. Returns false if the name is taken.
    pub fn add_shared_resource_multi(&mut self, name: &str, channels: &[&[f32]]) -> bool {
        self.resources.add(name, SharedResource::multi(channels))
    }

    /// Drop shared resources no node references.
    pub fn prune_shared_resources(&mut self) {
        self.resources.prune();
    }

    /// Names of the loaded shared resources.
    pub fn shared_resource_names(&self) -> Vec<String> {
        self.resources.keys().map(str::to_string).collect()
    }

    //==========================================================================
    // Introspection

    /// JSON snapshot of every node: type, properties and inlet connections,
    /// keyed by the node id in `0x%08x` form.
    pub fn snapshot(&mut self) -> String {
        let mut ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        ids.sort_unstable();
        let mut root = std::collections::BTreeMap::new();
        for id in ids {
            let entry = &self.nodes[&id];
            let inlets: Vec<Value> = entry
                .inlets
                .iter()
                .map(|i| {
                    Value::object([
                        ("source", Value::Number(i.source as f64)),
                        ("channel", Value::Number(i.channel as f64)),
                    ])
                })
                .collect();
            root.insert(
                format!("0x{:08x}", id as u32),
                Value::object([
                    ("type", Value::String(entry.type_name.clone())),
                    ("properties", Value::Object(entry.props.clone())),
                    ("inlets", Value::Array(inlets)),
                ]),
            );
        }
        Value::Object(root).to_json()
    }

    /// Ids of the nodes currently in the graph table.
    pub fn node_ids(&self) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        ids.sort_unstable();
        ids
    }
}
