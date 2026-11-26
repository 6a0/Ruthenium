// Ruthenium: fog, as every stage that draws the world has it.

// How much fog there is at a distance. `line` is the fog's (scale, bias), worked out on
// the CPU from where it starts and where it is complete (atmosphere.rs, fog_line): none
// up to the start, all from the end, and in a straight line between, as vanilla's
// linear fog. One multiply-add instead of two branches and a division.
float fog_amount(float distance, vec2 line) {
    return clamp(distance * line.x + line.y, 0.0, 1.0);
}
