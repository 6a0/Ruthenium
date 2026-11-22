// Ruthenium: per-frame data, descriptor set 1 of every pipeline (ruthenium-gpu's
// FrameData): this frame's region of one host-written storage buffer. Include it after
// declaring the element struct, with FRAME_ELEMENT naming it; draws find their elements
// by an index in push constants.
#define FRAME_DATA_SET 1

layout(set = FRAME_DATA_SET, binding = 0, std430) readonly buffer FrameData {
    FRAME_ELEMENT frame_data[];
};
