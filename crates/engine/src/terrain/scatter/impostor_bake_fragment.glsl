// The colour (linear: the picture is sRGB) and the model-space normal of a
// scatter model, cut out as its material cuts it (see `impostor.rs`).
// Textures are read at full detail (the material's code is made so): the
// picture is far smaller than the leaves' maps, whose smaller levels have
// averaged the cut-out below the threshold, so the crowns would come out
// bare.

layout (location = 0) out vec4 Albedo;
layout (location = 1) out vec4 NormalOut;

void main() {
    MaterialInputs i = materialInputs();
    vec3 n = normalize(Normal);
    if (!gl_FrontFacing) {
        n = -n;
    }
    i.normal = n;
    Surface s = materialDefaults(n);
    evaluateMaterial(s, i);
    Albedo = vec4(s.baseColor, 1.0);
    NormalOut = vec4(n * 0.5 + 0.5, 1.0);
}
