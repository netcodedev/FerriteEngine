// What a material graph reads about the point it shades or moves (see
// `material/codegen.rs`), and helpers its nodes call. Both stages.

struct MaterialInputs {
    vec3 worldPos;
    // World space; per fragment the geometric normal turned towards the
    // eye (and bent up for grass), per vertex the mesh's.
    vec3 normal;
    vec3 objectNormal;
    vec2 uv;
    vec4 color;
    vec3 objectPos;
    // The model's origin in the world.
    vec3 pivot;
    vec3 tint;
};

// The world's wind: direction (x, z), strength, and its clock (seconds).
uniform vec4 wind;

float materialHash(vec3 p) {
    p = fract(p * 0.3183099 + vec3(0.1, 0.7, 0.3));
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}

// Smooth value noise, 0..1.
float materialNoise(vec3 x) {
    vec3 i = floor(x);
    vec3 f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(materialHash(i + vec3(0, 0, 0)), materialHash(i + vec3(1, 0, 0)), f.x),
                   mix(materialHash(i + vec3(0, 1, 0)), materialHash(i + vec3(1, 1, 0)), f.x), f.y),
               mix(mix(materialHash(i + vec3(0, 0, 1)), materialHash(i + vec3(1, 0, 1)), f.x),
                   mix(materialHash(i + vec3(0, 1, 1)), materialHash(i + vec3(1, 1, 1)), f.x), f.y), f.z);
}
