// Ruthenium: the shared texture table, descriptor set 0 of every pipeline (ruthenium-gpu's
// TextureTable). Draws pass a TextureId and a SamplerId in push constants or per-instance
// data and sample through sample_texture; nothing is bound per draw.
#extension GL_EXT_nonuniform_qualifier : require

// SamplerId, in order.
#define SAMPLER_NEAREST_CLAMP 0
#define SAMPLER_NEAREST_REPEAT 1
#define SAMPLER_LINEAR_CLAMP 2
#define SAMPLER_LINEAR_REPEAT 3
#define SAMPLER_COUNT 4

// TextureId::MISSING: the checkerboard every unused slot shows.
#define TEXTURE_MISSING 0

layout(set = 0, binding = 0) uniform sampler u_samplers[SAMPLER_COUNT];
layout(set = 0, binding = 1) uniform texture2D u_textures[];

// The indices may differ between invocations of one draw (one GUI batch mixes textures).
vec4 sample_texture(uint tex, uint smp, vec2 uv) {
    return texture(sampler2D(u_textures[nonuniformEXT(tex)], u_samplers[nonuniformEXT(smp)]), uv);
}

ivec2 texture_size(uint tex) {
    return textureSize(sampler2D(u_textures[nonuniformEXT(tex)], u_samplers[SAMPLER_NEAREST_CLAMP]), 0);
}
