#version 330 core

// Log2 luminance of the scene, drawn into a small target whose mipmaps
// then average it for the eye adaptation.

in vec2 uv;
out float FragColor;

uniform sampler2D hdrColor;

void main() {
    vec3 c = texture(hdrColor, uv).rgb;
    float luminance = dot(c, vec3(0.2126, 0.7152, 0.0722));
    // Not finite, or black: count as very dark rather than poisoning the mean.
    if (isnan(luminance) || isinf(luminance)) {
        luminance = 0.0;
    }
    FragColor = log2(clamp(luminance, 1e-4, 6e4));
}
