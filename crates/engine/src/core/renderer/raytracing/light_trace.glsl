// ReSTIR, third pass: one shadow ray per pixel toward a random point on
// the lamp its reservoir chose (`radius` big), and from it the share of
// the shadow casting lamps' light that arrives here, per colour: that
// lamp's light past what is in the way, times the reservoir's weight,
// divided by all those lamps' light with nothing in the way (Heitz et al.
// 2018). Writes the share (rgb) and whether any such lamp reaches here
// (a).

in vec2 uv;
out vec4 FragColor;

uniform sampler2D reservoirs;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    if (depth >= 1.0) {
        FragColor = vec4(1.0, 1.0, 1.0, 0.0);
        return;
    }
    vec3 position = relativeAt(texel);
    vec3 N = normalAt(texel, position);
    vec3 P = eye + position;

    // Every shadow casting lamp's light here, with nothing in the way.
    vec3 unshadowed = vec3(0.0);
    ivec2 list = lightsAt(P);
    for (int k = 0; k < list.y; ++k) {
        unshadowed += lampIrradiance(lightAt(list, k), P, N);
    }
    if (max(unshadowed.r, max(unshadowed.g, unshadowed.b)) <= 1e-7) {
        FragColor = vec4(1.0, 1.0, 1.0, 0.0);
        return;
    }
    vec4 s = texelFetch(reservoirs, texel, 0);
    int lamp = int(s.x + 0.5 * sign(s.x));
    vec3 light = lampIrradiance(lamp, P, N);
    if (s.x < 0.0 || s.y <= 0.0 || dot(light, light) <= 0.0) {
        // No lamp chosen (rare: all candidates gave nothing): unknown, lit.
        FragColor = vec4(1.0, 1.0, 1.0, 1.0);
        return;
    }

    // A random point on the lamp's disc as seen from here.
    Light l = lights[lamp];
    vec3 origin = P + N * (0.02 + length(position) * 0.002);
    vec3 toCenter = l.positionRange.xyz - origin;
    vec3 axis = normalize(toCenter);
    vec3 tangent = normalize(abs(axis.y) < 0.99 ? cross(axis, vec3(0.0, 1.0, 0.0)) : cross(axis, vec3(1.0, 0.0, 0.0)));
    vec3 bitangent = cross(axis, tangent);
    float r = l.cone.w * sqrt(noise(gl_FragCoord.xy, 0));
    float angle = 6.2831853 * noise(gl_FragCoord.xy, 1);
    vec3 target = l.positionRange.xyz + (tangent * cos(angle) + bitangent * sin(angle)) * r;
    vec3 toTarget = target - origin;
    float reach = length(toTarget);
    // Short of the lamp itself, so its own bulb or glass does not count.
    float tMax = reach - 0.05;
    float visible = 1.0;
    if (tMax > 0.01) {
        float blocker;
        visible = traceShadow(origin, toTarget / reach, tMax, blocker);
    }
    vec3 estimate = light * visible * s.y;
    vec3 ratio = vec3(
        unshadowed.r > 1e-7 ? estimate.r / unshadowed.r : 1.0,
        unshadowed.g > 1e-7 ? estimate.g / unshadowed.g : 1.0,
        unshadowed.b > 1e-7 ? estimate.b / unshadowed.b : 1.0);
    // An unbiased share may pass 1 for a frame; far past it is noise.
    FragColor = vec4(clamp(ratio, vec3(0.0), vec3(4.0)), 1.0);
}
