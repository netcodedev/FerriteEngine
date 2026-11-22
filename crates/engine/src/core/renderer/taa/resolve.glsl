#version 330 core

// Temporal anti-aliasing: blends this frame (drawn with a sub-pixel
// jitter) into the history of the frames before, found through the motion
// vectors. The history is clipped to the colours around the texel in this
// frame, so what was revealed or moved does not leave trails, and dropped
// where it shows a surface at another distance than this one had then
// (the history's alpha keeps each texel's distance, clip w). Works on
// linear HDR light, before bloom and exposure.

in vec2 uv;
out vec4 FragColor;

uniform sampler2D current;
uniform sampler2D history;
uniform sampler2D velocity;
uniform sampler2D sceneDepth;
uniform vec2 texelSize;
uniform int historyValid;
// Share of this frame in the result where the history is fine.
uniform float currentWeight;

// Bright samples weigh less (Karis), so a single bright pixel does not
// flicker through the history.
vec3 compress(vec3 c) {
    return c / (1.0 + max(c.r, max(c.g, c.b)));
}

vec3 uncompress(vec3 c) {
    return c / max(1.0 - max(c.r, max(c.g, c.b)), 1e-4);
}

vec3 toYCoCg(vec3 c) {
    return vec3(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b
    );
}

vec3 fromYCoCg(vec3 c) {
    return vec3(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

vec3 load(ivec2 texel) {
    ivec2 size = textureSize(current, 0);
    vec3 c = texelFetch(current, clamp(texel, ivec2(0), size - 1), 0).rgb;
    if (any(isnan(c)) || any(isinf(c))) {
        c = vec3(0.0);
    }
    return toYCoCg(compress(max(c, vec3(0.0))));
}

// The history, sharper than bilinear: Catmull-Rom from five bilinear taps
// (Jimenez).
vec3 sampleHistory(vec2 at) {
    vec2 size = 1.0 / texelSize;
    vec2 position = at * size;
    vec2 center = floor(position - 0.5) + 0.5;
    vec2 f = position - center;
    vec2 w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    vec2 w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    vec2 w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    vec2 w3 = f * f * (-0.5 + 0.5 * f);
    vec2 w12 = w1 + w2;
    vec2 tc0 = (center - 1.0) * texelSize;
    vec2 tc3 = (center + 2.0) * texelSize;
    vec2 tc12 = (center + w2 / w12) * texelSize;
    vec3 result =
        texture(history, vec2(tc12.x, tc0.y)).rgb * (w12.x * w0.y) +
        texture(history, vec2(tc0.x, tc12.y)).rgb * (w0.x * w12.y) +
        texture(history, vec2(tc12.x, tc12.y)).rgb * (w12.x * w12.y) +
        texture(history, vec2(tc3.x, tc12.y)).rgb * (w3.x * w12.y) +
        texture(history, vec2(tc12.x, tc3.y)).rgb * (w12.x * w3.y);
    float weight = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(result / weight, vec3(0.0));
}

// Move `history` toward the middle of the box until it lies inside
// (clipping keeps its hue better than clamping each channel).
vec3 clipToBox(vec3 history, vec3 low, vec3 high) {
    vec3 middle = 0.5 * (high + low);
    vec3 extent = 0.5 * (high - low) + 1e-5;
    vec3 offset = history - middle;
    vec3 units = abs(offset / extent);
    float furthest = max(units.x, max(units.y, units.z));
    return furthest > 1.0 ? middle + offset / furthest : history;
}

// Whether the history around `at` shows no surface at `expected` (the
// distance this texel's surface had a frame before): it was hidden then.
// Of the four texels around `at` the closest in distance counts, so the
// edges of things keep their history.
bool revealed(vec2 at, float expected) {
    ivec2 size = textureSize(history, 0);
    ivec2 base = ivec2(floor(at / texelSize - 0.5));
    float best = 1e20;
    for (int i = 0; i < 4; i++) {
        ivec2 t = clamp(base + ivec2(i & 1, i >> 1), ivec2(0), size - 1);
        best = min(best, abs(texelFetch(history, t, 0).a - expected));
    }
    return best > 0.05 * expected + 0.05;
}

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    vec3 center = load(texel);

    // The colours around the texel: their mean and spread make the box
    // the history has to fit in. The nearest depth among them picks the
    // motion, so edges of things in front move with them.
    vec3 sum = center;
    vec3 squares = center * center;
    float nearest = texelFetch(sceneDepth, texel, 0).r;
    ivec2 nearestTexel = texel;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            if (x == 0 && y == 0) {
                continue;
            }
            ivec2 at = texel + ivec2(x, y);
            vec3 c = load(at);
            sum += c;
            squares += c * c;
            float depth = texelFetch(sceneDepth, clamp(at, ivec2(0), textureSize(sceneDepth, 0) - 1), 0).r;
            if (depth < nearest) {
                nearest = depth;
                nearestTexel = at;
            }
        }
    }
    vec3 mean = sum / 9.0;
    vec3 deviation = sqrt(max(squares / 9.0 - mean * mean, vec3(0.0)));
    vec3 low = mean - deviation * 1.25;
    vec3 high = mean + deviation * 1.25;

    vec4 moved = texelFetch(velocity, clamp(nearestTexel, ivec2(0), textureSize(velocity, 0) - 1), 0);
    vec2 motion = moved.rg;
    vec2 previousUv = uv - motion;
    bool outside = any(lessThan(previousUv, vec2(0.0))) || any(greaterThan(previousUv, vec2(1.0)));
    // This texel's own distance, kept in the history for the next frame.
    float distanceNow = texelFetch(velocity, texel, 0).a;

    vec3 result = center;
    if (historyValid == 1 && !outside && !revealed(previousUv, moved.b)) {
        vec3 previous = toYCoCg(compress(sampleHistory(previousUv)));
        previous = clipToBox(previous, low, high);
        // Fast motion: lean on this frame a little more, the history is
        // resampled every frame and would blur.
        float pixels = length(motion / texelSize);
        float weight = clamp(currentWeight + pixels * 0.01, currentWeight, 0.5);
        result = mix(previous, center, weight);
    }
    FragColor = vec4(uncompress(fromYCoCg(result)), distanceNow);
}
