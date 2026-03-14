mod ffi;

use std::ffi::{CStr, CString};
use std::fmt;
use std::os::raw::c_void;
use std::ptr;

//==============================================================================
// Error types
//==============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    UnknownNodeType,
    NodeNotFound,
    NodeAlreadyExists,
    NodeTypeAlreadyExists,
    InvalidPropertyType,
    InvalidPropertyValue,
    InvariantViolation,
    InvalidInstructionFormat,
    Unknown(i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownNodeType => write!(f, "Node type not recognized"),
            Error::NodeNotFound => write!(f, "Node not found"),
            Error::NodeAlreadyExists => write!(f, "Node already exists"),
            Error::NodeTypeAlreadyExists => write!(f, "Node type already exists"),
            Error::InvalidPropertyType => write!(f, "Invalid property type"),
            Error::InvalidPropertyValue => write!(f, "Invalid property value"),
            Error::InvariantViolation => write!(f, "Invariant violation"),
            Error::InvalidInstructionFormat => write!(f, "Invalid instruction format"),
            Error::Unknown(code) => write!(f, "Unknown error (code {})", code),
        }
    }
}

impl std::error::Error for Error {}

fn return_code_to_result(code: i32) -> Result<(), Error> {
    match code {
        0 => Ok(()),
        1 => Err(Error::UnknownNodeType),
        2 => Err(Error::NodeNotFound),
        3 => Err(Error::NodeAlreadyExists),
        4 => Err(Error::NodeTypeAlreadyExists),
        5 => Err(Error::InvalidPropertyType),
        6 => Err(Error::InvalidPropertyValue),
        7 => Err(Error::InvariantViolation),
        8 => Err(Error::InvalidInstructionFormat),
        c => Err(Error::Unknown(c)),
    }
}

//==============================================================================
// Property values
//==============================================================================

/// A value that can be set as a node property.
pub enum PropertyValue {
    Number(f64),
    String(String),
    Bool(bool),
    FloatArray(Vec<f32>),
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

//==============================================================================
// InstructionBatch
//==============================================================================

/// A batch of instructions to apply to the runtime.
///
/// Build up a set of graph mutations (create nodes, set properties, wire
/// children, activate roots) then apply them atomically via
/// [`Runtime::apply_instructions`].
pub struct InstructionBatch {
    ptr: *mut ffi::ElemBatch,
}

impl InstructionBatch {
    pub fn new() -> Self {
        let ptr = unsafe { ffi::elem_batch_new() };
        assert!(!ptr.is_null());
        InstructionBatch { ptr }
    }

    /// Remove all instructions from the batch so it can be reused.
    pub fn clear(&mut self) {
        unsafe { ffi::elem_batch_clear(self.ptr) }
    }

    /// Create a new node with the given id and type.
    pub fn create_node(&mut self, node_id: i32, node_type: &str) {
        let c_type = CString::new(node_type).expect("node_type must not contain null bytes");
        unsafe { ffi::elem_batch_create_node(self.ptr, node_id, c_type.as_ptr()) }
    }

    /// Set a property on a node.
    pub fn set_property(&mut self, node_id: i32, key: &str, value: impl Into<PropertyValue>) {
        let c_key = CString::new(key).expect("key must not contain null bytes");
        match value.into() {
            PropertyValue::Number(v) => unsafe {
                ffi::elem_batch_set_property_number(self.ptr, node_id, c_key.as_ptr(), v);
            },
            PropertyValue::String(v) => {
                let c_val = CString::new(v).expect("string value must not contain null bytes");
                unsafe {
                    ffi::elem_batch_set_property_string(
                        self.ptr,
                        node_id,
                        c_key.as_ptr(),
                        c_val.as_ptr(),
                    );
                }
            }
            PropertyValue::Bool(v) => unsafe {
                ffi::elem_batch_set_property_bool(
                    self.ptr,
                    node_id,
                    c_key.as_ptr(),
                    v as i32,
                );
            },
            PropertyValue::FloatArray(v) => unsafe {
                ffi::elem_batch_set_property_float_array(
                    self.ptr,
                    node_id,
                    c_key.as_ptr(),
                    v.as_ptr(),
                    v.len(),
                );
            },
        }
    }

    /// Wire a child node's output to a parent node's next inlet.
    pub fn append_child(&mut self, parent_id: i32, child_id: i32, output_channel: i32) {
        unsafe { ffi::elem_batch_append_child(self.ptr, parent_id, child_id, output_channel) }
    }

    /// Activate the given root node ids (deactivating any prior roots not in this list).
    pub fn activate_roots(&mut self, root_ids: &[i32]) {
        unsafe { ffi::elem_batch_activate_roots(self.ptr, root_ids.as_ptr(), root_ids.len()) }
    }

