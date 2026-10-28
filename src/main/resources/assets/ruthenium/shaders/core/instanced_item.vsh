#version 330
#extension GL_ARB_separate_shader_objects : require
#extension GL_ARB_gpu_shader5 : require

#include <minecraft:light.glsl>
#include <minecraft:fog.glsl>
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
#include <minecraft:sample_lightmap.glsl>

layout(location = 0) in vec3 Position;
layout(location = 1) in vec4 Color;
layout(location = 2) in vec2 UV0;
layout(location = 3) in ivec2 UV1;
layout(location = 4) in ivec2 UV2;
layout(location = 5) in vec3 Normal;
// Ruthenium: per instance, the rows of the item's affine pose (camera-relative) and of its normal matrix, its light
// and overlay; the mesh's color is the quad's tint, its UV2.x the quad's light emission (with MESH_LIGHT: its UV2 the
// vertex's own light, of a Fabric Renderer API mesh), its overlay is not used
layout(location = 6) in vec4 InstanceRow0;
layout(location = 7) in vec4 InstanceRow1;
layout(location = 8) in vec4 InstanceRow2;
layout(location = 9) in vec3 InstanceNormal0;
layout(location = 10) in vec3 InstanceNormal1;
layout(location = 11) in vec3 InstanceNormal2;
layout(location = 12) in ivec4 InstanceLightOverlay;
layout(location = 13) in vec4 InstanceColor;

uniform sampler2D Sampler1;
uniform sampler2D Sampler2;

layout(location = 0) out float sphericalVertexDistance;
layout(location = 1) out float cylindricalVertexDistance;
layout(location = 2) out vec4 vertexColor;
layout(location = 3) out vec4 lightMapColor;
layout(location = 4) out vec4 overlayColor;
layout(location = 5) out vec2 texCoord0;

void main() {
    // Ruthenium: vanilla's core/item.vsh for an item-space mesh drawn once per instance, with positions transformed in
    // the order of JOML's Matrix4f.transformPosition, as the instanced parts are
    precise vec3 pos;
    pos.x = InstanceRow0.x * Position.x + (InstanceRow0.y * Position.y + (InstanceRow0.z * Position.z + InstanceRow0.w));
    pos.y = InstanceRow1.x * Position.x + (InstanceRow1.y * Position.y + (InstanceRow1.z * Position.z + InstanceRow1.w));
    pos.z = InstanceRow2.x * Position.x + (InstanceRow2.y * Position.y + (InstanceRow2.z * Position.z + InstanceRow2.w));
    vec3 normal = normalize(vec3(dot(InstanceNormal0, Normal), dot(InstanceNormal1, Normal), dot(InstanceNormal2, Normal)));
    gl_Position = ProjMat * ModelViewMat * vec4(pos, 1.0);

    sphericalVertexDistance = fog_spherical_distance(pos);
    cylindricalVertexDistance = fog_cylindrical_distance(pos);
    vertexColor = minecraft_mix_light(Light0_Direction, Light1_Direction, normal, Color);

#ifdef MESH_LIGHT
    // ExtraLightCoordsUtil.smoothMax of the vertex's light and the instance's, as Indigo's minLightmap
    ivec2 light = max(UV2 & 255, InstanceLightOverlay.xy & 255);
#else
    // LightCoordsUtil.lightCoordsWithEmission: block and sky light raised to the quad's emission (whole levels)
    ivec2 light = InstanceLightOverlay.xy;
    int emission = UV2.x;
    if (emission != 0) {
        light = ivec2(max((light.x >> 4) & 15, emission) << 4, max((light.y >> 4) & 15, emission) << 4);
    }
#endif
    lightMapColor = sample_lightmap(Sampler2, light);
    overlayColor = texelFetch(Sampler1, InstanceLightOverlay.zw, 0);

    texCoord0 = UV0;
}
