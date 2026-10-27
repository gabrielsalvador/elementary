use super::*;

fn render(rt: &mut Runtime, blocks: usize, n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    for _ in 0..blocks {
        let mut out: Vec<&mut [f32]> = vec![&mut l, &mut r];
        rt.process(&[], &mut out, n);
    }
    (l, r)
}

fn const_root(value: f64, channel: f64) -> InstructionBatch {
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", value);
    b.create_node(2, NodeType::Root);
    b.set_property(2, "channel", channel);
    b.append_child(2, 1, 0);
    b.activate_roots(&[2]);
    b.commit();
    b
}

#[test]
fn create_and_drop_runtime() {
    let _rt = Runtime::new(44100.0, 512);
}

#[test]
fn every_node_type_is_constructible() {
    let mut rt = Runtime::new(44100.0, 512);
    let mut b = InstructionBatch::new();
    for (i, t) in NodeType::ALL.iter().enumerate() {
        b.create_node(i as i32, *t);
        assert_eq!(NodeType::from_str(t.as_str()), Some(*t));
    }
    rt.apply_instructions(&b).unwrap();
    assert_eq!(rt.node_ids().len(), NodeType::ALL.len());
}

#[test]
fn unknown_type_and_duplicate_ids() {
    let mut rt = Runtime::new(44100.0, 512);
    let mut b = InstructionBatch::new();
    b.create_node_named(1, "nope");
    assert_eq!(rt.apply_instructions(&b), Err(Error::UnknownNodeType));
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.create_node(1, NodeType::Const);
    assert_eq!(rt.apply_instructions(&b), Err(Error::NodeAlreadyExists));
    let mut b = InstructionBatch::new();
    b.set_property(99, "value", 1.0);
    assert_eq!(rt.apply_instructions(&b), Err(Error::NodeNotFound));
    let mut b = InstructionBatch::new();
    b.set_property(1, "value", "not a number");
    assert_eq!(rt.apply_instructions(&b), Err(Error::InvalidPropertyType));
}

#[test]
fn process_silence() {
    let mut rt = Runtime::new(44100.0, 512);
    let (l, r) = render(&mut rt, 1, 512);
    assert!(l.iter().all(|&s| s == 0.0));
    assert!(r.iter().all(|&s| s == 0.0));
}

#[test]
fn process_const_graph_fades_in() {
    let mut rt = Runtime::new(44100.0, 512);
    rt.apply_instructions(&const_root(0.5, 0.0)).unwrap();
    let (l, r) = render(&mut rt, 1, 512);
    // 20ms fade at 44.1k is 882 samples: first block is still ramping.
    assert!(l[0] < 0.01);
    assert!(l[511] > 0.2 && l[511] < 0.5);
    assert!(r.iter().all(|&s| s == 0.0));
    let (l, _) = render(&mut rt, 10, 512);
    assert!((l[511] - 0.5).abs() < 1e-6, "got {}", l[511]);
}

#[test]
fn root_deactivation_fades_out_and_stops() {
    let mut rt = Runtime::new(44100.0, 512);
    rt.apply_instructions(&const_root(1.0, 0.0)).unwrap();
    render(&mut rt, 10, 512);
    let mut b = InstructionBatch::new();
    b.activate_roots(&[]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let (l, _) = render(&mut rt, 1, 512);
    assert!(l[0] > 0.9 && l[511] < 0.6, "fade-out expected, got {} .. {}", l[0], l[511]);
    let (l, _) = render(&mut rt, 5, 512);
    assert!(l.iter().all(|&s| s == 0.0));
}

#[test]
fn sine_tone_has_expected_amplitude() {
    let mut rt = Runtime::new(48000.0, 256);
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 480.0);
    b.create_node(2, NodeType::Phasor);
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::Const);
    b.set_property(3, "value", std::f64::consts::TAU);
    b.create_node(4, NodeType::Mul);
    b.append_child(4, 2, 0);
    b.append_child(4, 3, 0);
    b.create_node(5, NodeType::Sin);
    b.append_child(5, 4, 0);
    b.create_node(6, NodeType::Root);
    b.set_property(6, "channel", 1.0);
    b.append_child(6, 5, 0);
    b.activate_roots(&[6]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    render(&mut rt, 20, 256);
    let (l, r) = render(&mut rt, 1, 256);
    assert!(l.iter().all(|&s| s == 0.0));
    let max = r.iter().cloned().fold(0.0f32, f32::max);
    let min = r.iter().cloned().fold(0.0f32, f32::min);
    assert!(max > 0.99 && min < -0.99, "max {max} min {min}");
    // 480 Hz at 48 kHz: 100 samples per cycle, so r[i] == r[i+100].
    for i in 0..100 {
        assert!((r[i] - r[i + 100]).abs() < 1e-3);
    }
}

