#version 460 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
layout (location = 2) in vec3 color;

out vec3 Normal;
out vec3 Color;
out vec3 toLightVector;
out vec4 fragPosLightSpace;

uniform vec3 lightPosition;
uniform mat4 model;
uniform mat4 viewProjection;
uniform mat4 lightProjection;

void main()
{
    vec4 worldPosition = model * vec4(position, 1.0);
    gl_Position = viewProjection * worldPosition;
    Normal = normalize(normals);
    if(position.y < 50.0) {
        Color = vec3(0.1, 0.2, 0.8);
    } else if(position.y < 51.0) {
        Color = vec3(0.76078431, 0.69803921, 0.50196078);
    } else if(position.y > 90.0) {
        Color = vec3(0.95, 0.95, 0.95);
    } else if(position.y > 80.0) {
        Color = vec3(0.5, 0.5, 0.5);
    } else {
        Color = color;
    }
    fragPosLightSpace = lightProjection * worldPosition;
    toLightVector = lightPosition - worldPosition.xyz;
}