#pragma once

#include "../../Invariant.h"


namespace elem
{
namespace ValueHelpers
{

    // Name says it all
    //
    // Just a utility for converting a Array to a std::vector<float>
    template <typename FloatType>
    std::vector<FloatType> arrayToFloatVector (Array const& ar)
    {
        try {
            std::vector<FloatType> ret (ar.size());

            for (size_t i = 0; i < ar.size(); ++i) {
                ret[i] = static_cast<FloatType>(std::get<Number>(ar)[i]);
            }

            return ret;
        } catch (std::exception const& e) {
            throw InvariantViolation("Failed to convert Array to float vector; invalid array child!");
        }
    }

    // Same thing as above, but with in-place mutation
    template <typename FloatType>
    void arrayToFloatVector (std::vector<FloatType>& target, Array const& ar)
    {
        try {
            target.resize(ar.size());

            for (size_t i = 0; i < ar.size(); ++i) {
                target[i] = static_cast<FloatType>(std::get<Number>(ar)[i]);
            }
        } catch (std::exception const& e) {
            throw InvariantViolation("Failed to convert Array to float vector; invalid array child!");
        }
    }


} // namespace ValueHelpers
} // namespace elem
