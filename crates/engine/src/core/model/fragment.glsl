in vec3 Normal;
in vec3 WorldPos;
in vec2 TexCoords;

out vec4 FragColor;

// 1: a see-through silhouette in this colour instead of the material (a
// preview), lit just enough to show its shape. 2: the material as it is,
// tint.a opaque (a faded model in an editor preview).
uniform int useTint;
uniform vec4 tint;

void main()
{
    if (useTint == 1) {
        float light = 0.55 + 0.45 * abs(dot(normalize(Normal), normalize(vec3(0.4, 1.0, 0.25))));
        FragColor = vec4(tint.rgb * light, tint.a);
        return;
    }
    FragColor = shadePbr(WorldPos, Normal, TexCoords);
    if (useTint == 2) {
        FragColor.a *= tint.a;
    }
}
