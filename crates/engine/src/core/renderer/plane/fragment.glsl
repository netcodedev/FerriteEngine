#version 330 core

in vec4 vertex_color;
in vec2 vertex_position;
in vec4 rect_size;

out vec4 FragColor;

flat in vec4 borderColor;
flat in vec4 borderRadius;
flat in float borderThickness;

// Blurred planes (`Plane::blur`): the view behind, blurred, tinted by the
// plane's colour by its alpha. `backdropRect`: the view's window origin
// and size.
uniform int useBackdrop;
uniform sampler2D backdrop;
uniform vec4 backdropRect;

float RectSDF(vec2 position, vec2 halfSize, vec4 radius)
{
    radius.xy = (position.x > 0.0) ? radius.xy : radius.zw;
    radius.x = (position.y > 0.0) ? radius.x : radius.y;
    vec2 d = abs(position) - halfSize + radius.x;
    return min(max(d.x, d.y), 0.0) + length(max(d, 0.0)) - radius.x;
}

void main()
{
    vec2 pos = rect_size.xy * vertex_position - rect_size.zw;

    float dist = RectSDF(pos - (rect_size.xy / 2.0), rect_size.xy / 2.0, borderRadius);
    float blend = smoothstep(-1.0, 1.0, abs(dist) - borderThickness);
    if(dist > 0.0) {
        discard;
    }

    vec4 fill = vertex_color;
    if (useBackdrop == 1) {
        vec2 at = (gl_FragCoord.xy - backdropRect.xy) / backdropRect.zw;
        vec3 behind = texture(backdrop, at).rgb;
        fill = vec4(mix(behind, vertex_color.rgb, vertex_color.a), 1.0);
    }
    FragColor = mix(borderColor, fill, blend);
}