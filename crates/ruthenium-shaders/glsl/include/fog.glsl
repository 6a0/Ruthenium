// Ruthenium: fog, as every stage that draws the world has it.

// How much fog there is at a distance, from none at `range.x` to all at `range.y`.
float fog_amount(float distance, vec2 range) {
    if (distance <= range.x) {
        return 0.0;
    }
    if (distance >= range.y) {
        return 1.0;
    }
    return (distance - range.x) / (range.y - range.x);
}