    /// Signal the end of the instruction batch, triggering a render sequence rebuild.
    pub fn commit(&mut self) {
        unsafe { ffi::elem_batch_commit(self.ptr) }
    }
}

impl Default for InstructionBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for InstructionBatch {
    fn drop(&mut self) {
        unsafe { ffi::elem_batch_free(self.ptr) }
    }
}

//==============================================================================
// Runtime
//==============================================================================

/// The Elementary Audio runtime.
///
/// Wraps the C++ `elem::Runtime<float>` engine. Create one per audio context,
/// feed it instructions to build the audio graph, then call [`process`](Self::process)
/// from your audio callback.
pub struct Runtime {
    ptr: *mut ffi::ElemRuntime,
}

// The C++ Runtime can be moved between threads safely. Thread-safety for
// concurrent access is the caller's responsibility (same as the C++ API).
unsafe impl Send for Runtime {}

impl Runtime {
    /// Create a new runtime with the given sample rate and block size.
    pub fn new(sample_rate: f64, block_size: i32) -> Self {
        let ptr = unsafe { ffi::elem_runtime_new(sample_rate, block_size) };
        assert!(!ptr.is_null());
        Runtime { ptr }
    }

    /// Apply a batch of graph instructions.
    pub fn apply_instructions(&mut self, batch: &InstructionBatch) -> Result<(), Error> {
        let rc = unsafe { ffi::elem_runtime_apply_instructions(self.ptr, batch.ptr) };
        return_code_to_result(rc)
    }

    /// Run the audio processing callback.
    ///
    /// `input` and `output` are slices of channel buffers. Each channel buffer
    /// must be at least `num_samples` long.
    pub fn process(
        &mut self,
        input: &[&[f32]],
        output: &mut [&mut [f32]],
        num_samples: usize,
    ) {
        let input_ptrs: Vec<*const f32> = input.iter().map(|ch| ch.as_ptr()).collect();
        let mut output_ptrs: Vec<*mut f32> = output.iter_mut().map(|ch| ch.as_mut_ptr()).collect();

        unsafe {
            ffi::elem_runtime_process(
                self.ptr,
                input_ptrs.as_ptr(),
                input_ptrs.len(),
                output_ptrs.as_mut_ptr(),
                output_ptrs.len(),
                num_samples,
                ptr::null_mut(),
            );
        }
    }

    /// Run the audio processing callback with raw pointers.
    ///
    /// Use this in real-time audio callbacks where you already have raw channel
    /// pointers from your audio API (e.g. CPAL, CoreAudio, JACK).
    ///
    /// # Safety
    /// All pointer arguments must be valid for the given channel counts and
    /// `num_samples`.
    pub unsafe fn process_raw(
        &mut self,
        input: *const *const f32,
        num_inputs: usize,
        output: *mut *mut f32,
        num_outputs: usize,
        num_samples: usize,
        user_data: *mut c_void,
    ) {
        ffi::elem_runtime_process(
            self.ptr,
            input,
            num_inputs,
            output,
            num_outputs,
            num_samples,
            user_data,
        );
    }

    /// Process queued events from analysis nodes (meter, scope, snapshot, etc.).
    ///
    /// The callback receives the event type as a string and the event value
    /// serialized as a JSON string.
    pub fn process_queued_events<F: FnMut(&str, &str)>(&mut self, mut callback: F) {
        unsafe extern "C" fn trampoline<F: FnMut(&str, &str)>(
            event_type: *const std::os::raw::c_char,
            value_json: *const std::os::raw::c_char,
            user_data: *mut c_void,
        ) {
            let cb = &mut *(user_data as *mut F);
            let t = CStr::from_ptr(event_type).to_str().unwrap_or("");
            let v = CStr::from_ptr(value_json).to_str().unwrap_or("");
            cb(t, v);
        }

        unsafe {
            ffi::elem_runtime_process_queued_events(
                self.ptr,
                trampoline::<F>,
                &mut callback as *mut F as *mut c_void,
            );
        }
    }

    /// Reset all graph node internal state (delay lines, sample readers, etc.).
    pub fn reset(&mut self) {
        unsafe { ffi::elem_runtime_reset(self.ptr) }
    }

    /// Garbage-collect unreferenced nodes. Returns the pruned node ids.
    pub fn gc(&mut self) -> Vec<i32> {
        let mut buf = vec![0i32; 4096];
        let count =
            unsafe { ffi::elem_runtime_gc(self.ptr, buf.as_mut_ptr(), buf.len()) };
        buf.truncate(count);
        buf
    }

    /// Load a mono audio buffer as a shared resource.
    ///
    /// Returns `true` if the resource was added, `false` if a resource with
    /// that name already exists.
    pub fn add_shared_resource(&mut self, name: &str, data: &[f32]) -> bool {
        let c_name = CString::new(name).expect("name must not contain null bytes");
        let rc = unsafe {
            ffi::elem_runtime_add_shared_resource(
                self.ptr,
                c_name.as_ptr(),
                data.as_ptr(),
                data.len(),
            )
        };
        rc != 0
    }

