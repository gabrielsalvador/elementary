use std::os::raw::{c_char, c_int, c_void};

// Opaque C++ types
#[repr(C)]
pub struct ElemRuntime {
    _private: [u8; 0],
}

#[repr(C)]
pub struct ElemBatch {
    _private: [u8; 0],
}

pub type ElemEventCallback = unsafe extern "C" fn(
    event_type: *const c_char,
    value_json: *const c_char,
    user_data: *mut c_void,
);

extern "C" {
    // Runtime lifecycle
    pub fn elem_runtime_new(sample_rate: f64, block_size: c_int) -> *mut ElemRuntime;
    pub fn elem_runtime_free(rt: *mut ElemRuntime);

    // Audio processing
    pub fn elem_runtime_process(
        rt: *mut ElemRuntime,
        input: *const *const f32,
        num_inputs: usize,
        output: *mut *mut f32,
        num_outputs: usize,
        num_samples: usize,
        user_data: *mut c_void,
    );

    // Instruction batch
    pub fn elem_batch_new() -> *mut ElemBatch;
    pub fn elem_batch_free(batch: *mut ElemBatch);
    pub fn elem_batch_clear(batch: *mut ElemBatch);

    pub fn elem_batch_create_node(batch: *mut ElemBatch, node_id: i32, node_type: *const c_char);

    pub fn elem_batch_set_property_number(
        batch: *mut ElemBatch,
        node_id: i32,
        key: *const c_char,
        value: f64,
    );

    pub fn elem_batch_set_property_string(
        batch: *mut ElemBatch,
        node_id: i32,
        key: *const c_char,
        value: *const c_char,
    );

    pub fn elem_batch_set_property_bool(
        batch: *mut ElemBatch,
        node_id: i32,
        key: *const c_char,
        value: c_int,
    );

    pub fn elem_batch_set_property_float_array(
        batch: *mut ElemBatch,
        node_id: i32,
        key: *const c_char,
        data: *const f32,
        len: usize,
    );

    pub fn elem_batch_set_property_seq(
        batch: *mut ElemBatch,
        node_id: i32,
        key: *const c_char,
        times: *const f64,
        values: *const f64,
        len: usize,
    );

    pub fn elem_batch_append_child(
        batch: *mut ElemBatch,
        parent_id: i32,
        child_id: i32,
        output_channel: i32,
    );

    pub fn elem_batch_activate_roots(batch: *mut ElemBatch, root_ids: *const i32, count: usize);

    pub fn elem_batch_commit(batch: *mut ElemBatch);

    pub fn elem_runtime_apply_instructions(rt: *mut ElemRuntime, batch: *const ElemBatch) -> c_int;

    // Events
    pub fn elem_runtime_process_queued_events(
        rt: *mut ElemRuntime,
        callback: ElemEventCallback,
        user_data: *mut c_void,
    );

    // State
    pub fn elem_runtime_reset(rt: *mut ElemRuntime);
    pub fn elem_runtime_gc(rt: *mut ElemRuntime, out_ids: *mut i32, max_ids: usize) -> usize;

    // Shared resources
    pub fn elem_runtime_add_shared_resource(
        rt: *mut ElemRuntime,
        name: *const c_char,
        data: *const f32,
        num_samples: usize,
    ) -> c_int;

    pub fn elem_runtime_add_shared_resource_multi(
        rt: *mut ElemRuntime,
        name: *const c_char,
        data: *const *const f32,
        num_channels: usize,
        num_samples: usize,
    ) -> c_int;

    pub fn elem_runtime_prune_shared_resources(rt: *mut ElemRuntime);

    // Graph introspection
    pub fn elem_runtime_snapshot(rt: *mut ElemRuntime) -> *mut c_char;
    pub fn elem_free_string(s: *mut c_char);
}
