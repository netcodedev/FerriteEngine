// ReSTIR, second pass: the reservoirs of a few neighbours on the same
// surface merged into each pixel's, their lamps' light looked at anew for
// this pixel. Writes the reservoir the trace uses, and the next frame's
// history.

in vec2 uv;
out vec4 FragColor;

// The first pass's reservoirs.
uniform sampler2D reservoirs;
uniform int neighbours;
// Pixels around within which neighbours are taken.
uniform float spatialRadius;
// A neighbour stands for at most this many candidates.
uniform float neighbourCap;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    vec4 center = texelFetch(reservoirs, texel, 0);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    if (depth >= 1.0) {
        FragColor = center;
        return;
    }
    vec3 position = relativeAt(texel);
    vec3 N = normalAt(texel, position);
    vec3 P = eye + position;
    uint state = randomState(texel, 1);
    ivec2 size = textureSize(reservoirs, 0);

    Reservoir r = emptyReservoir();
    merge(r, center, P, N, 1e9, random(state));
    for (int n = 0; n < neighbours; ++n) {
        float angle = 6.2831853 * random(state);
        float radius = spatialRadius * sqrt(random(state));
        ivec2 t = texel + ivec2(round(vec2(cos(angle), sin(angle)) * radius));
        if (t == texel || any(lessThan(t, ivec2(0))) || any(greaterThanEqual(t, size))) {
            continue;
        }
        vec4 s = texelFetch(reservoirs, t, 0);
        // Only from the same surface (about the same distance).
        if (abs(s.w - center.w) > 0.05 * center.w + 0.05) {
            continue;
        }
        merge(r, s, P, N, neighbourCap, random(state));
    }
    FragColor = packReservoir(r, center.w);
}
