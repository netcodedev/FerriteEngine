// The HDR scene onto the screen: exposure, bloom, tone mapping and the
// sRGB transfer, with a little dither against banding. Also writes the
// scene's depth, so what is drawn over the scene afterwards (gizmos,
// collider lines) is still hidden behind it. `tonemap.glsl` and
// `exposure.glsl` come before this.

in vec2 uv;
out vec4 FragColor;

uniform sampler2D hdrColor;
uniform sampler2D sceneDepth;
uniform sampler2D bloom;
uniform int useBloom;
uniform float bloomIntensity;
uniform int tonemapper;
uniform ivec2 origin; // window position of the view

// Interleaved gradient noise (Jimenez), -0.5..0.5.
float dither(vec2 p) {
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715)))) - 0.5;
}

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy) - origin;
    vec3 color = texelFetch(hdrColor, texel, 0).rgb;
    if (any(isnan(color))) {
        color = vec3(0.0);
    }
    color *= exposure();
    if (useBloom == 1) {
        color += texture(bloom, uv).rgb * bloomIntensity;
    }
    color = linearToSrgb(tonemap(color, tonemapper));
    color += dither(gl_FragCoord.xy) / 255.0;
    FragColor = vec4(color, 1.0);
    gl_FragDepth = texelFetch(sceneDepth, texel, 0).r;
}
