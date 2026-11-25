#version 460
// Ruthenium: translucent terrain without fog or fade: texels less than a tenth opaque are left
// out and the rest blended, as in vanilla's translucent terrain pipeline.

#define TERRAIN_PLAIN
#define ALPHA_CUTOUT 0.1
#include "terrain_frag.glsl"
