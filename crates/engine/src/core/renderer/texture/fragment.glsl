#version 460 core

out vec4 FragColor;

in vec2 texCoord;

uniform sampler2D texture0;

// 0 = colour texture (sample RGB directly)
// 1 = depth texture  (show raw depth as greyscale)
uniform int isDepth;

void main() {
    if (isDepth == 1) {
        float depth = texture(texture0, texCoord).r;
        FragColor = vec4(vec3(depth), 1.0);
    } else {
        FragColor = texture(texture0, texCoord);
    }
}