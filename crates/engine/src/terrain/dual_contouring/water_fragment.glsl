#version 460 core

in vec3 toLightVector;
in vec2 worldXZ;

out vec4 FragColor;

void main()
{
    vec3 waterColor = vec3(0.07, 0.24, 0.60);

    // Flat water normal always points straight up
    vec3 normal = vec3(0.0, 1.0, 0.0);
    vec3 unitLight = normalize(toLightVector);
    float diffuse = max(dot(normal, unitLight), 0.4);

    FragColor = vec4(waterColor * diffuse, 0.78);
}
