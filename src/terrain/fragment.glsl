#version 460 core

in vec4 outColor;
in vec3 Normal;
in vec3 toLightVector;
in vec2 TexCoords;
flat in uint BlockType;

uniform sampler2D texture0;
uniform sampler2D texture1;

out vec4 FragColor;

void main()
{
    vec3 unitNormal = normalize(Normal);
    vec3 normal = vec3(0.0, 0.0, 0.0);

    if (abs(unitNormal.r) > abs(unitNormal.g) && abs(unitNormal.r) > abs(unitNormal.g)) {
        if (unitNormal.r < 0.0) {
            normal = vec3(0.0, -1.0, 0.0);
        } else {
            normal = vec3(0.0, 1.0, 0.0);
        }
    } else if (abs(unitNormal.g) > abs(unitNormal.r) && abs(unitNormal.g) > abs(unitNormal.b)) {
        if (unitNormal.g < 0.0) {
            normal = vec3(-1.0, 0.0, 0.0);
        } else {
            normal = vec3(1.0, 0.0, 0.0);
        }
    } else {
        if (unitNormal.b < 0.0) {
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
    vec4 texColor = vec4(0.0);
    if(BlockType == 1)
        texColor = texture(texture0, TexCoords);
    else if(BlockType == 2)
        texColor = texture(texture1, TexCoords);
    FragColor = texColor * vec4(diffuse, 1.0);
}