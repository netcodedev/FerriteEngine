#version 460 core

// The colour (linear: the picture is sRGB) and the model-space normal of a
// scatter model, cut out as `shadePbr` cuts it (see `impostor.rs`).
in vec3 Normal;
in vec2 TexCoords;

uniform sampler2D baseColorMap;
uniform int hasBaseColorMap;

layout (location = 0) out vec4 Albedo;
layout (location = 1) out vec4 NormalOut;

void main() {
    // The leaves' full detail: the picture is far smaller than the leaves'
    // maps, whose smaller levels have averaged the cut-out below the
    // threshold, so the crowns would come out bare. Whatever the picture
    // then shows of each leaf, its own levels average.
    vec4 base = hasBaseColorMap == 1 ? textureLod(baseColorMap, TexCoords, 0.0) : vec4(1.0);
    if (base.a < 0.5) {
        discard;
    }
    vec3 n = normalize(Normal);
    if (!gl_FrontFacing) {
        n = -n;
    }
    Albedo = vec4(base.rgb, 1.0);
    NormalOut = vec4(n * 0.5 + 0.5, 1.0);
}
