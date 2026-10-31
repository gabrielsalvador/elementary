//! The renderer half of Elementary: a graph of nodes hashed by their content, reconciled
//! against what the runtime already holds, so each render sends only what changed and
//! every untouched node keeps playing with its state.
//!
//! A port of `@elemaudio/core`'s `NodeRepr`, `Hash` and `Reconciler` modules. A node's id
//! is a hash of its kind, its props and its children, so the same subtree hashes the same
//! on every render and is created once. A *keyed* node hashes to its key alone: its props
//! may change, and here its children too, while the runtime keeps the node and its state.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::node::{NodeId, Props};
use crate::value::Value;
use crate::{Instruction, InstructionBatch, NodeType};

/// Set in every key and clear in every hash, so the two can never collide.
pub const KEY_BIT: NodeId = 1 << 30;

const FNV_OFFSET: NodeId = 0x811c_9dc5_u32 as NodeId;
const FNV_PRIME: NodeId = 0x0100_0193;

/// One FNV-1a step on signed 32-bit integers, as upstream's `mixNumber`.
pub fn mix(seed: NodeId, n: NodeId) -> NodeId {
    (seed ^ n).wrapping_mul(FNV_PRIME)
}

fn hash_str(seed: NodeId, s: &str) -> NodeId {
    // Upstream folds in every UTF-16 unit and then one past the end, a NaN read as 0.
    mix(s.encode_utf16().fold(seed, |r, c| mix(r, c as NodeId)), 0)
}

/// What upstream's `hashNode` works out from a kind, its props and its children's hashes.
pub fn hash_node(kind: &str, props: &Props, children: impl IntoIterator<Item = NodeId>) -> NodeId {
    let mut r = hash_str(FNV_OFFSET, kind);
    for (key, value) in props {
        r = hash_str(r, key);
        r = hash_str(r, &value.to_json());
    }
    children.into_iter().fold(r, mix) & (KEY_BIT - 1)
}

/// A node as the renderer sees it: shared by every parent that uses it, hashed once at
/// creation.
#[derive(Debug, Clone)]
pub struct NodeRepr {
    pub hash: NodeId,
    pub kind: String,
    pub props: Props,
    /// Which of the node's outputs a parent holding this handle reads.
    pub output_channel: i32,
    pub children: Vec<Rc<NodeRepr>>,
}

/// A handle to a node in a graph under construction.
pub type Graph = Rc<NodeRepr>;

impl NodeRepr {
    pub fn new(kind: NodeType, props: Props, children: Vec<Graph>) -> Graph {
        Self::named(kind.as_str(), props, children)
    }

    /// A node of a registered type by name, as a native module's.
    pub fn named(kind: &str, props: Props, children: Vec<Graph>) -> Graph {
        let hash = hash_node(
            kind,
            &props,
            children.iter().map(|c| mix(c.hash, c.output_channel)),
        );
        Rc::new(NodeRepr {
            hash,
            kind: kind.to_string(),
            props,
            output_channel: 0,
            children,
        })
    }

    /// A node found by `key` on every render, whatever its props or children, so a value
    /// set on it in place survives and so it keeps its state while its inputs move.
    pub fn keyed(key: NodeId, kind: NodeType, props: Props, children: Vec<Graph>) -> Graph {
        assert!(key & KEY_BIT != 0, "a key carries KEY_BIT");
        Rc::new(NodeRepr {
            hash: key,
            kind: kind.as_str().to_string(),
            props,
            output_channel: 0,
            children,
        })
    }

    /// The same node read at output `channel`.
    pub fn channel(self: &Graph, channel: i32) -> Graph {
        Rc::new(NodeRepr {
            output_channel: channel,
            ..(**self).clone()
        })
    }
}

/// A node as last sent to the runtime.
#[derive(Debug, Clone, PartialEq)]
struct Mounted {
    kind: String,
    props: Props,
    children: Vec<(NodeId, i32)>,
}

