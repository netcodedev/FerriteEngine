// Falling rain streaks and snowflakes in a box around the camera
// (`weather/precipitation.rs`). Each instance is one drop or flake, placed
// by its id: it falls through the box and wraps around, so the box follows
// the camera without the drops moving with it.

#ifdef VERTEX
uniform mat4 viewProjection;
uniform vec3 eye;
uniform float time;
uniform int snowing;
// Metres per side of the box around the camera.
uniform float boxSize;
uniform vec3 wind;

// The sky exposure map: no drops under a roof.
uniform int useExposure;
uniform sampler2D exposureMap;
uniform mat4 exposureMatrix;

out vec2 corner;
out float strength;

vec3 hash3(uint n) {
    n = (n << 13U) ^ n;
    n = n * (n * n * 15731U + 789221U) + 1376312589U;
    uvec3 k = n * uvec3(n, n * 16807U, n * 48271U);
    return vec3((k >> 8U) & uvec3(0x7fffffffU)) / float(0x7fffffffU >> 8U);
}

void main() {
    vec3 r = fract(hash3(uint(gl_InstanceID) + 1U));
    float speed = snowing == 1 ? 0.9 + 0.6 * r.z : 8.0 + 3.0 * r.z;
    // Flakes are carried by the wind more slowly than drops.
    float carried = snowing == 1 ? 0.5 : 1.0;
    vec3 velocity = vec3(wind.x * carried, -speed, wind.z * carried);
    vec3 p = r * boxSize * 7.0 + velocity * time;
    if (snowing == 1) {
        // Flakes drift and tumble.
        p.x += sin(time * 0.8 + r.x * 40.0) * 0.4;
        p.z += cos(time * 0.7 + r.y * 40.0) * 0.4;
    }
    p = eye + mod(p - eye, boxSize) - boxSize * 0.5;
    float distance = length(p - eye);
    // Fade out at the box's edge, and right at the camera, where a streak
    // would cross the whole view.
    strength = (1.0 - smoothstep(0.3, 0.5, distance / boxSize)) * smoothstep(0.8, 2.5, distance);

    if (useExposure == 1 && strength > 0.0) {
        vec4 clip = exposureMatrix * vec4(p, 1.0);
        vec3 q = clip.xyz / clip.w * 0.5 + 0.5;
        if (all(greaterThan(q.xy, vec2(0.0))) && all(lessThan(q.xy, vec2(1.0)))) {
            if (q.z > texture(exposureMap, q.xy).r + 0.0005) {
                strength = 0.0;
            }
        }
    }

    const vec2 corners[6] = vec2[](vec2(-1, -1), vec2(1, -1), vec2(1, 1),
                                   vec2(-1, -1), vec2(1, 1), vec2(-1, 1));
    corner = corners[gl_VertexID];
    vec3 toEye = normalize(eye - p);
    vec3 world;
    if (snowing == 1) {
        vec3 right = normalize(cross(vec3(0.0, 1.0, 0.0), toEye));
        vec3 up = cross(toEye, right);
        float size = 0.010 + 0.012 * r.x;
        world = p + (right * corner.x + up * corner.y) * size;
    } else {
        // A streak along the drop's path, as long as it falls in a frame or
        // two; at least about a pixel wide, fainter when widened.
        vec3 axis = normalize(velocity);
        vec3 side = normalize(cross(axis, toEye));
        float width = max(0.004, distance * 0.0009);
        strength *= 0.004 / width;
        float len = 0.35 + 0.2 * r.y;
        world = p + axis * corner.y * len * 0.5 + side * corner.x * width;
    }
    if (strength <= 0.001) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        return;
    }
    gl_Position = viewProjection * vec4(world, 1.0);
}
#endif

#ifdef FRAGMENT
in vec2 corner;
in float strength;
out vec4 FragColor;
uniform int snowing;

void main() {
    vec3 up = vec3(0.0, 1.0, 0.0);
    if (snowing == 1) {
        float d = length(corner);
        float a = 1.0 - smoothstep(0.4, 1.0, d);
        vec3 light = skyLight(up) + sunRadiance() * 0.25;
        FragColor = vec4(SNOW_ALBEDO * light, a * strength * 0.9);
    } else {
        float a = (1.0 - abs(corner.x)) * (1.0 - smoothstep(0.3, 1.0, abs(corner.y)));
        // Drops show the sky they refract and reflect.
        vec3 light = skyLight(up) * 1.6 + sunRadiance() * 0.05;
        FragColor = vec4(light, a * strength * 0.35);
    }
}
#endif
