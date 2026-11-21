in vec3 Normal;
in vec3 WorldPos;
in vec2 TexCoords;
flat in vec3 Tint;

// How far the shading normal bends towards the sky (grass), 0..1.
uniform float normalsUp;

out vec4 FragColor;

void main() {
    pbrTint = Tint;
    pbrNormalUp = normalsUp;
    FragColor = shadePbr(WorldPos, Normal, TexCoords);
}
