// What tracing rays takes (after `surface.glsl` in the ray traced passes,
// under `RAY_QUERIES` in the lit shaders): the scene's top level
// structure, a record per surface with its buffers, cut-out and look, the
// atlases of opacity and colour, a shadow ray (`traceShadow`) and the
// nearest hit with what it looks like (`traceClosest`).

uniform accelerationStructureEXT scene;
// Share of the light a cut-out surface stops whose texture is not tested.
uniform float cutoutCoverage;
uniform sampler2DArray opacityAtlas;
uniform sampler2DArray albedoAtlas;
// The ground's colour seen from above (`raytracing::ground`), sRGB: a near
// level and one over the loaded ground, with the matrices they were drawn
// with. 0 in `useGroundMaps` until both are drawn.
uniform sampler2D groundNear;
uniform sampler2D groundFar;
uniform mat4 groundNearMatrix;
uniform mat4 groundFarMatrix;
uniform int useGroundMaps;
// The wind as materials sway in it (`material::wind`): direction (xy),
// strength (z), time (w).
uniform vec4 rayWind;

// A mesh's buffers, read through their device addresses.
layout(buffer_reference, std430, buffer_reference_align = 4) readonly buffer Words {
    uint w[];
};

// As `SurfaceRecord` in `raytracing/mod.rs`.
struct Cutout {
    uvec2 vertices;
    uvec2 indices;
    uint stride;
    uint uvOffset;      // NONE: no UV
    uint layer;         // opacity atlas layer, NONE: not tested
    uint cutoff;
    uint indexed;
    // How the material sways (float bits; all 0 when the shadows do not
    // follow the wind): strength, flutter, bend height.
    uint windAmount;
    uint windFlutter;
    uint windHeight;
    uint colorOffset;   // the ground's vertex colour, NONE: none
    uint albedoLayer;   // albedo atlas layer, NONE: none
    uint baseColor[4];
    uint emissive[3];
    uint uvTransform[4];
    uint pad;
};
layout(std430, binding = 5) readonly buffer Cutouts {
    Cutout cutouts[];
};

