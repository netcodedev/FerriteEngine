// One step down the bloom chain: a 13 tap filter (Jimenez, "Next
// generation post processing in Call of Duty: Advanced Warfare") from
// `source` into a target half its size. The first step also exposes the
// scene, keeps only what is brighter than the threshold and averages with
// Karis' weights, so single very bright pixels do not flicker.

in vec2 uv;
out vec3 FragColor;

uniform sampler2D source;
uniform vec2 sourceTexel;     // 1 / size of `source`
uniform int firstStep;
// x: threshold, y: threshold - knee, z: 2 knee, w: 0.25 / knee
uniform vec4 threshold;

vec3 tap(float x, float y) {
    vec3 c = texture(source, uv + vec2(x, y) * sourceTexel).rgb;
    // Half floats overflow to inf, and a NaN would spread across the bloom.
    if (any(isnan(c)) || any(isinf(c))) {
        return vec3(0.0);
    }
    return min(c, vec3(60000.0));
}

float luma(vec3 c) {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

// Soft threshold: a quadratic knee instead of a hard cut.
vec3 prefilter(vec3 c) {
    c *= exposure();
    float brightness = max(c.r, max(c.g, c.b));
    float soft = clamp(brightness - threshold.y, 0.0, threshold.z);
    soft = soft * soft * threshold.w;
    float contribution = max(soft, brightness - threshold.x) / max(brightness, 1e-5);
    return c * contribution;
}

// Karis' weight of a box: its share shrinks the brighter it is, so a lone
// bright pixel does not outweigh its neighbours. Evenly bright areas keep
// their brightness, as the weights are normalised.
float karisWeight(vec3 box) {
    return 1.0 / (1.0 + luma(box * exposure()));
}

void main() {
    vec3 a = tap(-2.0, 2.0), b = tap(0.0, 2.0), c = tap(2.0, 2.0);
    vec3 d = tap(-2.0, 0.0), e = tap(0.0, 0.0), f = tap(2.0, 0.0);
    vec3 g = tap(-2.0, -2.0), h = tap(0.0, -2.0), i = tap(2.0, -2.0);
    vec3 j = tap(-1.0, 1.0), k = tap(1.0, 1.0);
    vec3 l = tap(-1.0, -1.0), m = tap(1.0, -1.0);

    vec3 color;
    if (firstStep == 1) {
        // Five overlapping boxes, each weighed down by its brightness.
        vec3 boxes[5] = vec3[5](
            (j + k + l + m) * 0.25,
            (a + b + d + e) * 0.25, (b + c + e + f) * 0.25,
            (d + e + g + h) * 0.25, (e + f + h + i) * 0.25);
        vec3 sum = vec3(0.0);
        float total = 0.0;
        for (int n = 0; n < 5; n++) {
            float w = (n == 0 ? 0.5 : 0.125) * karisWeight(boxes[n]);
            sum += boxes[n] * w;
            total += w;
        }
        color = prefilter(sum / total);
    } else {
        color = e * 0.125
            + (a + c + g + i) * 0.03125
            + (b + d + f + h) * 0.0625
            + (j + k + l + m) * 0.125;
    }
    FragColor = max(color, vec3(0.0));
}
