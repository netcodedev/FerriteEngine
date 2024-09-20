#version 330 core

in vec4 outColor;
in vec3 Normal;
in vec3 toLightVector;
in vec2 TexCoords;
in float BlockType;

uniform sampler2D texture_diffuse;
uniform sampler2D texture_normals;
uniform sampler2D texture_shininess;
uniform sampler2D texture_specular;

out vec4 FragColor;

void main()
{
    vec3 unitNormal = normalize(Normal * texture(texture_normals, TexCoords).rgb);
    vec3 unitToLightVector = normalize(toLightVector);
    float intensity = dot(unitNormal, unitToLightVector);
    float brightness = max(intensity, 0.5);
    vec3 diffuse = brightness * texture(texture_diffuse, TexCoords).rgb;

    FragColor = vec4(diffuse, 1.0);
}