// A model's material, shaded with `lighting.glsl` (which comes first).
// The material's own code (`evaluateMaterial`, see `material/codegen.rs`)
// is put after this.

// Set by a fragment shader before `shadeMaterial`, for what it shades:
// the albedo multiplied by `pbrTint` (linear), and the shading normal bent
// towards the sky by `pbrNormalUp` (0..1; grass lit like the ground).
vec3 pbrTint = vec3(1.0);
float pbrNormalUp = 0.0;

// Shade one fragment. The material may discard it (a cut-out); in the
// shadow pass only that cut-out matters.
vec4 shadeMaterial(MaterialInputs i) {
    // Before any discard, while every pixel still runs.
    snowGradients(i.worldPos);
    vec3 Ng = normalize(i.normal);
    if (!gl_FrontFacing) {
        Ng = -Ng; // two-sided geometry such as leaves
    }
    if (pbrNormalUp > 0.0) {
        Ng = normalize(mix(Ng, vec3(0.0, 1.0, 0.0), pbrNormalUp));
    }
    i.normal = Ng;
    Surface s = materialDefaults(Ng);
    evaluateMaterial(s, i);
    if (isShadowPass == 1) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    vec3 albedo = s.baseColor * pbrTint;
    vec3 N = s.normal;
    float roughness = s.roughness;
    float metallic = s.metallic;
    applyWeather(i.worldPos, Ng, albedo, N, roughness, metallic);
    vec3 color = shadeSurface(i.worldPos, N, Ng, albedo, roughness, metallic, s.occlusion, 1.0) + s.emissive;
    return vec4(toDisplay(color), 1.0);
}
