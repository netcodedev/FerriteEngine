#version 330 core

// Ambient occlusion from the camera's depth: for every pixel, how many
// points in the hemisphere above its surface lie behind other geometry.

in vec2 uv;
out float occlusion;

const int SAMPLES = 16;
// Beyond this distance the occlusion fades out: depth gets coarse and a
// metre-sized radius covers less than a pixel.
const float FADE_START = 80.0;
const float FADE_END = 150.0;

uniform sampler2D depthMap;
uniform mat4 projection;
uniform mat4 inverseProjection;
uniform vec3 kernel[SAMPLES];
uniform float radius;
uniform float intensity;

// 4x4 ordered dither: each pixel of a 4x4 block turns the samples by a
// different angle, and the 4x4 blur after this averages the pattern away.
const int BAYER[16] = int[16](0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5);

vec3 viewPosition(ivec2 texel, ivec2 size) {
    float depth = texelFetch(depthMap, clamp(texel, ivec2(0), size - 1), 0).r;
    vec2 ndc = (vec2(texel) + 0.5) / vec2(size) * 2.0 - 1.0;
    vec4 view = inverseProjection * vec4(ndc, depth * 2.0 - 1.0, 1.0);
    return view.xyz / view.w;
}

void main() {
    ivec2 size = textureSize(depthMap, 0);
    ivec2 texel = ivec2(gl_FragCoord.xy);
    if (texelFetch(depthMap, texel, 0).r >= 1.0) {
        occlusion = 1.0; // sky
        return;
    }
    vec3 P = viewPosition(texel, size);
    float distance = -P.z;
    if (distance > FADE_END) {
        occlusion = 1.0;
        return;
    }

    // Surface normal from the neighbours, from the side that continues the
    // surface (the smaller depth step), so edges do not bend it.
    vec3 left = P - viewPosition(texel - ivec2(1, 0), size);
    vec3 right = viewPosition(texel + ivec2(1, 0), size) - P;
    vec3 down = P - viewPosition(texel - ivec2(0, 1), size);
    vec3 up = viewPosition(texel + ivec2(0, 1), size) - P;
    vec3 dx = abs(left.z) < abs(right.z) ? left : right;
    vec3 dy = abs(down.z) < abs(up.z) ? down : up;
    vec3 N = normalize(cross(dx, dy));
    if (dot(N, P) > 0.0) {
        N = -N; // toward the camera
    }

    float angle = (float(BAYER[(texel.y & 3) * 4 + (texel.x & 3)]) + 0.5) / 16.0 * 6.2831853;
    vec3 turn = vec3(cos(angle), sin(angle), 0.0);
    vec3 T = turn - N * dot(turn, N);
    T = dot(T, T) < 1e-4 ? normalize(cross(N, vec3(0.0, 1.0, 0.0))) : normalize(T);
    mat3 TBN = mat3(T, cross(N, T), N);

    // Farther away a depth step covers more distance; the bias grows along.
    float bias = 0.02 + distance * 0.0015;
    float occluded = 0.0;
    for (int i = 0; i < SAMPLES; ++i) {
        vec3 S = P + TBN * kernel[i] * radius;
        vec4 clip = projection * vec4(S, 1.0);
        vec2 at = clip.xy / clip.w * 0.5 + 0.5;
        if (any(lessThan(at, vec2(0.0))) || any(greaterThan(at, vec2(1.0)))) {
            continue;
        }
        float sceneZ = viewPosition(ivec2(at * vec2(size)), size).z;
        // Geometry far in front of the point (a tree before a hill) does
        // not shade it.
        float inRange = smoothstep(0.0, 1.0, radius / max(abs(P.z - sceneZ), 1e-4));
        occluded += (sceneZ >= S.z + bias ? 1.0 : 0.0) * inRange;
    }
    float ao = pow(clamp(1.0 - occluded / float(SAMPLES), 0.0, 1.0), intensity);
    occlusion = mix(ao, 1.0, smoothstep(FADE_START, FADE_END, distance));
}
