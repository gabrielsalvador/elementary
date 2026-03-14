#pragma once

#include <map>
#include <sstream>
#include <variant>
#include <vector>
#include <functional>

namespace elem
{

    //==============================================================================
    // Representations of primitive JavaScript values
    struct Undefined {};
    struct Null {};

    using Boolean = bool;
    using Number = double;
    using String = std::string;

    //==============================================================================
    // Forward declare the Value to allow recursive type definitions
    struct Value;

    //==============================================================================
    // Representations of JavaScript Objects
    using Object = std::map<String, Value>;
    using Array = std::vector<Value>;
    using Float32Array = std::vector<float>;
    using Function = std::function<Value(Array)>;

    //==============================================================================
    // Value is a std::variant representing any dynamically-typed JavaScript value.
    struct Value : std::variant<Undefined, Null, Boolean, Number, String, Object, Array, Float32Array, Function>
    {
        using variant::variant;

        // Default constructor creates an undefined value
        Value() : variant(Undefined{}) {}

        // Allow construction from const char* as String
        Value(char const* v) : variant(String(v)) {}
    };

    // We need moves to avoid allocations on the realtime thread if moving from
    // a lock free queue.
    static_assert(std::is_move_assignable<Value>::value);

    //==============================================================================
    // Type checks
    inline bool isUndefined(Value const& v)    { return std::holds_alternative<Undefined>(v); }
    inline bool isNull(Value const& v)         { return std::holds_alternative<Null>(v); }
    inline bool isBool(Value const& v)         { return std::holds_alternative<Boolean>(v); }
    inline bool isNumber(Value const& v)       { return std::holds_alternative<Number>(v); }
    inline bool isString(Value const& v)       { return std::holds_alternative<String>(v); }
    inline bool isArray(Value const& v)        { return std::holds_alternative<Array>(v); }
    inline bool isFloat32Array(Value const& v) { return std::holds_alternative<Float32Array>(v); }
    inline bool isObject(Value const& v)       { return std::holds_alternative<Object>(v); }
    inline bool isFunction(Value const& v)     { return std::holds_alternative<Function>(v); }

    //==============================================================================
    // Getters
    inline Array const&        getArray(Value const& v)        { return std::get<Array>(v); }
    inline Array&              getArray(Value& v)              { return std::get<Array>(v); }
    inline Float32Array const& getFloat32Array(Value const& v) { return std::get<Float32Array>(v); }
    inline Float32Array&       getFloat32Array(Value& v)       { return std::get<Float32Array>(v); }
    inline Object const&       getObject(Value const& v)       { return std::get<Object>(v); }
    inline Object&             getObject(Value& v)             { return std::get<Object>(v); }
    inline Function const&     getFunction(Value const& v)     { return std::get<Function>(v); }
    inline Function&           getFunction(Value& v)           { return std::get<Function>(v); }

    //==============================================================================
    // Object property access with a default return value
    template <typename T>
    inline T getWithDefault(Value const& v, std::string const& k, T const& def) {
        auto const& o = std::get<Object>(v);
        auto it = o.find(k);
        if (it != o.end()) return T(it->second);
        return def;
    }

    //==============================================================================
    // String representation (forward-declared; defined after all types are complete)
    inline String toString(Value const& v);

    inline String toString(Value const& v)
    {
        if (isUndefined(v)) { return "undefined"; }
        if (isNull(v)) { return "null"; }
        if (isBool(v)) { return String(std::to_string(std::get<Boolean>(v))); }
        if (isNumber(v)) { return String(std::to_string(std::get<Number>(v))); }
        if (isString(v)) { return std::get<String>(v); }
        if (isArray(v))
        {
            auto& a = getArray(v);
            std::stringstream ss;
            ss << "[";

            for (size_t i = 0; i < std::min((size_t) 3, a.size()); ++i)
                ss << toString(a[i]) << ", ";

            if (a.size() > 3)
            {
                ss << "...]";
                return ss.str();
            }

            auto s = ss.str();
            return s.substr(0, s.size() - 2) + "]";
        }
        if (isFloat32Array(v))
        {
            auto& fa = getFloat32Array(v);
            std::stringstream ss;
            ss << "[";

            for (size_t i = 0; i < std::min((size_t) 3, fa.size()); ++i)
                ss << std::to_string(fa[i]) << ", ";

            if (fa.size() > 3)
            {
                ss << "...]";
                return ss.str();
            }

            auto s = ss.str();
            return s.substr(0, s.size() - 2) + "]";
        }
        if (isObject(v))
        {
            std::stringstream ss;
            ss << "{\n";

            for (auto const& [k, val] : getObject(v)) {
                ss << "    " << k << ": " << toString(val) << "\n";
            }

            ss << "}\n";
            return ss.str();
        }

        // TODO
        if (isFunction(v)) { return "[Object Function]"; }

        return "undefined";
    }

    static inline std::ostream& operator<< (std::ostream& s, Value const& v)
    {
        s << toString(v);
        return s;
    }

} // namespace elem
