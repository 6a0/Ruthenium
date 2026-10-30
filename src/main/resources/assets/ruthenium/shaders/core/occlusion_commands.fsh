#version 330
#extension GL_ARB_separate_shader_objects : require

layout(location = 0) flat in uint value;

layout(location = 0) out uvec4 fragValue;

void main() {
    fragValue = uvec4(value, 0u, 0u, 0u);
}
