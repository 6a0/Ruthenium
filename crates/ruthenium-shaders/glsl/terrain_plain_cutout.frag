#version 460
// Ruthenium: cutout terrain without fog or fade: texels less than half opaque are
// holes, as in vanilla's cutout terrain pipeline.

#define TERRAIN_PLAIN
#define ALPHA_CUTOUT 0.5
#include "terrain_frag.glsl"
