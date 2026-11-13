in vec3 Normal;
in vec3 WorldPos;
in vec2 TexCoords;

out vec4 FragColor;

void main()
{
    FragColor = shadePbr(WorldPos, Normal, TexCoords);
}
