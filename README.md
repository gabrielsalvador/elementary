# elementary

A pure-Rust port of the [Elementary Audio](https://elementary.audio) runtime:
a graph-based audio engine driven by instruction batches.

The original C++ runtime, its FFI bridge, and the WebAssembly build have been
replaced by a single Rust crate with no dependencies. The public API is the
same shape as the previous bindings, so host code that built `InstructionBatch`es
and called `Runtime::process` keeps working.

## Usage

```rust
use elementary::{InstructionBatch, NodeType, Runtime};

let mut rt = Runtime::new(44100.0, 512);

// const(440) -> phasor -> mul(2π) -> sin -> root(channel 0)
let mut batch = InstructionBatch::new();
batch.create_node(1, NodeType::Const);
batch.set_property(1, "value", 440.0);
batch.create_node(2, NodeType::Phasor);
batch.append_child(2, 1, 0);
batch.create_node(3, NodeType::Const);
batch.set_property(3, "value", std::f64::consts::TAU);
batch.create_node(4, NodeType::Mul);
batch.append_child(4, 2, 0);
batch.append_child(4, 3, 0);
batch.create_node(5, NodeType::Sin);
batch.append_child(5, 4, 0);
batch.create_node(6, NodeType::Root);
batch.set_property(6, "channel", 0.0);
batch.append_child(6, 5, 0);
batch.activate_roots(&[6]);
batch.commit();
rt.apply_instructions(&batch).unwrap();

let mut left = vec![0.0f32; 512];
let mut right = vec![0.0f32; 512];
let mut output: Vec<&mut [f32]> = vec![&mut left, &mut right];
rt.process(&[], &mut output, 512);

rt.process_queued_events(|event_type, value_json| {
    println!("{event_type}: {value_json}");
});
```

## Threading model

`Runtime` is `Send` but every method takes `&mut self`. The host serializes
access: apply instructions and render from the same thread, or hand the
runtime to the audio thread and send it batches through a channel. There are
no atomics or lock-free queues inside; a `Mutex` guards only the `tapIn` /
`tapOut` feedback buffers and is never contended.

`process` calls longer than the configured block size are rendered in
block-size chunks.

## Node types

All builtin types of the original runtime are available, see `NodeType::ALL`:
`const`, `phasor`, `sphasor`, `sr`, `root`, `in`; unary math (`sin`, `cos`,
`tan`, `tanh`, `asinh`, `ln`, `log`, `log2`, `ceil`, `floor`, `round`, `sqrt`,
`exp`, `abs`); binary (`le`, `leq`, `ge`, `geq`, `pow`, `eq`, `and`, `or`);
reducing (`add`, `sub`, `mul`, `div`, `mod`, `min`, `max`); sequencers (`seq`,
`seq2`, `sparseq`, `sparseq2`, `counter`, `accum`, `latch`, `maxhold`, `once`);
delays (`delay`, `sdelay`, `z`); filters (`pole`, `env`, `biquad`, `prewarp`,
`mm1p`, `svf`, `svfshelf`); feedback (`tapIn`, `tapOut`); sampling (`sample`,
`sampleseq`, `sampleseq2`, `table`, `mc.sample`, `mc.sampleseq`,
`mc.sampleseq2`, `mc.table`, `mc.capture`); oscillators (`blepsaw`,
`blepsquare`, `bleptriangle`); analysis (`meter`, `scope`, `snapshot`,
`capture`); and `rand`.

### Differences from the C++ runtime

- `sampleseq2` and `mc.sampleseq2` accept the `shift` and `stretch`
  properties for compatibility but do not time-stretch. The original used the
  Signalsmith Stretch library, which is not ported. They play at natural rate.
- `seq` / `seq2` accept both a plain number array (`Vec<f64>`) and a
  `Float32Array` for their `seq` property.
- `sparseq` accepts `time` as an alias for `tickTime` in its events.
- JSON numbers in events and snapshots are printed with full precision rather
  than the six significant digits of `std::ostream`.
- Output channels are always cleared at the start of `process`, even when no
  graph has been committed.

## Shared resources

```rust
rt.add_shared_resource("kick", &mono_samples);
rt.add_shared_resource_multi("pad", &[&left, &right]);
rt.prune_shared_resources(); // drop buffers no node references
```

Sample-playing nodes refer to a resource via their `path` property. Setting
`path` to an unknown name fails with `Error::InvalidPropertyValue`.
