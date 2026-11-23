#version 460 core
#extension GL_EXT_ray_query : require
#extension GL_EXT_buffer_reference : require
#extension GL_EXT_buffer_reference_uvec2 : require

// Ray traced sun shadows, one ray per pixel: from the surface the view
// shows toward a random point on the sun's disc (a new one every frame,
// so the frames blended later give soft shadows). Opaque geometry stops
// the ray; cut-out geometry (leaf cards) stops it where its opacity
// texture (a layer of `opacityAtlas`, found through the instance's record
// in `cutouts`) is at least the cut-off, as the drawn leaves are cut out.
// A cut-out without a record lets a share of the light through instead.
// Writes the light that arrives (r, 1 = all) and how far along the ray
// the first thing in the way was (g, metres; 0 when nothing was), which
// tells the filter how wide the penumbra is.

in vec2 uv;
out vec4 FragColor;

uniform accelerationStructureEXT scene;
uniform sampler2D sceneDepth;
// NDC to positions relative to the camera (homogeneous).
uniform mat4 inverseRelative;
uniform vec3 eye;
uniform vec3 toLight;
// Angular radius of the light's disc (radians).
uniform float lightRadius;
uniform float maxDistance;
// Share of the light a cut-out surface stops.
uniform float cutoutCoverage;
uniform int frame;
uniform sampler2DArray opacityAtlas;
// The wind as materials sway in it (`material::wind`): direction (xy),
// strength (z), time (w).
uniform vec4 wind;

// A mesh's buffers, read through their device addresses.
layout(buffer_reference, std430, buffer_reference_align = 4) readonly buffer Words {
    uint w[];
};

// As `CutoutRecord` in `raytracing/mod.rs`.
struct Cutout {
    uvec2 vertices;
    uvec2 indices;
    uint stride;
    uint uvOffset;
    uint layer;
    uint cutoff;
    uint indexed;
    // How the material sways (float bits; all 0 when the shadows do not
    // follow the wind): strength, flutter, bend height.
    uint windAmount;
    uint windFlutter;
    uint windHeight;
};
layout(std430, binding = 5) readonly buffer Cutouts {
    Cutout cutouts[];
};

const uint NO_CUTOUT = 0xFFFFFFu;

vec2 uvOf(Cutout c, uint vertex) {
    Words v = Words(c.vertices);
    uint at = vertex * c.stride + c.uvOffset;
    return uintBitsToFloat(uvec2(v.w[at], v.w[at + 1u]));
}

// The vertex's position, in the mesh's own space (first in each vertex).
vec3 positionOf(Cutout c, uint vertex) {
    Words v = Words(c.vertices);
    uint at = vertex * c.stride;
    return uintBitsToFloat(uvec3(v.w[at], v.w[at + 1u], v.w[at + 2u]));
}

// The wind's offset of a point, as `materialWind` (`vertex_prelude.glsl`)
// moves the vertices with both masks 1.
vec3 windOffset(vec3 worldPos, vec3 pivot, vec3 normal, float amount, float flutter, float height) {
    vec3 offset = vec3(0.0);
    float t = wind.w;
    if (amount > 0.0) {
        float H = max(height, 0.01);
        float h = max(worldPos.y - pivot.y, 0.0) / H;
        float phase = dot(pivot.xz, vec2(0.71, 0.37));
        float gust = 0.5 + 0.5 * sin(dot(pivot.xz, wind.xy) * 0.08 - t * 0.9);
        float sway = sin(t * 1.9 + phase) * 0.5 + sin(t * 3.7 + phase * 1.7) * 0.2 + gust * 0.7;
        offset.xz = wind.xy * (amount * wind.z * 0.35 * H * h * h * sway);
    }
    if (flutter > 0.0) {
        float phase = dot(worldPos, vec3(2.1, 1.7, 2.9));
        offset += normal * (flutter * wind.z * 0.03 * (sin(t * 9.0 + phase) + 0.5 * sin(t * 13.7 + phase * 1.3)));
    }
    return offset;
}

