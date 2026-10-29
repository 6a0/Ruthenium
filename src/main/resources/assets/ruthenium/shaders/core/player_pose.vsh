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

// Ruthenium: a part-space mesh (one copy per member of the draw); each vertex names its member and part slot
layout(location = 0) in vec3 Position;
layout(location = 1) in vec2 UV0;
layout(location = 2) in vec3 Normal;
layout(location = 3) in ivec2 PartIndex;

// Per member: the entity format's UV2 (light) and UV1 (overlay), its color, and per slot the rows of the part's
// affine pose (camera-relative) and of its normal matrix (the first one's w: 1 if the pose's normals are normalized)
struct PoseMember {
    ivec4 LightOverlay;
    vec4 Color;
    vec4 Rows[POSE_SLOTS * 6];
};

layout(std140) uniform PlayerPoses {
    PoseMember Members[POSE_MEMBERS];
};

// The bits of max(c / 127.0F, -1.0F) for c = -127..127: the entity format's signed byte normal components as vertex
// fetch decodes them
const int SNORM8[255] = int[](
        -1082130432, -1082262536, -1082394640, -1082526744, -1082658848, -1082790952, -1082923056, -1083055160,
        -1083187265, -1083319369, -1083451473, -1083583577, -1083715681, -1083847785, -1083979889, -1084111993,
        -1084244097, -1084376201, -1084508305, -1084640409, -1084772513, -1084904617, -1085036721, -1085168825,
        -1085300930, -1085433034, -1085565138, -1085697242, -1085829346, -1085961450, -1086093554, -1086225658,
        -1086357762, -1086489866, -1086621970, -1086754074, -1086886178, -1087018282, -1087150386, -1087282490,
        -1087414595, -1087546699, -1087678803, -1087810907, -1087943011, -1088075115, -1088207219, -1088339323,
        -1088471427, -1088603531, -1088735635, -1088867739, -1088999843, -1089131947, -1089264051, -1089396155,
        -1089528260, -1089660364, -1089792468, -1089924572, -1090056676, -1090188780, -1090320884, -1090452988,
        -1090651144, -1090915352, -1091179560, -1091443768, -1091707977, -1091972185, -1092236393, -1092500601,
        -1092764809, -1093029017, -1093293225, -1093557433, -1093821642, -1094085850, -1094350058, -1094614266,
        -1094878474, -1095142682, -1095406890, -1095671098, -1095935307, -1096199515, -1096463723, -1096727931,
        -1096992139, -1097256347, -1097520555, -1097784763, -1098048972, -1098313180, -1098577388, -1098841596,
        -1099303960, -1099832376, -1100360793, -1100889209, -1101417625, -1101946041, -1102474458, -1103002874,
        -1103531290, -1104059706, -1104588123, -1105116539, -1105644955, -1106173371, -1106701788, -1107230204,
        -1108220984, -1109277817, -1110334649, -1111391482, -1112448314, -1113505147, -1114561979, -1115618812,
        -1117666425, -1119780090, -1121893755, -1124007420, -1128168698, -1132396028, -1140784636, 0,
        1006699012, 1015087620, 1019314950, 1023476228, 1025589893, 1027703558, 1029817223, 1031864836,
        1032921669, 1033978501, 1035035334, 1036092166, 1037148999, 1038205831, 1039262664, 1040253444,
        1040781860, 1041310277, 1041838693, 1042367109, 1042895525, 1043423942, 1043952358, 1044480774,
        1045009190, 1045537607, 1046066023, 1046594439, 1047122855, 1047651272, 1048179688, 1048642052,
        1048906260, 1049170468, 1049434676, 1049698885, 1049963093, 1050227301, 1050491509, 1050755717,
        1051019925, 1051284133, 1051548341, 1051812550, 1052076758, 1052340966, 1052605174, 1052869382,
        1053133590, 1053397798, 1053662006, 1053926215, 1054190423, 1054454631, 1054718839, 1054983047,
        1055247255, 1055511463, 1055775671, 1056039880, 1056304088, 1056568296, 1056832504, 1057030660,
        1057162764, 1057294868, 1057426972, 1057559076, 1057691180, 1057823284, 1057955388, 1058087493,
        1058219597, 1058351701, 1058483805, 1058615909, 1058748013, 1058880117, 1059012221, 1059144325,
        1059276429, 1059408533, 1059540637, 1059672741, 1059804845, 1059936949, 1060069053, 1060201158,
        1060333262, 1060465366, 1060597470, 1060729574, 1060861678, 1060993782, 1061125886, 1061257990,
        1061390094, 1061522198, 1061654302, 1061786406, 1061918510, 1062050614, 1062182718, 1062314823,
        1062446927, 1062579031, 1062711135, 1062843239, 1062975343, 1063107447, 1063239551, 1063371655,
        1063503759, 1063635863, 1063767967, 1063900071, 1064032175, 1064164279, 1064296383, 1064428488,
        1064560592, 1064692696, 1064824800, 1064956904, 1065089008, 1065221112, 1065353216
);

