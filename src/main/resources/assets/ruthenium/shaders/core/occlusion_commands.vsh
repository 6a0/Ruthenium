#version 330
#extension GL_ARB_separate_shader_objects : require

// Five points per terrain draw command (one per int): the command as the CPU wrote it, with its instance count
// replaced by its section's visibility, at its ints' texels of the command target, which is then copied into the
// indirect buffer.

layout(std140) uniform OcclusionInfo {
    ivec2 DepthSize;
    ivec2 VisibilitySize;
    ivec2 CommandSize;
    int HiZLevels;
};

uniform sampler2D VisibilitySampler;

// Index count, instance count, first index, base vertex
layout(location = 0) in uvec4 Command;
// Base instance: the index of the section's info
layout(location = 1) in uint CommandSection;

layout(location = 0) flat out uint value;

void main() {
    int field = gl_VertexIndex;
    int index = gl_InstanceIndex * 5 + field;
    ivec2 texel = ivec2(index % CommandSize.x, index / CommandSize.x);
    gl_Position = vec4((vec2(texel) + 0.5) / vec2(CommandSize) * 2.0 - 1.0, 0.0, 1.0);
    gl_PointSize = 1.0;
    if (field == 4) {
        value = CommandSection;
    } else if (field == 1) {
        int section = int(CommandSection);
        value = texelFetch(VisibilitySampler, ivec2(section % VisibilitySize.x, section / VisibilitySize.x), 0).r > 0.5 ? 1u : 0u;
    } else {
        value = Command[field];
    }
}