#[test]
fn large_process_calls_are_chunked() {
    let mut rt = Runtime::new(44100.0, 64);
    rt.apply_instructions(&const_root(0.25, 0.0)).unwrap();
    render(&mut rt, 20, 64);
    let (l, _) = render(&mut rt, 1, 1000);
    assert!(l.iter().all(|&s| (s - 0.25).abs() < 1e-6));
}

#[test]
fn shared_resource_and_table() {
    let mut rt = Runtime::new(44100.0, 512);
    let samples: Vec<f32> = (0..1024).map(|i| (i as f32) / 1023.0).collect();
    assert!(rt.add_shared_resource("ramp", &samples));
    assert!(!rt.add_shared_resource("ramp", &samples));

    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 0.5);
    b.create_node(2, NodeType::Table);
    b.set_property(2, "path", "ramp");
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::Root);
    b.set_property(3, "channel", 0.0);
    b.append_child(3, 2, 0);
    b.activate_roots(&[3]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    render(&mut rt, 10, 512);
    let (l, _) = render(&mut rt, 1, 512);
    assert!((l[511] - 0.5).abs() < 1e-3, "got {}", l[511]);

    let mut b = InstructionBatch::new();
    b.set_property(2, "path", "missing");
    assert_eq!(rt.apply_instructions(&b), Err(Error::InvalidPropertyValue));

    rt.prune_shared_resources();
    assert_eq!(rt.shared_resource_names(), vec!["ramp".to_string()]);
}

#[test]
fn gc_prunes_unreachable_nodes() {
    let mut rt = Runtime::new(44100.0, 512);
    let mut b = const_root(1.0, 0.0);
    b.create_node(10, NodeType::Sin);
    b.create_node(11, NodeType::Const);
    b.append_child(10, 11, 0);
    rt.apply_instructions(&b).unwrap();
    let pruned = rt.gc();
    assert_eq!(pruned, vec![10, 11]);
    assert_eq!(rt.node_ids(), vec![1, 2]);
    assert!(rt.gc().is_empty());
}

#[test]
fn batch_reuse() {
    let mut rt = Runtime::new(44100.0, 512);
    let mut batch = const_root(1.0, 0.0);
    rt.apply_instructions(&batch).unwrap();
    batch.clear();
    batch.set_property(1, "value", 0.25);
    batch.commit();
    rt.apply_instructions(&batch).unwrap();
    render(&mut rt, 20, 512);
    let (l, _) = render(&mut rt, 1, 512);
    assert!((l[0] - 0.25).abs() < 1e-6);
}

#[test]
fn meter_and_snapshot_events() {
    let mut rt = Runtime::new(44100.0, 512);
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", -0.5);
    b.create_node(2, NodeType::Meter);
    b.set_property(2, "name", "m1");
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::Root);
    b.set_property(3, "channel", 0.0);
    b.append_child(3, 2, 0);
    b.activate_roots(&[3]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    render(&mut rt, 1, 512);
    let mut events = Vec::new();
    rt.process_queued_events(|t, v| events.push((t.to_string(), v.to_string())));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, "meter");
    assert_eq!(events[0].1, r#"{"max":-0.5,"min":-0.5,"rms":0.5,"source":"m1"}"#);
    events.clear();
    rt.process_queued_events(|t, v| events.push((t.to_string(), v.to_string())));
    assert!(events.is_empty());
}

