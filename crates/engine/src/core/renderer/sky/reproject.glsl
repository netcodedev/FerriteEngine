// Cloud reprojection: the full cloud texture from the quarter of its texels
// drawn this frame (`freshClouds`, one of each 2x2 block, at `phase`) and
// the full texture of the frame before (`history`). Texels not drawn this
// frame take the history where the same direction was on screen then,
// held within the range of the fresh texels around them (so what moved or
// came out from behind the terrain does not leave a trail), or else the
// fresh texels blended. The clouds are so far away that only the camera's
// turning moves them on screen. `common.glsl` comes before this.

in vec2 ndcPos;
out vec4 FragColor;

uniform sampler2D freshClouds;
uniform sampler2D history;
uniform ivec2 phase;

// The camera of the frame before.
uniform vec3 prevForward;
uniform vec3 prevRight;
uniform vec3 prevUp;
uniform float prevTanHalfFovX;
uniform float prevTanHalfFovY;

void main() {
    ivec2 pixel = ivec2(gl_FragCoord.xy);
    ivec2 block = pixel / 2;
    ivec2 last = textureSize(freshClouds, 0) - 1;
    vec4 own = texelFetch(freshClouds, clamp(block, ivec2(0), last), 0);
    if (pixel - block * 2 == phase) {
        FragColor = own;
        return;
    }

    vec4 lo = own;
    vec4 hi = own;
    for (int y = -1; y <= 1; ++y) {
        for (int x = -1; x <= 1; ++x) {
            vec4 c = texelFetch(freshClouds, clamp(block + ivec2(x, y), ivec2(0), last), 0);
            lo = min(lo, c);
            hi = max(hi, c);
        }
    }
    // The fresh texels around, blended: where the history has nothing.
    vec2 freshUv = ((gl_FragCoord.xy - vec2(phase) - 0.5) * 0.5 + 0.5) / vec2(last + 1);
    vec4 blended = texture(freshClouds, freshUv);

    vec3 dir = viewRay(ndcPos);
    float ahead = dot(dir, prevForward);
    if (ahead <= 1e-4) {
        FragColor = blended;
        return;
    }
    vec2 before = vec2(dot(dir, prevRight) / (ahead * prevTanHalfFovX),
                       dot(dir, prevUp) / (ahead * prevTanHalfFovY));
    if (any(greaterThan(abs(before), vec2(1.0)))) {
        FragColor = blended;
        return;
    }
    FragColor = clamp(texture(history, before * 0.5 + 0.5), lo, hi);
}
