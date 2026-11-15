// Particles (`renderer/particles`): one camera-facing quad per instance.
// Instances: position and size, colour (linear) and alpha, velocity and
// rotation, a random seed and how far through its life it is.

#ifdef VERTEX
layout(location = 0) in vec4 positionSize;
layout(location = 1) in vec4 color;
layout(location = 2) in vec4 velocityRotation;
layout(location = 3) in vec4 seedLife;

uniform mat4 viewProjection;
uniform vec3 camRight;
uniform vec3 camUp;
uniform vec3 eye;
uniform int look;        // 0 soft, 1 smoke, 2 spark
uniform float stretch;   // sparks: extra length per speed

out vec2 corner;
out vec4 particleColor;
out vec3 worldPos;
out float seed;

void main() {
    const vec2 corners[6] = vec2[](vec2(-1, -1), vec2(1, -1), vec2(1, 1),
                                   vec2(-1, -1), vec2(1, 1), vec2(-1, 1));
    corner = corners[gl_VertexID];
    vec3 center = positionSize.xyz;
    float half_ = positionSize.w * 0.5;
    vec3 offset;
    if (look == 2) {
        // A streak along the velocity as the camera sees it.
        vec3 v = velocityRotation.xyz;
        vec3 toEye = normalize(eye - center);
        vec3 along = v - toEye * dot(v, toEye);
        float speed = length(along);
        vec3 axis = speed > 1e-4 ? along / speed : camUp;
        vec3 side = normalize(cross(axis, toEye));
        float len = half_ + speed * stretch;
        offset = axis * corner.y * len + side * corner.x * half_;
    } else {
        float c = cos(velocityRotation.w);
        float s = sin(velocityRotation.w);
        vec2 r = vec2(c * corner.x - s * corner.y, s * corner.x + c * corner.y);
        offset = (camRight * r.x + camUp * r.y) * half_;
    }
    worldPos = center + offset;
    particleColor = color;
    seed = seedLife.x;
    gl_Position = viewProjection * vec4(worldPos, 1.0);
}
#endif

#ifdef FRAGMENT
in vec2 corner;
in vec4 particleColor;
in vec3 worldPos;
in float seed;
out vec4 FragColor;

uniform int look;
uniform int lit;
uniform int additive;
uniform float emissive;
// Fading into the opaque scene (`hdr::opaque_scene`): its depth, and the
// projection's terms to turn depth into distance.
uniform int soft;
uniform float softness;
uniform sampler2D sceneDepth;
uniform float projZ;     // projection[2][2]
uniform float projW;     // projection[3][2]

float hashP(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

float valueNoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hashP(i);
    float b = hashP(i + vec2(1.0, 0.0));
    float c = hashP(i + vec2(0.0, 1.0));
    float d = hashP(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float distanceOf(float depth) {
    return projW / (depth * 2.0 - 1.0 + projZ);
}

void main() {
    float r = length(corner);
    float alpha;
    if (look == 1) {
        // A ragged puff: the edge pushed in and out by noise, its own for
        // every particle.
        vec2 q = corner * 1.6 + vec2(seed * 37.0, seed * 91.0);
        float n = valueNoise(q) * 0.6 + valueNoise(q * 2.3) * 0.4;
        alpha = 1.0 - smoothstep(0.25, 1.0, r + (n - 0.5) * 0.7);
        alpha *= 0.55 + 0.45 * n;
    } else if (look == 2) {
        alpha = (1.0 - corner.x * corner.x) * (1.0 - smoothstep(0.5, 1.0, abs(corner.y)));
    } else {
        alpha = 1.0 - smoothstep(0.0, 1.0, r);
        alpha *= alpha;
    }
    alpha *= particleColor.a;

    if (soft == 1) {
        vec2 size = vec2(textureSize(sceneDepth, 0));
        float behind = distanceOf(texelFetch(sceneDepth, ivec2(gl_FragCoord.xy), 0).r);
        float here = distanceOf(gl_FragCoord.z);
        alpha *= clamp((behind - here) / softness, 0.0, 1.0);
    }
    if (alpha <= 0.002) {
        discard;
    }

    vec3 rgb = particleColor.rgb;
    vec3 light = vec3(emissive);
    if (lit == 1) {
        // Lit all round: the sky above, the sun and moon (behind the
        // shadow map), the lamps near it.
        vec3 L = normalize(sunDirection);
        float sunLit = sunShadow(worldPos, vec3(0.0), L);
        float moonLit = moonShadows == 1 ? sunShadow(worldPos, vec3(0.0), normalize(moonDirection)) : 1.0;
        light += skyLight(vec3(0.0, 1.0, 0.0)) + sunRadiance() * 0.35 * sunLit
            + moonRadiance() * 0.35 * moonLit + localLightAt(worldPos) * 0.35;
    }
    vec3 color = rgb * light;
    if (additive == 0) {
        color = weatherFog(color, worldPos);
    }
    // Premultiplied: additive effects leave alpha 0 (only add light).
    FragColor = vec4(toDisplay(color) * alpha, additive == 1 ? 0.0 : alpha);
}
#endif
