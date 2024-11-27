#version 460 core

in vec3 Color;
in vec3 Normal;
in vec3 toLightVector;
in vec4 fragPosLightSpace;

out vec4 FragColor;

uniform sampler2D shadowMap;

float ShadowCalculation(vec4 fragPosLightSpace, vec3 toLightVector, vec3 normal) {
    vec3 projCoords = fragPosLightSpace.xyz / fragPosLightSpace.w;
    projCoords = projCoords * 0.5 + 0.5;
    if (projCoords.z > 1.0) {
        return 0.0;
    }
    float closestDepth = texture(shadowMap, projCoords.xy).r;
    float currentDepth = projCoords.z;
    float bias = max(0.01 * (1.0 - dot(normal, toLightVector)), 0.005);
    float shadow = 0.0;
    vec2 texelSize = 1.0 / textureSize(shadowMap, 0);
    for(int x = -2; x <= 2; ++x) {
        for(int y = -2; y <= 2; ++y) {
            float pcfDepth = texture(shadowMap, projCoords.xy + vec2(x, y) * texelSize).r; 
            shadow += currentDepth - bias > pcfDepth ? 1.0 : 0.0;        
        }    
    }
    shadow /= 25.0;
    return shadow;
}

void main() {
    vec3 unitNormal = normalize(Normal);
    vec3 normal = unitNormal;

    normal = normalize(normal);

    vec3 unitToLightVector = normalize(toLightVector);
    float intensity = dot(normal, unitToLightVector);
    float brightness = max(intensity, 0.5);
    vec3 diffuse = brightness * vec3(1.0);
    float shadow = ShadowCalculation(fragPosLightSpace, unitToLightVector, normal);
    FragColor = vec4((0.5 + (1.0 - shadow) * diffuse) * Color, 1.0);
}