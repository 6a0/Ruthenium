#version 460
// Ruthenium: translucent terrain with hard texels: texels less than a tenth opaque are left
// out and the rest blended, as in vanilla's translucent terrain pipeline.

#define ALPHA_CUTOUT 0.1
#include "terrain_frag.glsl"
