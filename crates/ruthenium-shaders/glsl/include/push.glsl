// Ruthenium: shared by every pipeline. One pipeline layout has a single push-constant
// range of PUSH_CONSTANT_BYTES (the minimum every Vulkan device guarantees), so switching
// pipelines never invalidates push constants or descriptor bindings.
#define PUSH_CONSTANT_BYTES 128