#[test]
fn scope_emits_after_enough_samples() {
    let mut rt = Runtime::new(44100.0, 256);
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 1.0);
    b.create_node(2, NodeType::Scope);
    b.set_property(2, "size", 256.0);
    b.set_property(2, "channels", 1.0);
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::Root);
    b.set_property(3, "channel", 0.0);
    b.append_child(3, 2, 0);
    b.activate_roots(&[3]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let mut events = Vec::new();
    render(&mut rt, 1, 256);
    rt.process_queued_events(|t, v| events.push((t.to_string(), v.to_string())));
    assert!(events.is_empty());
    render(&mut rt, 1, 256);
    rt.process_queued_events(|t, v| events.push((t.to_string(), v.to_string())));
    assert_eq!(events.len(), 1);
    assert!(events[0].1.starts_with(r#"{"data":[[1,1,1"#), "{}", events[0].1);

    let mut b = InstructionBatch::new();
    b.set_property(2, "size", 100.0);
    assert_eq!(rt.apply_instructions(&b), Err(Error::InvalidPropertyValue));
}

#[test]
fn snapshot_lists_nodes_props_and_inlets() {
    let mut rt = Runtime::new(44100.0, 512);
    rt.apply_instructions(&const_root(0.5, 0.0)).unwrap();
    let snap = rt.snapshot();
    assert_eq!(
        snap,
        r#"{"0x00000001":{"inlets":[],"properties":{"value":0.5},"type":"const"},"0x00000002":{"inlets":[{"channel":0,"source":1}],"properties":{"_internal:numChildren":1,"active":true,"channel":0},"type":"root"}}"#
    );
}

#[test]
fn seq_steps_on_trigger_edges() {
    let mut rt = Runtime::new(4.0, 8);
    let mut b = InstructionBatch::new();
    // A 1 Hz phasor at 4 Hz sample rate ticks every 4 samples; seq triggers on
    // rising edges of a square derived from it.
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 1.0);
    b.create_node(2, NodeType::Phasor);
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::Const);
    b.set_property(3, "value", 0.5);
    b.create_node(4, NodeType::Le);
    b.append_child(4, 2, 0);
    b.append_child(4, 3, 0);
    b.create_node(5, NodeType::Seq);
    b.set_property(5, "seq", vec![1.0f64, 2.0, 3.0]);
    b.set_property(5, "hold", true);
    b.append_child(5, 4, 0);
    b.create_node(6, NodeType::Root);
    b.set_property(6, "channel", 0.0);
    b.set_property(6, "fadeInMs", 0.0);
    b.append_child(6, 5, 0);
    b.activate_roots(&[6]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let (l, _) = render(&mut rt, 1, 8);
    // phase 0,.25,.5,.75,0,... -> le(.5): 1,1,0,0,1,1,0,0. First rising edge at
    // sample 0 (from 0 to 1) picks 1; next rising edge at sample 4 picks 2.
    // Sample 0 is always scaled by the root's starting gain of zero.
    assert_eq!(&l[1..8], &[1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0]);
}

#[test]
fn sparseq2_holds_and_interpolates() {
    let mut rt = Runtime::new(10.0, 10);
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 1.0);
    b.create_node(2, NodeType::Counter);
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::Sparseq2);
    b.set_property_seq(3, "seq", &[2.0, 6.0], &[10.0, 20.0]);
    b.append_child(3, 2, 0);
    b.create_node(4, NodeType::Root);
    b.set_property(4, "channel", 0.0);
    b.set_property(4, "fadeInMs", 0.0);
    b.append_child(4, 3, 0);
    b.activate_roots(&[4]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let (l, _) = render(&mut rt, 1, 10);
    assert_eq!(&l[..10], &[0.0, 0.0, 10.0, 10.0, 10.0, 10.0, 20.0, 20.0, 20.0, 20.0]);

    let mut b = InstructionBatch::new();
    b.set_property(3, "interpolate", 1.0);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let mut rt2 = Runtime::new(10.0, 10);
    let mut b2 = InstructionBatch::new();
    b2.create_node(1, NodeType::Const);
    b2.set_property(1, "value", 1.0);
    b2.create_node(2, NodeType::Counter);
    b2.append_child(2, 1, 0);
    b2.create_node(3, NodeType::Sparseq2);
    b2.set_property_seq(3, "seq", &[2.0, 6.0], &[10.0, 20.0]);
    b2.set_property(3, "interpolate", 1.0);
    b2.append_child(3, 2, 0);
    b2.create_node(4, NodeType::Root);
    b2.set_property(4, "channel", 0.0);
    b2.set_property(4, "fadeInMs", 0.0);
    b2.append_child(4, 3, 0);
    b2.activate_roots(&[4]);
    b2.commit();
    rt2.apply_instructions(&b2).unwrap();
    let (l, _) = render(&mut rt2, 1, 10);
    assert_eq!(&l[..10], &[0.0, 0.0, 10.0, 12.5, 15.0, 17.5, 20.0, 20.0, 20.0, 20.0]);
}

