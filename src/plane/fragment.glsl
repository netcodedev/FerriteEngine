#version 330 core

in vec4 vertex_color;
in vec2 vertex_position;
in vec2 rect_size;

out vec4 FragColor;

uniform float borderThickness = 0.0;
uniform float borderRadius = 0.0;
uniform vec4 borderColor = vec4(0.0, 0.0, 0.0, 0.0);

float RectSDF(vec2 position, vec2 halfSize, float radius)
{
    vec2 d = abs(position) - halfSize + vec2(radius);
    return min(max(d.x, d.y), 0.0) + length(max(d, 0.0)) - radius;   
}

void main()
{
    vec2 pos = rect_size * vertex_position;

    float dist = RectSDF(pos - (rect_size / 2.0), rect_size / 2.0, borderRadius);
    float blend = smoothstep(-1.0, 1.0, abs(dist) - borderThickness);
    if(dist > 0.0) {
        discard;
    }

    FragColor = mix(borderColor, vertex_color, blend);
}