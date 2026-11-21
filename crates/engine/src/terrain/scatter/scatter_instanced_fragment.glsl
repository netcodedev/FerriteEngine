// How far the shading normal bends towards the sky (grass), 0..1.
uniform float normalsUp;

out vec4 FragColor;

void main() {
    pbrTint = Tint;
    pbrNormalUp = normalsUp;
    FragColor = shadeMaterial(materialInputs());
}
