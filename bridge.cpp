#include <elem/Runtime.h>
#include <elem/AudioBufferResource.h>

#include <cstring>
#include <sstream>
#include <string>
#include <vector>

// We instantiate Runtime<float> for the Rust bridge.
// A Runtime<double> bridge can be added later if needed.
using ElemRT = elem::Runtime<float>;

//==============================================================================
// JSON serialization for elem::Value (used for event callbacks)
static std::string valueToJson(elem::Value const& v)
{
    if (elem::isUndefined(v)) return "null";
    if (elem::isNull(v)) return "null";

    if (elem::isBool(v))
        return std::get<elem::Boolean>(v) ? "true" : "false";

    if (elem::isNumber(v)) {
        std::ostringstream ss;
        ss << std::get<elem::Number>(v);
        return ss.str();
    }

    if (elem::isString(v)) {
        auto const& s = std::get<elem::String>(v);
        std::string out = "\"";
        for (char c : s) {
            switch (c) {
                case '"':  out += "\\\""; break;
                case '\\': out += "\\\\"; break;
                case '\n': out += "\\n";  break;
                case '\r': out += "\\r";  break;
                case '\t': out += "\\t";  break;
                default:   out += c;      break;
            }
        }
        out += "\"";
        return out;
    }

    if (elem::isArray(v)) {
        auto const& arr = elem::getArray(v);
        std::string out = "[";
        for (size_t i = 0; i < arr.size(); ++i) {
            if (i > 0) out += ",";
            out += valueToJson(arr[i]);
        }
        out += "]";
        return out;
    }

    if (elem::isFloat32Array(v)) {
        auto const& arr = elem::getFloat32Array(v);
        std::string out = "[";
        for (size_t i = 0; i < arr.size(); ++i) {
            if (i > 0) out += ",";
            std::ostringstream ss;
            ss << arr[i];
            out += ss.str();
        }
        out += "]";
        return out;
    }

    if (elem::isObject(v)) {
        auto const& obj = elem::getObject(v);
        std::string out = "{";
        bool first = true;
        for (auto const& [key, val] : obj) {
            if (!first) out += ",";
            first = false;
            out += "\"" + key + "\":" + valueToJson(val);
        }
        out += "}";
        return out;
    }

    return "null";
}

//==============================================================================
// Instruction batch builder
struct ElemBatch {
    elem::Array instructions;
};

