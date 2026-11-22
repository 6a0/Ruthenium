#version 460
// Ruthenium: the development overlay's rectangles, one colour each.

layout(location = 0) in vec4 v_color;
layout(location = 0) out vec4 o_color;

void main() {
    o_color = v_color;
}
