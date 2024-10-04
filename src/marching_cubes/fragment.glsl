#version 460 core

in vec3 Color;
in vec3 Normal;
in vec3 toLightVector;

out vec4 FragColor;

void main() {
    vec3 unitNormal = normalize(Normal);
    vec3 normal = unitNormal;

    normal = normalize(normal);

    vec3 unitToLightVector = normalize(toLightVector);
    float intensity = dot(normal, unitToLightVector);
    float brightness = max(intensity, 0.5);
    vec3 diffuse = brightness * vec3(1.0);
    FragColor = vec4(Color * diffuse, 1.0);
}