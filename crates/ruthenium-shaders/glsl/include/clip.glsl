// Ruthenium: clipping for the modern menus' shapes and text. `clip` is x0, y0, x1, y1 in
// window pixels; within `fade` pixels of its edges (top, bottom, left, right) coverage
// falls to 0, so a scrolled list melts away at its ends.
float clip_coverage(vec4 clip, vec4 fade) {
    vec2 p = gl_FragCoord.xy;
    if (p.x < clip.x || p.y < clip.y || p.x >= clip.z || p.y >= clip.w) {
        return 0.0;
    }
    float c = 1.0;
    if (fade.x > 0.0) {
        c = min(c, (p.y - clip.y) / fade.x);
    }
    if (fade.y > 0.0) {
        c = min(c, (clip.w - p.y) / fade.y);
    }
    if (fade.z > 0.0) {
        c = min(c, (p.x - clip.x) / fade.z);
    }
    if (fade.w > 0.0) {
        c = min(c, (clip.z - p.x) / fade.w);
    }
    return clamp(c, 0.0, 1.0);
}
