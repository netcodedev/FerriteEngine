// ReSTIR, first pass: per pixel, a few lamps drawn at random from those
// its cluster lists (`lightsAt`), one kept in proportion to the light it
// would give (`lampTarget`); then the reservoir the pixel had a frame
// before (found by moving it with the camera, only where it showed the
// same surface) merged in. Writes the reservoir (`lamps.glsl`).

in vec2 uv;
out vec4 FragColor;

// The reservoirs of the frame before (after the spatial pass).
uniform sampler2D history;
// This frame's NDC to the last frame's clip space.
uniform mat4 reprojection;
// Row w of this frame's inverse view-projection.
uniform vec4 inverseW;
uniform int historyValid;
uniform int candidates;
// The history stands for at most this many times this frame's candidates.
uniform float historyCap;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    float now = clipW(inverseW, texel, depth);
    if (depth >= 1.0) {
        FragColor = vec4(-1.0, 0.0, 0.0, now);
        return;
    }
    vec3 position = relativeAt(texel);
    vec3 N = normalAt(texel, position);
    vec3 P = eye + position;
    uint state = randomState(texel, 0);

    Reservoir r = emptyReservoir();
    ivec2 list = lightsAt(P);
    if (list.y > 0) {
        for (int c = 0; c < candidates; ++c) {
            int k = min(int(random(state) * float(list.y)), list.y - 1);
            int i = lightAt(list, k);
            float target = lampTarget(i, P, N);
            // Drawn with probability 1 / list.y.
            update(r, i, target, target * float(list.y), 1.0, random(state));
        }
    }

    if (historyValid == 1) {
        vec2 at = (vec2(texel) + 0.5) / vec2(textureSize(sceneDepth, 0));
        vec4 ndc = vec4(at * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
        vec4 previous = reprojection * ndc;
        if (previous.w > 1e-6) {
            vec2 previousUv = previous.xy / previous.w * 0.5 + 0.5;
            ivec2 size = textureSize(history, 0);
            ivec2 t = ivec2(floor(previousUv * vec2(size)));
            if (all(greaterThanEqual(t, ivec2(0))) && all(lessThan(t, size))) {
                vec4 s = texelFetch(history, t, 0);
                // The surface's distance a frame before.
                float expected = previous.w * now;
                if (abs(s.w - expected) <= 0.03 * expected + 0.05) {
                    merge(r, s, P, N, historyCap * float(max(candidates, 1)), random(state));
                }
            }
        }
    }
    FragColor = packReservoir(r, now);
}
