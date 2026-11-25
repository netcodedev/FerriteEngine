// The first step of the backdrop UI panels blur (`Plane::blur`): the scene
// as it ends up on screen (exposure, tone mapping, sRGB), a quarter of the
// view's size, four bilinear taps for sixteen texels. `tonemap.glsl` and
// `exposure.glsl` come before this.

in vec2 uv;
out vec4 FragColor;

uniform sampler2D source;
uniform vec2 sourceTexel; // 1 / size of `source`
uniform int tonemapper;

vec3 tap(vec2 offset) {
    vec3 c = texture(source, uv + offset * sourceTexel).rgb;
    if (any(isnan(c)) || any(isinf(c))) {
        return vec3(0.0);
    }
    return min(c, vec3(60000.0));
}

void main() {
    vec3 c = (tap(vec2(-1.0, -1.0)) + tap(vec2(1.0, -1.0)) + tap(vec2(-1.0, 1.0)) + tap(vec2(1.0, 1.0))) * 0.25;
    c *= exposure();
    FragColor = vec4(linearToSrgb(tonemap(c, tonemapper)), 1.0);
}
