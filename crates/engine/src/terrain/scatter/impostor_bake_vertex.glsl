#version 460 core

// A scatter model's tier, pictured for far away (see `impostor.rs`).
layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normal;
layout (location = 2) in vec2 texCoords;

uniform mat4 viewProjection;

out vec3 Normal;
out vec2 TexCoords;

void main() {
    gl_Position = viewProjection * vec4(position, 1.0);
    Normal = normal;
    TexCoords = texCoords;
}