//==============================================================================
extern "C" {

//------------------------------------------------------------------------------
// Runtime lifecycle
//------------------------------------------------------------------------------
ElemRT* elem_runtime_new(double sample_rate, int block_size)
{
    return new ElemRT(sample_rate, block_size);
}

void elem_runtime_free(ElemRT* rt)
{
    delete rt;
}

//------------------------------------------------------------------------------
// Audio processing
//------------------------------------------------------------------------------
void elem_runtime_process(
    ElemRT* rt,
    const float* const* input,
    size_t num_inputs,
    float** output,
    size_t num_outputs,
    size_t num_samples,
    void* user_data)
{
    rt->process(const_cast<const float**>(input), num_inputs, output, num_outputs, num_samples, user_data);
}

//------------------------------------------------------------------------------
// Instruction batch
//------------------------------------------------------------------------------
ElemBatch* elem_batch_new()
{
    return new ElemBatch();
}

void elem_batch_free(ElemBatch* batch)
{
    delete batch;
}

void elem_batch_clear(ElemBatch* batch)
{
    batch->instructions.clear();
}

void elem_batch_create_node(ElemBatch* batch, int32_t node_id, const char* type)
{
    batch->instructions.push_back(elem::Array {
        elem::Value(0.0),                           // CREATE_NODE
        elem::Value(static_cast<double>(node_id)),
        elem::Value(std::string(type)),
    });
}

void elem_batch_set_property_number(ElemBatch* batch, int32_t node_id, const char* key, double value)
{
    batch->instructions.push_back(elem::Array {
        elem::Value(3.0),                           // SET_PROPERTY
        elem::Value(static_cast<double>(node_id)),
        elem::Value(std::string(key)),
        elem::Value(value),
    });
}

void elem_batch_set_property_string(ElemBatch* batch, int32_t node_id, const char* key, const char* value)
{
    batch->instructions.push_back(elem::Array {
        elem::Value(3.0),                           // SET_PROPERTY
        elem::Value(static_cast<double>(node_id)),
        elem::Value(std::string(key)),
        elem::Value(std::string(value)),
    });
}

void elem_batch_set_property_bool(ElemBatch* batch, int32_t node_id, const char* key, int value)
{
    batch->instructions.push_back(elem::Array {
        elem::Value(3.0),
        elem::Value(static_cast<double>(node_id)),
        elem::Value(std::string(key)),
        elem::Value(static_cast<elem::Boolean>(value != 0)),
    });
}

void elem_batch_set_property_float_array(
    ElemBatch* batch,
    int32_t node_id,
    const char* key,
    const float* data,
    size_t len)
{
    elem::Float32Array arr(data, data + len);

    batch->instructions.push_back(elem::Array {
        elem::Value(3.0),
        elem::Value(static_cast<double>(node_id)),
        elem::Value(std::string(key)),
        elem::Value(std::move(arr)),
    });
}

void elem_batch_set_property_seq(
    ElemBatch* batch,
    int32_t node_id,
    const char* key,
    const double* times,
    const double* values,
    size_t len)
{
    elem::Array arr;
    arr.reserve(len);

    for (size_t i = 0; i < len; ++i) {
        elem::Object obj;
        obj["time"] = elem::Value(times[i]);
        obj["value"] = elem::Value(values[i]);
        arr.push_back(elem::Value(std::move(obj)));
    }

    batch->instructions.push_back(elem::Array {
        elem::Value(3.0),
        elem::Value(static_cast<double>(node_id)),
        elem::Value(std::string(key)),
        elem::Value(std::move(arr)),
    });
}

void elem_batch_append_child(ElemBatch* batch, int32_t parent_id, int32_t child_id, int32_t output_channel)
{
    batch->instructions.push_back(elem::Array {
        elem::Value(2.0),                           // APPEND_CHILD
        elem::Value(static_cast<double>(parent_id)),
        elem::Value(static_cast<double>(child_id)),
        elem::Value(static_cast<double>(output_channel)),
    });
}

void elem_batch_activate_roots(ElemBatch* batch, const int32_t* root_ids, size_t count)
{
    elem::Array roots;
    for (size_t i = 0; i < count; ++i) {
        roots.push_back(elem::Value(static_cast<double>(root_ids[i])));
    }

    batch->instructions.push_back(elem::Array {
        elem::Value(4.0),                           // ACTIVATE_ROOTS
        elem::Value(std::move(roots)),
    });
}

void elem_batch_commit(ElemBatch* batch)
{
    batch->instructions.push_back(elem::Array {
        elem::Value(5.0),                           // COMMIT_UPDATES
    });
}

int elem_runtime_apply_instructions(ElemRT* rt, const ElemBatch* batch)
{
    // applyInstructions takes a const& so we can pass batch->instructions directly
    return rt->applyInstructions(batch->instructions);
}

//------------------------------------------------------------------------------
// Events
//------------------------------------------------------------------------------
typedef void (*ElemEventCallback)(const char* type, const char* value_json, void* user_data);

void elem_runtime_process_queued_events(ElemRT* rt, ElemEventCallback callback, void* user_data)
{
    rt->processQueuedEvents([&](std::string const& type, elem::Value evt) {
        std::string json = valueToJson(evt);
        callback(type.c_str(), json.c_str(), user_data);
    });
}

//------------------------------------------------------------------------------
// State management
//------------------------------------------------------------------------------
void elem_runtime_reset(ElemRT* rt)
{
    rt->reset();
}

size_t elem_runtime_gc(ElemRT* rt, int32_t* out_ids, size_t max_ids)
{
    auto pruned = rt->gc();
    size_t count = 0;

    for (auto const& id : pruned) {
        if (count >= max_ids) break;
        out_ids[count++] = id;
    }

    return count;
}

//------------------------------------------------------------------------------
// Shared resources
//------------------------------------------------------------------------------
int elem_runtime_add_shared_resource(
    ElemRT* rt,
    const char* name,
    const float* data,
    size_t num_samples)
{
    auto resource = std::make_unique<elem::AudioBufferResource>(
        const_cast<float*>(data), num_samples);
    return rt->addSharedResource(std::string(name), std::move(resource)) ? 1 : 0;
}

int elem_runtime_add_shared_resource_multi(
    ElemRT* rt,
    const char* name,
    const float* const* data,
    size_t num_channels,
    size_t num_samples)
{
    auto resource = std::make_unique<elem::AudioBufferResource>(
        const_cast<float**>(data), num_channels, num_samples);
    return rt->addSharedResource(std::string(name), std::move(resource)) ? 1 : 0;
}

void elem_runtime_prune_shared_resources(ElemRT* rt)
{
    rt->pruneSharedResources();
}

//------------------------------------------------------------------------------
// Graph introspection
//------------------------------------------------------------------------------
// Returns a JSON snapshot of the graph. Caller must free the returned string with elem_free_string.
char* elem_runtime_snapshot(ElemRT* rt)
{
    auto obj = rt->snapshot();
    std::string json = valueToJson(elem::Value(std::move(obj)));
    char* result = static_cast<char*>(std::malloc(json.size() + 1));
    std::memcpy(result, json.c_str(), json.size() + 1);
    return result;
}

void elem_free_string(char* s)
{
    std::free(s);
}

} // extern "C"