#[test]
fn tap_feedback_delays_by_one_block() {
    let mut rt = Runtime::new(44100.0, 16);
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 1.0);
    b.create_node(2, NodeType::TapOut);
    b.set_property(2, "name", "fb");
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::TapIn);
    b.set_property(3, "name", "fb");
    b.create_node(4, NodeType::Add);
    b.append_child(4, 2, 0);
    b.append_child(4, 3, 0);
    b.create_node(5, NodeType::Root);
    b.set_property(5, "channel", 0.0);
    b.set_property(5, "fadeInMs", 0.0);
    b.append_child(5, 4, 0);
    b.activate_roots(&[5]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let (l, _) = render(&mut rt, 1, 16);
    assert!(l[1..].iter().all(|&s| s == 1.0), "first block: tapIn is silent: {:?}", l);
    let (l, _) = render(&mut rt, 1, 16);
    assert!(l.iter().all(|&s| s == 2.0), "second block: tapIn carries block 1");
}

#[test]
fn sample_node_plays_on_trigger() {
    let mut rt = Runtime::new(44100.0, 8);
    let data: Vec<f32> = (1..=8).map(|i| i as f32).collect();
    rt.add_shared_resource("clip", &data);
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 1.0);
    b.create_node(2, NodeType::Sample);
    b.set_property(2, "path", "clip");
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::Root);
    b.set_property(3, "channel", 0.0);
    b.set_property(3, "fadeInMs", 0.0);
    b.append_child(3, 2, 0);
    b.activate_roots(&[3]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let (l, _) = render(&mut rt, 1, 8);
    // Trigger rises at sample 0; the reader ramps its gain in over ~10ms so the
    // output follows the ramp shape rather than the raw data, but is nonzero
    // and monotonically increasing across the buffer.
    assert!(l[0] >= 0.0);
    assert!(l[7] > l[1], "{:?}", l);
    let (l, _) = render(&mut rt, 1, 8);
    assert!(l.iter().all(|&s| s == 0.0), "one-shot ends: {:?}", l);
}

#[test]
fn multichannel_outlets_get_separate_buffers() {
    let mut rt = Runtime::new(44100.0, 8);
    let left: Vec<f32> = vec![0.25; 4];
    let right: Vec<f32> = vec![0.75; 4];
    rt.add_shared_resource_multi("stereo", &[&left, &right]);
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 0.0);
    b.create_node(2, NodeType::McTable);
    b.set_property(2, "path", "stereo");
    b.append_child(2, 1, 0);
    b.create_node(3, NodeType::Root);
    b.set_property(3, "channel", 0.0);
    b.set_property(3, "fadeInMs", 0.0);
    b.append_child(3, 2, 0);
    b.create_node(4, NodeType::Root);
    b.set_property(4, "channel", 1.0);
    b.set_property(4, "fadeInMs", 0.0);
    b.append_child(4, 2, 1);
    b.activate_roots(&[3, 4]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let (l, r) = render(&mut rt, 1, 8);
    assert!(l[1..].iter().all(|&s| (s - 0.25).abs() < 1e-6), "{:?}", l);
    assert!(r[1..].iter().all(|&s| (s - 0.75).abs() < 1e-6), "{:?}", r);
}

#[test]
fn runtime_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Runtime>();
    assert_send::<InstructionBatch>();
}

#[test]
fn slew_starts_at_its_input_then_ramps_at_its_rate() {
    let mut rt = Runtime::new(48000.0, 256);
    let mut b = InstructionBatch::new();
    b.create_node(1, NodeType::Const);
    b.set_property(1, "value", 100.0);
    b.create_node(2, NodeType::Const);
    b.set_property(2, "value", 0.5);
    b.create_node(3, NodeType::Slew);
    b.append_child(3, 2, 0);
    b.append_child(3, 1, 0);
    b.create_node(4, NodeType::Root);
    b.set_property(4, "channel", 0.0);
    b.append_child(4, 3, 0);
    b.activate_roots(&[4]);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    // Past the root's fade-in.
    let (l, _) = render(&mut rt, 20, 256);
    assert!(l.iter().all(|&s| s == 100.0), "no ramp up from zero");

    let mut b = InstructionBatch::new();
    b.set_property(1, "value", 0.0);
    b.commit();
    rt.apply_instructions(&b).unwrap();
    let (l, _) = render(&mut rt, 1, 256);
    assert_eq!(l[0], 99.5);
    assert_eq!(l[99], 50.0);
    assert_eq!(l[199], 0.0, "reaches the target");
    assert_eq!(l[255], 0.0, "and stays there");
}