const uint NO_CUTOUT = 0xFFFFFFu;
const uint NONE = 0xFFFFFFFFu;

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
    float t = rayWind.w;
    if (amount > 0.0) {
        float H = max(height, 0.01);
        float h = max(worldPos.y - pivot.y, 0.0) / H;
        float phase = dot(pivot.xz, vec2(0.71, 0.37));
        float gust = 0.5 + 0.5 * sin(dot(pivot.xz, rayWind.xy) * 0.08 - t * 0.9);
        float sway = sin(t * 1.9 + phase) * 0.5 + sin(t * 3.7 + phase * 1.7) * 0.2 + gust * 0.7;
        offset.xz = rayWind.xy * (amount * rayWind.z * 0.35 * H * h * h * sway);
    }
    if (flutter > 0.0) {
        float phase = dot(worldPos, vec3(2.1, 1.7, 2.9));
        offset += normal * (flutter * rayWind.z * 0.03 * (sin(t * 9.0 + phase) + 0.5 * sin(t * 13.7 + phase * 1.3)));
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

// A ray from `origin` (world) along `direction` up to `tMax`: the light
// that gets through (1 all, 0 none). Opaque geometry stops it; cut-outs
// where their opacity texture is solid, or by `cutoutCoverage` without a
// record. `blocker`: how far along the first thing in the way was (0 when
// nothing was).
float traceShadow(vec3 origin, vec3 direction, float tMax, out float blocker) {
    rayQueryEXT query;
    rayQueryInitializeEXT(query, scene, gl_RayFlagsTerminateOnFirstHitEXT, 0xFF, origin, 0.01, direction, tMax);
    float light = 1.0;
    blocker = 0.0;
    while (rayQueryProceedEXT(query)) {
        // Only cut-outs come here; opaque hits end the query by themselves.
        if (rayQueryGetIntersectionTypeEXT(query, false) == gl_RayQueryCandidateIntersectionTriangleEXT) {
            uint record = uint(rayQueryGetIntersectionInstanceCustomIndexEXT(query, false));
            if (record != NO_CUTOUT && cutouts[record].layer != NONE) {
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
    return light;
}

// The ground's colour at `P` from the maps above (a 1 where found): the
// near level for hits near the ray's start (`t` metres along), past that
// the far one, which also stands in where the near one ends.
vec4 groundAlbedo(vec3 P, float t) {
    if (useGroundMaps == 0) {
        return vec4(0.0);
    }
    vec4 c = groundNearMatrix * vec4(P, 1.0);
    vec2 uv = c.xy / c.w * 0.5 + 0.5;
    if (t < 60.0 && all(greaterThan(uv, vec2(0.002))) && all(lessThan(uv, vec2(0.998)))) {
        vec4 near = textureLod(groundNear, uv, 0.0);
        if (near.a > 0.5) {
            return vec4(near.rgb, 1.0);
        }
    }
    c = groundFarMatrix * vec4(P, 1.0);
    uv = c.xy / c.w * 0.5 + 0.5;
    if (all(greaterThan(uv, vec2(0.0))) && all(lessThan(uv, vec2(1.0)))) {
        vec4 far = textureLod(groundFar, uv, 0.0);
        if (far.a > 0.5) {
            return vec4(far.rgb, 1.0);
        }
    }
    return vec4(0.0);
}

// Where a ray met the scene and what is there.
struct Hit {
    vec3 position;
    vec3 normal;    // of the triangle, toward the ray's origin
    vec3 albedo;    // linear
    vec3 emissive;
};

vec3 srgbToLinearRay(vec3 c) {
    return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(vec3(0.04045), c));
}

// The vertex's three floats at word `offset` (the ground's colour).
vec3 vec3Of(Cutout c, uint vertex, uint offset) {
    Words v = Words(c.vertices);
    uint at = vertex * c.stride + offset;
    return uintBitsToFloat(uvec3(v.w[at], v.w[at + 1u], v.w[at + 2u]));
}

// The nearest thing along the ray from `origin` up to `tMax`: opaque
// geometry, cut-outs where their opacity texture is solid (whole where
// the rays cannot test them). False when nothing is there.
bool traceClosest(vec3 origin, vec3 direction, float tMax, out Hit hit) {
    rayQueryEXT query;
    rayQueryInitializeEXT(query, scene, gl_RayFlagsNoneEXT, 0xFF, origin, 0.01, direction, tMax);
    while (rayQueryProceedEXT(query)) {
        if (rayQueryGetIntersectionTypeEXT(query, false) == gl_RayQueryCandidateIntersectionTriangleEXT) {
            uint record = uint(rayQueryGetIntersectionInstanceCustomIndexEXT(query, false));
            if (record == NO_CUTOUT || cutouts[record].layer == NONE || solidAt(query, cutouts[record], origin, direction)) {
                rayQueryConfirmIntersectionEXT(query);
            }
        }
    }
    if (rayQueryGetIntersectionTypeEXT(query, true) == gl_RayQueryCommittedIntersectionNoneEXT) {
        return false;
    }
    float t = rayQueryGetIntersectionTEXT(query, true);
    hit.position = origin + direction * t;
    hit.albedo = vec3(0.6);
    hit.emissive = vec3(0.0);
    hit.normal = -direction;
    uint record = uint(rayQueryGetIntersectionInstanceCustomIndexEXT(query, true));
    if (record == NO_CUTOUT) {
        return true;
    }
    Cutout c = cutouts[record];
    uint primitive = uint(rayQueryGetIntersectionPrimitiveIndexEXT(query, true));
    vec2 b = rayQueryGetIntersectionBarycentricsEXT(query, true);
    vec3 w = vec3(1.0 - b.x - b.y, b.x, b.y);
    uvec3 corners = uvec3(primitive * 3u) + uvec3(0u, 1u, 2u);
    if (c.indexed == 1u) {
        Words i = Words(c.indices);
        corners = uvec3(i.w[corners.x], i.w[corners.y], i.w[corners.z]);
    }
    mat4x3 toWorld = rayQueryGetIntersectionObjectToWorldEXT(query, true);
    vec3 p0 = toWorld * vec4(positionOf(c, corners.x), 1.0);
    vec3 p1 = toWorld * vec4(positionOf(c, corners.y), 1.0);
    vec3 p2 = toWorld * vec4(positionOf(c, corners.z), 1.0);
    vec3 n = cross(p1 - p0, p2 - p0);
    if (dot(n, n) > 1e-20) {
        n = normalize(n);
        hit.normal = dot(n, direction) > 0.0 ? -n : n;
    }
    vec4 base = vec4(uintBitsToFloat(c.baseColor[0]), uintBitsToFloat(c.baseColor[1]),
                     uintBitsToFloat(c.baseColor[2]), uintBitsToFloat(c.baseColor[3]));
    vec3 albedo = base.rgb;
    if (c.colorOffset != NONE) {
        // The ground: as drawn, from the maps above; past them the
        // vertices' colour.
        vec4 mapped = groundAlbedo(hit.position, t);
        if (mapped.a > 0.0) {
            albedo = mapped.rgb;
        } else {
            vec3 color = vec3Of(c, corners.x, c.colorOffset) * w.x + vec3Of(c, corners.y, c.colorOffset) * w.y
                + vec3Of(c, corners.z, c.colorOffset) * w.z;
            albedo *= srgbToLinearRay(clamp(color, 0.0, 1.0));
        }
    } else if (c.albedoLayer != NONE && c.uvOffset != NONE) {
        vec2 uv = uvOf(c, corners.x) * w.x + uvOf(c, corners.y) * w.y + uvOf(c, corners.z) * w.z;
        vec4 transform = vec4(uintBitsToFloat(c.uvTransform[0]), uintBitsToFloat(c.uvTransform[1]),
                              uintBitsToFloat(c.uvTransform[2]), uintBitsToFloat(c.uvTransform[3]));
        uv = uv * transform.xy + transform.zw;
        albedo *= textureLod(albedoAtlas, vec3(uv, float(c.albedoLayer)), 0.0).rgb;
    }
    hit.albedo = albedo;
    hit.emissive = vec3(uintBitsToFloat(c.emissive[0]), uintBitsToFloat(c.emissive[1]), uintBitsToFloat(c.emissive[2]));
    return true;
}
