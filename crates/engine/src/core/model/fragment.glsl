in vec3 Normal;
in vec3 WorldPos;
in vec2 TexCoords;

out vec4 FragColor;

// A see-through silhouette in this colour instead of the material (a
// preview), lit just enough to show its shape.
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
}
