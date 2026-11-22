#version 330 core

// Motion vectors of the camera: for every texel, where its surface was on
// the screen the frame before, from the scene's depth. `reprojection`
// takes this frame's NDC to the last frame's clip space (both without the
// jitter). Written as `uv - previousUv`, in uv units, with the surface's
// distance from the camera then (b) and now (a; clip w), which tells TAA
// where its history shows something else (what was revealed). Moving
// objects draw their own over this afterwards (see `renderer::taa`).

in vec2 uv;
out vec4 FragColor;

uniform sampler2D sceneDepth;
uniform mat4 reprojection;
// Row w of this frame's inverse view-projection: 1 / clip w from NDC.
uniform vec4 inverseW;

void main() {
    float depth = texelFetch(sceneDepth, ivec2(gl_FragCoord.xy), 0).r;
    vec4 ndc = vec4(uv * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
    vec4 previous = reprojection * ndc;
    vec2 previousUv = previous.w > 1e-6 ? previous.xy / previous.w * 0.5 + 0.5 : uv;
    // `previous` is the last frame's clip position over this frame's w.
    float now = 1.0 / max(dot(inverseW, ndc), 1e-6);
    FragColor = vec4(uv - previousUv, previous.w * now, now);
}
