// Ray traced ambient occlusion: a few rays per pixel into the hemisphere
// above the surface (cosine weighted, a new set every frame), each as
// long as the occlusion's radius. What a ray meets darkens the pixel, the
// more the closer it is. Writes the share of the sky the surface sees
// past what is around it (rgb, as `light_trace.glsl` does, so the same
// denoising passes follow), a 1.
// `surface.glsl` and `rays.glsl` come before this.

in vec2 uv;
out vec4 FragColor;

uniform float radius;
// Exponent on the result: above 1 darkens corners further (as SSAO).
uniform float intensity;
uniform int rays;

// Beyond this distance the occlusion fades out, as SSAO's does.
const float FADE_START = 80.0;
const float FADE_END = 150.0;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    if (depth >= 1.0) {
        FragColor = vec4(1.0, 1.0, 1.0, 0.0);
        return;
    }
    vec3 position = relativeAt(texel);
    float distanceToEye = length(position);
    if (distanceToEye >= FADE_END) {
        FragColor = vec4(1.0, 1.0, 1.0, 0.0);
        return;
    }
    vec3 N = normalAt(texel, position);
    vec3 tangent = normalize(abs(N.y) < 0.99 ? cross(N, vec3(0.0, 1.0, 0.0)) : cross(N, vec3(1.0, 0.0, 0.0)));
    vec3 bitangent = cross(N, tangent);
    vec3 origin = eye + position + N * (0.01 + distanceToEye * 0.001);
    // What the rays see is drawn coarser further away (scatter is
    // collected a level of detail coarser than it is drawn; far trees are
    // drawn as impostor cards): a ray would start inside the thicker trunk
    // and darken it. So the first stretch of each ray, growing with the
    // distance, is skipped.
    float skip = distanceToEye * 0.006;
    uint state = randomState(texel, 2);

    float open = 0.0;
    for (int i = 0; i < rays; ++i) {
        // Cosine weighted: a point on the unit disc lifted onto the
        // hemisphere. The first ray's disc point from the blue noise.
        float u = i == 0 ? noise(gl_FragCoord.xy, 0) : random(state);
        float v = i == 0 ? noise(gl_FragCoord.xy, 1) : random(state);
        float r = sqrt(u);
        float angle = 6.2831853 * v;
        vec2 disc = r * vec2(cos(angle), sin(angle));
        vec3 direction = normalize(tangent * disc.x + bitangent * disc.y + N * sqrt(max(1.0 - u, 0.0)));
        float blocker;
        float reach = radius - skip;
        if (reach <= 0.01) {
            open += 1.0;
            continue;
        }
        float through = traceShadow(origin + direction * skip, direction, reach, blocker);
        // Something far along darkens less (it hides little of the sky).
        float t = (blocker + skip) / radius;
        float near = blocker > 0.0 ? 1.0 - t * t : 0.0;
        open += 1.0 - (1.0 - through) * near;
    }
    float ao = pow(clamp(open / float(max(rays, 1)), 0.0, 1.0), intensity);
    ao = mix(ao, 1.0, smoothstep(FADE_START, FADE_END, distanceToEye));
    FragColor = vec4(ao, ao, ao, 1.0);
}
