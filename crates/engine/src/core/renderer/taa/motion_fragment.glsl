#version 330 core

// See `motion_vertex.glsl`: the texel's motion since the last frame, in uv
// units, and its distance then and now, as `velocity.glsl` writes them.

in vec4 currentClip;
in vec4 previousClip;
out vec4 FragColor;

void main() {
    vec2 now = currentClip.xy / currentClip.w;
    vec2 before = previousClip.xy / max(previousClip.w, 1e-6);
    FragColor = vec4((now - before) * 0.5, previousClip.w, currentClip.w);
}
