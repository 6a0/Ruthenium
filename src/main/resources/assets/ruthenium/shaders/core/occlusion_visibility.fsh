#version 330
#extension GL_ARB_separate_shader_objects : require

layout(location = 0) flat in float visible;

layout(location = 0) out vec4 fragVisible;

void main() {
    fragVisible = vec4(visible, 0.0, 0.0, 0.0);
}
