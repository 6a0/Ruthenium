#version 460
// Ruthenium: the menu background: a vertical gradient with a tiled starfield texture
// (baked at start-up) added on top, one texel per pixel.

#include "push.glsl"
#include "textures.glsl"

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;

layout(push_constant) uniform Push {
    vec4 top;
    vec4 bottom;
    uint stars;
} pc;

void main() {
    vec4 sky = mix(pc.top, pc.bottom, v_uv.y);
    vec2 uv = gl_FragCoord.xy / vec2(texture_size(pc.stars));
    // Stars are stored premultiplied by their brightness: add them.
    vec3 star = sample_texture(pc.stars, SAMPLER_NEAREST_REPEAT, uv).rgb;
    o_color = vec4(min(sky.rgb + star, vec3(1.0)), sky.a);
}