/// Holds what the runtime has been sent, and turns each graph rendered into the
/// instructions that take the runtime from the last one to it.
#[derive(Debug)]
pub struct Renderer {
    nodes: HashMap<NodeId, Mounted>,
    fade_in_ms: f64,
    fade_out_ms: f64,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// Roots fade in and out over 20 ms, as upstream's do.
    pub fn new() -> Self {
        Self::with_fades(20.0, 20.0)
    }

    pub fn with_fades(fade_in_ms: f64, fade_out_ms: f64) -> Self {
        Self {
            nodes: HashMap::new(),
            fade_in_ms,
            fade_out_ms,
        }
    }

    /// Whether the runtime holds a node of this hash.
    pub fn has(&self, hash: NodeId) -> bool {
        self.nodes.contains_key(&hash)
    }

    /// The instructions that play `graphs`, one per output channel, in place of whatever
    /// played before: new nodes are created, keyed nodes take their new props and
    /// children, and roots that dropped out fade away. No graphs plays silence.
    pub fn render(&mut self, graphs: &[Graph]) -> InstructionBatch {
        let roots: Vec<Graph> = graphs
            .iter()
            .enumerate()
            .map(|(channel, graph)| {
                let props = Props::from([
                    ("channel".to_string(), Value::Number(channel as f64)),
                    ("fadeInMs".to_string(), Value::Number(self.fade_in_ms)),
                    ("fadeOutMs".to_string(), Value::Number(self.fade_out_ms)),
                ]);
                NodeRepr::new(NodeType::Root, props, vec![graph.clone()])
            })
            .collect();

        // Pre-order, each node once. Every node is created before any is appended to, as
        // upstream's delegate promises, by keeping the two apart until the end.
        let mut created = InstructionBatch::new();
        let mut appended = InstructionBatch::new();
        let mut visited = HashSet::new();
        let mut pending: Vec<Graph> = roots.iter().rev().cloned().collect();
        while let Some(node) = pending.pop() {
            if !visited.insert(node.hash) {
                continue;
            }
            self.mount(&node, &mut created, &mut appended);
            pending.extend(node.children.iter().rev().cloned());
        }

        // Upstream forgets a node when the runtime's garbage collector reports it gone.
        // Here the runtime runs on the audio thread and its word would arrive too late for
        // the next render, so the renderer forgets whatever this render didn't reach. The
        // runtime may still hold such a node while an old root fades; asked to create it
        // again, it keeps the node and its state and wires it afresh.
        self.nodes.retain(|hash, _| visited.contains(hash));

        created.instructions.extend(appended.instructions);
        created.activate_roots(&roots.iter().map(|r| r.hash).collect::<Vec<_>>());
        created.commit();
        created
    }

    fn mount(
        &mut self,
        node: &NodeRepr,
        created: &mut InstructionBatch,
        appended: &mut InstructionBatch,
    ) {
        let children: Vec<(NodeId, i32)> = node
            .children
            .iter()
            .map(|c| (c.hash, c.output_channel))
            .collect();
        if let Some(mounted) = self.nodes.get_mut(&node.hash) {
            if mounted.children == children {
                update_props(created, node.hash, &mut mounted.props, &node.props);
                return;
            }
        }
        // New, or a keyed node whose inputs moved: created again, the runtime keeps it and
        // takes the new wiring.
        let mut props = self
            .nodes
            .remove(&node.hash)
            .map(|m| m.props)
            .unwrap_or_default();
        created.create_node_named(node.hash, &node.kind);
        update_props(created, node.hash, &mut props, &node.props);
        for &(child, channel) in &children {
            appended.append_child(node.hash, child, channel);
        }
        self.nodes.insert(
            node.hash,
            Mounted {
                kind: node.kind.clone(),
                props,
                children,
            },
        );
    }

    /// Drops from `batch` what addresses a node the runtime no longer has, as a value set
    /// on a module nothing reaches: the runtime stops a batch at the first such node, and
    /// would lose what follows.
    pub fn retain_known(&self, batch: &mut InstructionBatch) {
        batch.retain(|instruction| match instruction {
            Instruction::SetProperty { id, .. } => self.nodes.contains_key(id),
            Instruction::AppendChild { parent, child, .. } => {
                self.nodes.contains_key(parent) && self.nodes.contains_key(child)
            }
            Instruction::CreateNode { .. }
            | Instruction::ActivateRoots(_)
            | Instruction::Commit => true,
        });
    }

