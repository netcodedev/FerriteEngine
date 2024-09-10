#version 330 core

in vec4 outColor;
in vec3 Normal;
in vec3 toLightVector;
out vec4 FragColor;

void main()
{
    vec3 unitNormal = normalize(Normal);
    vec3 unitToLightVector = normalize(toLightVector);
    float intensity = dot(unitNormal, unitToLightVector);
    float brightness = max(intensity, 0.0);
    vec3 diffuse = brightness * vec3(1.0);
    FragColor = outColor * vec4(diffuse, 1.0);
}