#version 460 core

layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normals;
layout (location = 2) in vec2 texCoords;
layout (location = 3) in uint block_type;

out vec4 outColor;
out vec3 Normal;
out vec3 toLightVector;
out vec2 TexCoords;
out uint BlockType;

uniform vec3 lightPosition;
uniform mat4 model;
uniform mat4 viewProjection;

void main()
{
    vec4 worldPosition = model * vec4(position, 1.0);
    gl_Position = viewProjection * worldPosition;
    if (block_type == 1.0)
        outColor = vec4(0.3, 0.6, 0.4, 1.0);
    else if (block_type == 2.0)
        outColor = vec4(0.5, 0.5, 0.5, 1.0);
    else
        outColor = vec4(0.0, 0.0, 0.0, 1.0);
    Normal = normals;
    TexCoords = texCoords;
    BlockType = block_type;
    toLightVector = lightPosition - worldPosition.xyz;
}