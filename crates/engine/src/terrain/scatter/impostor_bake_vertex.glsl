// A scatter model's tier, pictured for far away (see `impostor.rs`).
layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normal;
layout (location = 2) in vec2 texCoords;
layout (location = 7) in uint vertexColor;

uniform mat4 viewProjection;

void main() {
    gl_Position = viewProjection * vec4(position, 1.0);
    Normal = normal;
    WorldPos = position;
    TexCoords = texCoords;
    VertexColor = unpackUnorm4x8(vertexColor);
    ObjectPos = position;
    PivotPos = vec3(0.0);
    Tint = vec3(1.0);
}
