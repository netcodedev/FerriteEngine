#version 330 core

// One step up the bloom chain: a 3×3 tent filter over the smaller level,
// added onto the next larger one.

in vec2 uv;
out vec3 FragColor;

uniform sampler2D source;
uniform vec2 radius; // in UV units of the target

void main() {
    float x = radius.x, y = radius.y;
    vec3 a = texture(source, uv + vec2(-x, y)).rgb;
    vec3 b = texture(source, uv + vec2(0.0, y)).rgb;
    vec3 c = texture(source, uv + vec2(x, y)).rgb;
    vec3 d = texture(source, uv + vec2(-x, 0.0)).rgb;
    vec3 e = texture(source, uv).rgb;
    vec3 f = texture(source, uv + vec2(x, 0.0)).rgb;
    vec3 g = texture(source, uv + vec2(-x, -y)).rgb;
    vec3 h = texture(source, uv + vec2(0.0, -y)).rgb;
    vec3 i = texture(source, uv + vec2(x, -y)).rgb;
    FragColor = (e * 4.0 + (b + d + f + h) * 2.0 + (a + c + g + i)) / 16.0;
}
