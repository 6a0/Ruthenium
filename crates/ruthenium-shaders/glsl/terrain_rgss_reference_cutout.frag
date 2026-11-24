#version 460
// Ruthenium: cutout terrain with RGSS, every sample taken: texels less than half opaque are
// holes, as in vanilla's cutout terrain pipeline.

#define TERRAIN_RGSS_REFERENCE
#define ALPHA_CUTOUT 0.5
#include "terrain_frag.glsl"