// Whether the candidate hit of `query` lands on the solid part of its
// cut-out: its UV looked up in the opacity atlas. The ray met the leaf
// at rest; where the material sways, the opacity is looked up where the
// swayed leaf would be met instead (the point moved back by the wind, in
// the leaf's plane), so the light through the leaves moves with them.
bool solidAt(rayQueryEXT query, Cutout c, vec3 origin, vec3 direction) {
    uint primitive = uint(rayQueryGetIntersectionPrimitiveIndexEXT(query, false));
    vec2 b = rayQueryGetIntersectionBarycentricsEXT(query, false);
    uvec3 corners = uvec3(primitive * 3u) + uvec3(0u, 1u, 2u);
    if (c.indexed == 1u) {
        Words i = Words(c.indices);
        corners = uvec3(i.w[corners.x], i.w[corners.y], i.w[corners.z]);
    }
    float amount = uintBitsToFloat(c.windAmount);
    float flutter = uintBitsToFloat(c.windFlutter);
    if (amount > 0.0 || flutter > 0.0) {
        mat4x3 toWorld = rayQueryGetIntersectionObjectToWorldEXT(query, false);
        vec3 p0 = toWorld * vec4(positionOf(c, corners.x), 1.0);
        vec3 e1 = toWorld * vec4(positionOf(c, corners.y), 1.0) - p0;
        vec3 e2 = toWorld * vec4(positionOf(c, corners.z), 1.0) - p0;
        vec3 n = cross(e1, e2);
        float area = dot(n, n);
        if (area > 1e-12) {
            n *= inversesqrt(area);
            vec3 hit = origin + direction * rayQueryGetIntersectionTEXT(query, false);
            vec3 d = windOffset(hit, toWorld[3], n, amount, flutter, uintBitsToFloat(c.windHeight));
            // Moved along its normal, the leaf is met further along the
            // ray; moved within its plane, another part of it is met.
            float s = dot(d, n);
            float dn = dot(direction, n);
            dn = abs(dn) < 0.2 ? (dn < 0.0 ? -0.2 : 0.2) : dn;
            vec3 rest = hit + direction * (s / dn) - d;
            // Its barycentric coordinates in the leaf's plane (past the
            // triangle's edges the texture goes on).
            vec3 r = rest - p0;
            float d11 = dot(e1, e1), d12 = dot(e1, e2), d22 = dot(e2, e2);
            float denom = d11 * d22 - d12 * d12;
            if (abs(denom) > 1e-12) {
                float r1 = dot(r, e1), r2 = dot(r, e2);
                b = vec2(d22 * r1 - d12 * r2, d11 * r2 - d12 * r1) / denom;
            }
        }
    }
    vec2 uv = uvOf(c, corners.x) * (1.0 - b.x - b.y) + uvOf(c, corners.y) * b.x + uvOf(c, corners.z) * b.y;
    float opacity = textureLod(opacityAtlas, vec3(uv, float(c.layer)), 0.0).r;
    return opacity >= uintBitsToFloat(c.cutoff);
}

vec3 relativeAt(ivec2 texel) {
    ivec2 size = textureSize(sceneDepth, 0);
    texel = clamp(texel, ivec2(0), size - 1);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    vec2 at = (vec2(texel) + 0.5) / vec2(size);
    vec4 p = inverseRelative * vec4(at * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
    return p.xyz / p.w;
}

// Noise that spreads well over neighbouring pixels (Jimenez), moved on
// every frame.
float noise(vec2 pixel, int index) {
    pixel += float((frame * 2 + index) % 64) * vec2(47.0, 17.0) * 0.695;
    return fract(52.9829189 * fract(dot(pixel, vec2(0.06711056, 0.00583715))));
}

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    if (depth >= 1.0) {
        // Sky.
        FragColor = vec4(1.0, 0.0, 0.0, 1.0);
        return;
    }
    vec3 position = relativeAt(texel);
    // The surface's normal from its neighbours, on the side where they lie
    // closer (so edges do not take the slope to what lies behind).
    vec3 left = relativeAt(texel - ivec2(1, 0));
    vec3 right = relativeAt(texel + ivec2(1, 0));
    vec3 down = relativeAt(texel - ivec2(0, 1));
    vec3 up = relativeAt(texel + ivec2(0, 1));
    vec3 dx = distance(left, position) < distance(right, position) ? position - left : right - position;
    vec3 dy = distance(down, position) < distance(up, position) ? position - down : up - position;
    vec3 normal = normalize(cross(dx, dy));
    // Toward the camera (the camera sits at the origin here).
    if (dot(normal, position) > 0.0) {
        normal = -normal;
    }

    // A random direction within the light's disc.
    vec3 l = normalize(toLight);
    vec3 tangent = normalize(abs(l.y) < 0.99 ? cross(l, vec3(0.0, 1.0, 0.0)) : cross(l, vec3(1.0, 0.0, 0.0)));
    vec3 bitangent = cross(l, tangent);
    float radius = lightRadius * sqrt(noise(gl_FragCoord.xy, 0));
    float angle = 6.2831853 * noise(gl_FragCoord.xy, 1);
    vec3 direction = normalize(l + (tangent * cos(angle) + bitangent * sin(angle)) * radius);

    // Off the surface by more the further it is (depth precision).
    float distanceToEye = length(position);
    vec3 origin = eye + position + normal * (0.02 + distanceToEye * 0.002);

    rayQueryEXT query;
    rayQueryInitializeEXT(query, scene, gl_RayFlagsTerminateOnFirstHitEXT, 0xFF, origin, 0.01, direction, maxDistance);
    float light = 1.0;
    float blocker = 0.0;
    while (rayQueryProceedEXT(query)) {
        // Only cut-outs come here; opaque hits end the query by themselves.
        if (rayQueryGetIntersectionTypeEXT(query, false) == gl_RayQueryCandidateIntersectionTriangleEXT) {
            uint record = uint(rayQueryGetIntersectionInstanceCustomIndexEXT(query, false));
            if (record != NO_CUTOUT) {
                if (solidAt(query, cutouts[record], origin, direction)) {
                    // With "terminate on first hit" this ends the query.
                    rayQueryConfirmIntersectionEXT(query);
                }
                continue;
            }
            if (blocker == 0.0) {
                blocker = rayQueryGetIntersectionTEXT(query, false);
            }
            light *= 1.0 - cutoutCoverage;
            if (light < 0.02) {
                rayQueryConfirmIntersectionEXT(query);
                rayQueryTerminateEXT(query);
            }
        }
    }
    if (rayQueryGetIntersectionTypeEXT(query, true) != gl_RayQueryCommittedIntersectionNoneEXT) {
        light = 0.0;
        float t = rayQueryGetIntersectionTEXT(query, true);
        blocker = blocker == 0.0 ? t : min(blocker, t);
    }
    FragColor = vec4(light, blocker, 0.0, 1.0);
}
