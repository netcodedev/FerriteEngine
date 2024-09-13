#version 330 core

in vec4 outColor;
in vec3 Normal;
in vec3 toLightVector;
out vec4 FragColor;

void main()
{
    vec3 unitNormal = normalize(Normal);
    vec3 normal = vec3(0.0, 0.0, 0.0);

    if (abs(unitNormal.y) > abs(unitNormal.x) && abs(unitNormal.y) > abs(unitNormal.z)) {
        if (unitNormal.y < 0.0) {
            normal = vec3(0.0, -1.0, 0.0);
        } else {
            normal = vec3(0.0, 1.0, 0.0);
        }
    } else if (abs(unitNormal.x) > abs(unitNormal.y) && abs(unitNormal.x) > abs(unitNormal.z)) {
        if (unitNormal.x < 0.0) {
            normal = vec3(-1.0, 0.0, 0.0);
        } else {
            normal = vec3(1.0, 0.0, 0.0);
        }
    } else {
        if (unitNormal.z < 0.0) {
            normal = vec3(0.0, 0.0, -1.0);
        } else {
            normal = vec3(0.0, 0.0, 1.0);
        }
    }
    normal = normalize(normal);

    vec3 unitToLightVector = normalize(toLightVector);
    float intensity = dot(normal, unitToLightVector);
    float brightness = max(intensity, 0.5);
    vec3 diffuse = brightness * vec3(1.0);
    FragColor = outColor * vec4(diffuse, 1.0);
}