    /// Load a multi-channel audio buffer as a shared resource.
    ///
    /// Returns `true` if the resource was added, `false` if a resource with
    /// that name already exists.
    pub fn add_shared_resource_multi(&mut self, name: &str, channels: &[&[f32]]) -> bool {
        let c_name = CString::new(name).expect("name must not contain null bytes");
        let ptrs: Vec<*const f32> = channels.iter().map(|ch| ch.as_ptr()).collect();
        let num_samples = channels.first().map(|ch| ch.len()).unwrap_or(0);

        let rc = unsafe {
            ffi::elem_runtime_add_shared_resource_multi(
                self.ptr,
                c_name.as_ptr(),
                ptrs.as_ptr(),
                ptrs.len(),
                num_samples,
            )
        };
        rc != 0
    }

    /// Remove shared resources that are no longer referenced by the active graph.
    pub fn prune_shared_resources(&mut self) {
        unsafe { ffi::elem_runtime_prune_shared_resources(self.ptr) }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { ffi::elem_runtime_free(self.ptr) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_drop_runtime() {
        let _rt = Runtime::new(44100.0, 512);
    }

    #[test]
    fn build_simple_graph() {
        let mut rt = Runtime::new(44100.0, 512);

        let mut batch = InstructionBatch::new();

        // const(440) -> sin -> root(channel=0)
        batch.create_node(1, "const");
        batch.set_property(1, "value", 440.0);
        batch.create_node(2, "sin");
        batch.append_child(2, 1, 0);
        batch.create_node(3, "root");
        batch.set_property(3, "channel", 0.0);
        batch.append_child(3, 2, 0);
        batch.activate_roots(&[3]);
        batch.commit();

        rt.apply_instructions(&batch).unwrap();
    }

    #[test]
    fn process_silence() {
        let mut rt = Runtime::new(44100.0, 512);

        let input_buf = vec![0.0f32; 512];
        let input: &[&[f32]] = &[&input_buf];

        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];
        let mut output: Vec<&mut [f32]> = vec![&mut out_l, &mut out_r];

        rt.process(input, &mut output, 512);

        // No graph → output should remain silent
        assert!(out_l.iter().all(|&s| s == 0.0));
        assert!(out_r.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn process_const_graph() {
        let mut rt = Runtime::new(44100.0, 512);

        let mut batch = InstructionBatch::new();
        batch.create_node(1, "const");
        batch.set_property(1, "value", 0.5);
        batch.create_node(2, "root");
        batch.set_property(2, "channel", 0.0);
        batch.append_child(2, 1, 0);
        batch.activate_roots(&[2]);
        batch.commit();
        rt.apply_instructions(&batch).unwrap();

        let input_buf = vec![0.0f32; 512];
        let input: &[&[f32]] = &[&input_buf];
        let mut out_l = vec![0.0f32; 512];
        let mut out_r = vec![0.0f32; 512];

        // Process a few blocks to let the root node fade in
        for _ in 0..10 {
            out_l.fill(0.0);
            out_r.fill(0.0);
            let mut output: Vec<&mut [f32]> = vec![&mut out_l, &mut out_r];
            rt.process(input, &mut output, 512);
        }

        // After fade-in, output should be near 0.5
        let last = *out_l.last().unwrap();
        assert!(
            (last - 0.5).abs() < 0.01,
            "expected ~0.5, got {}",
            last
        );
    }

    #[test]
    fn shared_resource() {
        let mut rt = Runtime::new(44100.0, 512);

        let samples: Vec<f32> = (0..1024).map(|i| (i as f32) / 1024.0).collect();
        assert!(rt.add_shared_resource("test_buf", &samples));
        // Duplicate name should return false
        assert!(!rt.add_shared_resource("test_buf", &samples));
    }

    #[test]
    fn gc_empty() {
        let mut rt = Runtime::new(44100.0, 512);
        let pruned = rt.gc();
        assert!(pruned.is_empty());
    }

    #[test]
    fn batch_reuse() {
        let mut rt = Runtime::new(44100.0, 512);
        let mut batch = InstructionBatch::new();

        batch.create_node(1, "const");
        batch.set_property(1, "value", 1.0);
        batch.create_node(2, "root");
        batch.set_property(2, "channel", 0.0);
        batch.append_child(2, 1, 0);
        batch.activate_roots(&[2]);
        batch.commit();
        rt.apply_instructions(&batch).unwrap();

        // Clear and reuse
        batch.clear();
        batch.set_property(1, "value", 0.25);
        batch.commit();
        rt.apply_instructions(&batch).unwrap();
    }
}
