// Ruthenium: how the world looks behind a menu. `grade.x` is how much of its colour is
// left (1: all of it) and `grade.y` how bright it is; (1, 1) leaves a colour exactly as
// it is.
vec3 graded(vec3 c, vec2 grade) {
    float grey = dot(c, vec3(0.2126, 0.7152, 0.0722));
    return mix(vec3(grey), c, grade.x) * grade.y;
}
