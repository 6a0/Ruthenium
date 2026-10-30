#version 330
#extension GL_ARB_separate_shader_objects : require

// A level of the terrain occlusion depth pyramid: each texel keeps the farthest depth (reversed-Z: the smallest) of
// the FACTOR x FACTOR texels below it, and the last texel of a row or column also takes what is left of the level
// below. From the depth into mip 0, then from each even mip into the one two below it.

const int FACTOR = 4;

uniform sampler2D InSampler;

layout(location = 0) out vec4 fragColor;

void main() {
    ivec2 sourceSize = textureSize(InSampler, 0);
    ivec2 size = max(sourceSize / FACTOR, ivec2(1));
    ivec2 texel = ivec2(gl_FragCoord.xy);
    ivec2 from = texel * FACTOR;
    ivec2 to = min(from + (FACTOR - 1), sourceSize - 1);
    if (texel.x == size.x - 1) {
        to.x = sourceSize.x - 1;
    }
    if (texel.y == size.y - 1) {
        to.y = sourceSize.y - 1;
    }
    float farthest = 1.0;
    for (int y = from.y; y <= to.y; y++) {
        for (int x = from.x; x <= to.x; x++) {
            farthest = min(farthest, texelFetch(InSampler, ivec2(x, y), 0).r);
        }
    }
    fragColor = vec4(farthest, 0.0, 0.0, 0.0);
}
