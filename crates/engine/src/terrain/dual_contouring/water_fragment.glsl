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
    float sunAlt = unitLight.y;
    float dayFactor = smoothstep(-0.10, 0.20, sunAlt);
    float ambient = mix(0.08, 0.4, dayFactor);
    float diffuse = max(dot(normal, unitLight), 0.0) * dayFactor;
    float lit = ambient + diffuse * 0.6;

    FragColor = vec4(waterColor * lit, 0.78);
}
