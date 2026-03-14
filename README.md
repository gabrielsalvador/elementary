# Elementary Audio — Standalone Native Runtime

This is a modification of [Elementary Audio](https://elementary.audio) stripped down to its core C++ runtime engine, with Rust bindings for use in native applications.

The JavaScript packages, CLI tooling, and npm infrastructure have been removed. What remains is:

- **`runtime/`** — The header-only C++ audio DSP engine (`elem::Runtime<FloatType>`)
- **`wasm/`** — WebAssembly bridge (Emscripten bindings)
- **`src/`**, **`bridge.cpp`**, **`build.rs`** — Rust FFI bridge for embedding the runtime in native Rust code

## Rust Usage

Add as a git dependency:

```toml
[dependencies]
elementary = { git = "https://github.com/gabrielsalvador/elementary" }
```

```rust
use elementary::{Runtime, InstructionBatch};

fn main() {
    let mut rt = Runtime::new(44100.0, 512);

    // Build an audio graph: const(440) -> sin -> root
    let mut batch = InstructionBatch::new();
    batch.create_node(1, "const");
    batch.set_property(1, "value", 440.0);
    batch.create_node(2, "sin");
    batch.append_child(2, 1, 0);
    batch.create_node(3, "root");
    batch.set_property(3, "channel", 0.0); // output channel index
    batch.append_child(3, 2, 0);
    batch.activate_roots(&[3]);
    batch.commit();

    rt.apply_instructions(&batch).unwrap();

    // Process audio blocks
    let input_buf = vec![0.0f32; 512];
    let input: &[&[f32]] = &[&input_buf];
    let mut out_l = vec![0.0f32; 512];
    let mut out_r = vec![0.0f32; 512];

    let mut output: Vec<&mut [f32]> = vec![&mut out_l, &mut out_r];
    rt.process(input, &mut output, 512);

    // Handle events from analysis nodes (meter, scope, snapshot)
    rt.process_queued_events(|event_type, value_json| {
        println!("{}: {}", event_type, value_json);
    });
}
```

### Available Node Types

All 40+ built-in Elementary node types are available: `sin`, `cos`, `add`, `mul`, `phasor`, `biquad`, `svf`, `delay`, `sample`, `meter`, `scope`, and many more. See the [Elementary docs](https://www.elementary.audio/docs/) for the full reference.

### Shared Resources

Load audio buffers for sample playback:

```rust
let samples: Vec<f32> = load_audio_file("kick.wav");
rt.add_shared_resource("kick", &samples);

// Then reference it in your graph
batch.create_node(10, "sample");
batch.set_property(10, "path", "kick");
```

## Building

The Rust crate compiles the C++ bridge automatically via `build.rs` using the [`cc`](https://crates.io/crates/cc) crate. Requirements:

- Rust toolchain (stable)
- C++17-capable compiler (clang or gcc)

```sh
cargo build
cargo test
```

## Original Project

Based on [Elementary Audio](https://github.com/elemaudio/elementary) by Nick Thompson. Elementary is [MIT licensed](./LICENSE.md).