    /// Forgets nodes the runtime's garbage collector reports pruned, as upstream's
    /// `Renderer.prune`.
    pub fn prune(&mut self, pruned: &[NodeId]) {
        for hash in pruned {
            self.nodes.remove(hash);
        }
    }
}

// Upstream's `updateNodeProps`: sends each prop that differs from the last sent, and
// remembers it. A prop dropped from `next` stays as it was.
fn update_props(batch: &mut InstructionBatch, hash: NodeId, prev: &mut Props, next: &Props) {
    for (key, value) in next {
        if prev.get(key) != Some(value) {
            batch.set_property(hash, key, value.clone());
            prev.insert(key.clone(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Runtime;

    fn constant(value: f64) -> Graph {
        NodeRepr::new(
            NodeType::Const,
            Props::from([("value".to_string(), Value::Number(value))]),
            vec![],
        )
    }

    fn keyed_constant(key: NodeId, value: f64) -> Graph {
        let props = Props::from([("value".to_string(), Value::Number(value))]);
        NodeRepr::keyed(KEY_BIT | key, NodeType::Const, props, vec![])
    }

    fn add(a: Graph, b: Graph) -> Graph {
        NodeRepr::new(NodeType::Add, Props::new(), vec![a, b])
    }

    fn creates(batch: &InstructionBatch) -> Vec<NodeId> {
        batch
            .instructions()
            .iter()
            .filter_map(|i| match i {
                Instruction::CreateNode { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    }

    fn sets(batch: &InstructionBatch) -> Vec<(NodeId, String, Value)> {
        batch
            .instructions()
            .iter()
            .filter_map(|i| match i {
                Instruction::SetProperty { id, key, value } => {
                    Some((*id, key.clone(), value.clone()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_same_graph_hashes_the_same_and_a_changed_prop_differently() {
        assert_eq!(constant(1.0).hash, constant(1.0).hash);
        assert_ne!(constant(1.0).hash, constant(2.0).hash);
        assert_ne!(
            add(constant(1.0), constant(2.0)).hash,
            add(constant(2.0), constant(1.0)).hash
        );
        assert_ne!(constant(1.0).channel(1).hash, 0);
        assert_eq!(
            constant(1.0).channel(1).hash,
            constant(1.0).hash,
            "a channel is read off the same node"
        );
        assert_ne!(
            add(constant(1.0), constant(2.0).channel(1)).hash,
            add(constant(1.0), constant(2.0)).hash,
            "but a parent reading it differs"
        );
        assert!(constant(1.0).hash & KEY_BIT == 0);
    }

    #[test]
    fn a_second_render_sends_only_what_is_new() {
        let mut renderer = Renderer::new();
        let first = renderer.render(&[add(constant(1.0), constant(2.0))]);
        assert_eq!(
            creates(&first).len(),
            4,
            "two constants, the add and a root"
        );

        let same = renderer.render(&[add(constant(1.0), constant(2.0))]);
        assert!(creates(&same).is_empty());
        assert!(sets(&same).is_empty());

        let changed = renderer.render(&[add(constant(1.0), constant(3.0))]);
        assert_eq!(
            creates(&changed).len(),
            3,
            "a new constant, add and root; 1.0 is kept"
        );
        assert!(
            !renderer.has(constant(2.0).hash),
            "what the last render didn't reach is forgotten"
        );
        assert!(renderer.has(constant(1.0).hash));
    }

    #[test]
    fn a_keyed_node_takes_a_new_value_in_place() {
        let mut renderer = Renderer::new();
        renderer.render(&[keyed_constant(1, 1.0)]);
        let turned = renderer.render(&[keyed_constant(1, 2.0)]);
        assert!(
            creates(&turned).is_empty(),
            "the root and the node are the same"
        );
        assert_eq!(
            sets(&turned),
            vec![(KEY_BIT | 1, "value".to_string(), Value::Number(2.0))]
        );
    }

    #[test]
    fn a_keyed_node_whose_inputs_move_is_rewired_not_replaced() {
        let keyed_add =
            |a: Graph| NodeRepr::keyed(KEY_BIT | 7, NodeType::Add, Props::new(), vec![a]);
        let mut renderer = Renderer::new();
        renderer.render(&[keyed_add(constant(1.0))]);
        let moved = renderer.render(&[keyed_add(constant(2.0))]);
        assert_eq!(
            creates(&moved),
            vec![KEY_BIT | 7, constant(2.0).hash],
            "the add is created again, its root kept"
        );
        let appended: Vec<_> = moved
            .instructions()
            .iter()
            .filter(|i| matches!(i, Instruction::AppendChild { .. }))
            .collect();
        assert_eq!(appended.len(), 1);
    }

    fn block(rt: &mut Runtime) -> (Vec<f32>, Vec<f32>) {
        let mut l = vec![0.0f32; 64];
        let mut r = vec![0.0f32; 64];
        let mut out: Vec<&mut [f32]> = vec![&mut l, &mut r];
        rt.process(&[], &mut out, 64);
        (l, r)
    }

    #[test]
    fn a_runtime_follows_renders_without_restarting_shared_nodes() {
        let tone = || NodeRepr::new(NodeType::Phasor, Props::new(), vec![constant(100.0)]);
        let mut rendered = Renderer::new();
        let mut rt = Runtime::new(48_000.0, 64);
        rt.apply_instructions(&rendered.render(&[tone(), tone()]))
            .unwrap();
        let mut untouched = Runtime::new(48_000.0, 64);
        untouched
            .apply_instructions(&Renderer::new().render(&[tone(), tone()]))
            .unwrap();
        for _ in 0..40 {
            block(&mut rt);
            block(&mut untouched);
        }

        // Halving the right channel changes its root and adds a Mul; the phasor plays on.
        let half = NodeRepr::new(NodeType::Mul, Props::new(), vec![tone(), constant(0.5)]);
        let batch = rendered.render(&[tone(), half.clone()]);
        assert_eq!(creates(&batch).len(), 3, "a constant, the mul and a root");
        rt.apply_instructions(&batch).unwrap();
        for _ in 0..40 {
            let (l, _) = block(&mut rt);
            let (expected, _) = block(&mut untouched);
            assert_eq!(l, expected, "the phasor kept its phase");
        }
        // The faded root leaves the active set on the next render, and is collected then.
        let nodes = rt.node_ids().len();
        rt.gc();
        assert_eq!(
            rt.node_ids().len(),
            nodes,
            "a faded root stays until the roots are next set"
        );
        let again = rendered.render(&[tone(), half]);
        assert!(creates(&again).is_empty());
        rt.apply_instructions(&again).unwrap();
        rt.gc();
        assert_eq!(rt.node_ids().len(), nodes - 1, "then the old root goes");
    }

    #[test]
    fn a_batch_is_trimmed_to_the_nodes_the_runtime_has() {
        let mut renderer = Renderer::new();
        renderer.render(&[keyed_constant(1, 1.0)]);
        let mut batch = InstructionBatch::new();
        batch.set_property(KEY_BIT | 1, "value", 2.0);
        batch.set_property(KEY_BIT | 2, "value", 3.0);
        batch.commit();
        renderer.retain_known(&mut batch);
        assert_eq!(
            batch.len(),
            2,
            "the unknown node's value is dropped, the commit kept"
        );
        assert_eq!(
            sets(&batch),
            vec![(KEY_BIT | 1, "value".to_string(), Value::Number(2.0))]
        );
    }

    #[test]
    fn no_graphs_plays_silence_and_forgets_everything() {
        let mut renderer = Renderer::new();
        renderer.render(&[constant(1.0)]);
        let silence = renderer.render(&[]);
        assert!(silence
            .instructions()
            .contains(&Instruction::ActivateRoots(vec![])));
        assert!(!renderer.has(constant(1.0).hash));
    }
}
