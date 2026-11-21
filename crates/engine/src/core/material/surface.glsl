// What a material puts out per fragment (`evaluateMaterial`).
struct Surface {
    vec3 baseColor;
    float opacity;
    vec3 normal;
    float roughness;
    float metallic;
    float occlusion;
    vec3 emissive;
};

// Defined by the material's generated code.
void evaluateMaterial(inout Surface s, MaterialInputs i);

const float DEFAULT_ROUGHNESS = 0.8;

Surface materialDefaults(vec3 N) {
    Surface s;
    s.baseColor = vec3(1.0);
    s.opacity = 1.0;
    s.normal = N;
    s.roughness = DEFAULT_ROUGHNESS;
    s.metallic = 0.0;
    s.occlusion = 1.0;
    s.emissive = vec3(0.0);
    return s;
}
