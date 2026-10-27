#version 330
#extension GL_ARB_separate_shader_objects : require
#extension GL_ARB_gpu_shader5 : require

#if defined(PER_FACE_LIGHTING) || !defined(NO_CARDINAL_LIGHTING)
#include <minecraft:light.glsl>
#endif
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
// VulkanMod: per instance, the rows of the part's affine pose (camera-relative) and of its normal matrix, its light,
// overlay and color; the mesh's own color, overlay and light are not used
layout(location = 6) in vec4 InstanceRow0;
layout(location = 7) in vec4 InstanceRow1;
layout(location = 8) in vec4 InstanceRow2;
layout(location = 9) in vec3 InstanceNormal0;
layout(location = 10) in vec3 InstanceNormal1;
layout(location = 11) in vec3 InstanceNormal2;
layout(location = 12) in ivec4 InstanceLightOverlay;
layout(location = 13) in vec4 InstanceColor;

#if !defined(NO_OVERLAY) && !defined(OIT_ALPHA_ONLY)
uniform sampler2D Sampler1;
#endif

#if !defined(EMISSIVE) && !defined(OIT_ALPHA_ONLY)
uniform sampler2D Sampler2;
#endif

layout(location = 0) out float sphericalVertexDistance;
layout(location = 1) out float cylindricalVertexDistance;

#ifdef PER_FACE_LIGHTING
layout(location = 2) out vec4 vertexPerFaceColorBack;
layout(location = 3) out vec4 vertexPerFaceColorFront;
#else
layout(location = 2) out vec4 vertexColor;
#endif

#ifndef EMISSIVE
layout(location = 4) out vec4 lightMapColor;
#endif

#ifndef NO_OVERLAY
layout(location = 5) out vec4 overlayColor;
#endif

layout(location = 6) out vec2 texCoord0;
#ifdef GLINT
layout(location = 7) out vec2 texCoordGlint;
#endif

void main() {
    // VulkanMod: vanilla's core/entity.vsh for a part-space mesh drawn once per instance
    // The same operations in the same order as JOML's Matrix4f.transformPosition (no fused multiply-add), so positions
    // are bit-identical to vanilla's: layers written by vanilla on the same faces (horse markings, eyes) keep their
    // depth ties with the instanced parts
    precise vec3 pos;
    pos.x = InstanceRow0.x * Position.x + (InstanceRow0.y * Position.y + (InstanceRow0.z * Position.z + InstanceRow0.w));
    pos.y = InstanceRow1.x * Position.x + (InstanceRow1.y * Position.y + (InstanceRow1.z * Position.z + InstanceRow1.w));
    pos.z = InstanceRow2.x * Position.x + (InstanceRow2.y * Position.y + (InstanceRow2.z * Position.z + InstanceRow2.w));
    vec3 normal = normalize(vec3(dot(InstanceNormal0, Normal), dot(InstanceNormal1, Normal), dot(InstanceNormal2, Normal)));
    vec4 color = InstanceColor;
    gl_Position = ProjMat * ModelViewMat * vec4(pos, 1.0);

    sphericalVertexDistance = fog_spherical_distance(pos);
    cylindricalVertexDistance = fog_cylindrical_distance(pos);

#ifdef PER_FACE_LIGHTING
    vec2 light = minecraft_compute_light(Light0_Direction, Light1_Direction, normal);
    vertexPerFaceColorBack = minecraft_mix_light_separate(-light, color);
    vertexPerFaceColorFront = minecraft_mix_light_separate(light, color);
#elif defined(NO_CARDINAL_LIGHTING)
    vertexColor = color;
#else
    vertexColor = minecraft_mix_light(Light0_Direction, Light1_Direction, normal, color);
#endif

#if !defined(EMISSIVE) && !defined(OIT_ALPHA_ONLY)
    lightMapColor = sample_lightmap(Sampler2, InstanceLightOverlay.xy);
#endif

#if !defined(NO_OVERLAY) && !defined(OIT_ALPHA_ONLY)
    overlayColor = texelFetch(Sampler1, InstanceLightOverlay.zw, 0);
#endif

    texCoord0 = UV0;

#ifdef APPLY_TEXTURE_MATRIX
    texCoord0 = (TextureMat * vec4(UV0, 0.0, 1.0)).xy;
#endif

#ifdef GLINT
    texCoordGlint = (TextureMat * vec4(UV0, 0.0, 1.0)).xy;
#endif
}
