#version 460
// Ruthenium: cutout terrain (leaves, plants, panes): texels less than half opaque are
// holes, as in vanilla's cutout terrain pipeline.

#define ALPHA_CUTOUT 0.5
#include "terrain_frag.glsl"
