#version 460 core
#extension GL_EXT_ray_query : require

// Ray traced sun shadows, one ray per pixel: from the surface the view
// shows toward a random point on the sun's disc (a new one every frame,
// so the frames blended later give soft shadows). Opaque geometry stops
// the ray; cut-out geometry (leaf cards) lets part of the light through,
// as its texture is not looked at. Writes the light that arrives (1 = all).

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
        FragColor = vec4(1.0);
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
    while (rayQueryProceedEXT(query)) {
        // Only cut-outs come here; opaque hits end the query by themselves.
        if (rayQueryGetIntersectionTypeEXT(query, false) == gl_RayQueryCandidateIntersectionTriangleEXT) {
            light *= 1.0 - cutoutCoverage;
            if (light < 0.02) {
                rayQueryConfirmIntersectionEXT(query);
                rayQueryTerminateEXT(query);
            }
        }
    }
    if (rayQueryGetIntersectionTypeEXT(query, true) != gl_RayQueryCommittedIntersectionNoneEXT) {
        light = 0.0;
    }
    FragColor = vec4(light, light, light, 1.0);
}