// BufferBuilder.normalIntValue, then the vertex fetch's decode
float quantizedNormal(float c) {
    precise float scaled = clamp(c, -1.0, 1.0) * 127.0;
    return intBitsToFloat(SNORM8[int(scaled) + 127]);
}

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
    // Ruthenium: vanilla's core/entity.vsh for a part-space mesh posed per member and part
    int member = PartIndex.x;
    int row = PartIndex.y * 6;
    vec4 row0 = Members[member].Rows[row];
    vec4 row1 = Members[member].Rows[row + 1];
    vec4 row2 = Members[member].Rows[row + 2];
    // The same operations in the same order as JOML's Matrix4f.transformPosition (no fused multiply-add), so positions
    // are bit-identical to the vertices vanilla writes, and so are the depth ties with other geometry on the same faces
    precise vec3 pos;
    pos.x = row0.x * Position.x + (row0.y * Position.y + (row0.z * Position.z + row0.w));
    pos.y = row1.x * Position.x + (row1.y * Position.y + (row1.z * Position.z + row1.w));
    pos.z = row2.x * Position.x + (row2.y * Position.y + (row2.z * Position.z + row2.w));
    // The normal vanilla writes: PoseStack.Pose.transformNormal (JOML's Matrix3f.transform, then Vector3f.normalize unless
    // the pose's normals are trusted), quantized to signed bytes and decoded as the entity format's Normal attribute
    vec4 normal0 = Members[member].Rows[row + 3];
    vec4 normal1 = Members[member].Rows[row + 4];
    vec4 normal2 = Members[member].Rows[row + 5];
    precise vec3 transformedNormal;
    transformedNormal.x = normal0.x * Normal.x + (normal0.y * Normal.y + normal0.z * Normal.z);
    transformedNormal.y = normal1.x * Normal.x + (normal1.y * Normal.y + normal1.z * Normal.z);
    transformedNormal.z = normal2.x * Normal.x + (normal2.y * Normal.y + normal2.z * Normal.z);
    if (normal0.w != 0.0) {
        precise float lengthSquared = transformedNormal.x * transformedNormal.x + (transformedNormal.y * transformedNormal.y + transformedNormal.z * transformedNormal.z);
        precise float scale = 1.0 / sqrt(lengthSquared);
        transformedNormal = transformedNormal * scale;
    }
    vec3 normal = vec3(quantizedNormal(transformedNormal.x), quantizedNormal(transformedNormal.y), quantizedNormal(transformedNormal.z));
    vec4 color = Members[member].Color;
    ivec4 lightOverlay = Members[member].LightOverlay;
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
    lightMapColor = sample_lightmap(Sampler2, lightOverlay.xy);
#endif

#if !defined(NO_OVERLAY) && !defined(OIT_ALPHA_ONLY)
    overlayColor = texelFetch(Sampler1, lightOverlay.zw, 0);
#endif

    texCoord0 = UV0;

#ifdef APPLY_TEXTURE_MATRIX
    texCoord0 = (TextureMat * vec4(UV0, 0.0, 1.0)).xy;
#endif

#ifdef GLINT
    texCoordGlint = (TextureMat * vec4(UV0, 0.0, 1.0)).xy;
#endif
}
