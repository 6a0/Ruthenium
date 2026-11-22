// Ruthenium: clipping for the modern menus' shapes and text. `clip` is x0, y0, x1, y1 in
// window pixels; within `fade_top` pixels of its top edge (`fade_bottom` of its bottom
// edge) coverage falls to 0, so a scrolled list melts away at its ends.
float clip_coverage(vec4 clip, float fade_top, float fade_bottom) {
    vec2 p = gl_FragCoord.xy;
    if (p.x < clip.x || p.y < clip.y || p.x >= clip.z || p.y >= clip.w) {
        return 0.0;
    }
    float c = 1.0;
    if (fade_top > 0.0) {
        c = min(c, (p.y - clip.y) / fade_top);
    }
    if (fade_bottom > 0.0) {
        c = min(c, (clip.w - p.y) / fade_bottom);
    }
    return clamp(c, 0.0, 1.0);
}
