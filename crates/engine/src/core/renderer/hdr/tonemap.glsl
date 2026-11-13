// Tone mapping: scene light (linear, unbounded) to display colour (sRGB,
// 0..1). Shared by the HDR composite (`hdr/composite.glsl`) and by lit
// shaders that draw straight to the screen (`toDisplay` in
// `pbr/lighting.glsl`). The numbers match `Tonemapper` in `hdr/mod.rs`.

const int TONEMAP_ACES = 0;
const int TONEMAP_FILMIC = 1;
const int TONEMAP_REINHARD = 2;
const int TONEMAP_NONE = 3;

// ACES filmic curve, Stephen Hill's fit of the reference rendering and
// output transforms (sRGB primaries in and out). The curve expects about
// 1.6 times brighter input than a plain clamp: scaled so mid grey (0.18)
// stays where it was without tone mapping.
vec3 tonemapAces(vec3 color) {
    color *= 1.6;
    const mat3 inputMatrix = mat3(
        0.59719, 0.07600, 0.02840,
        0.35458, 0.90834, 0.13383,
        0.04823, 0.01566, 0.83777);
    const mat3 outputMatrix = mat3(
        1.60475, -0.10208, -0.00327,
        -0.53108, 1.10813, -0.07276,
        -0.07367, -0.00605, 1.07602);
    vec3 v = inputMatrix * color;
    vec3 a = v * (v + 0.0245786) - 0.000090537;
    vec3 b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return clamp(outputMatrix * (a / b), 0.0, 1.0);
}

// John Hable's filmic curve (Uncharted 2), white point 11.2. Scaled, like
// ACES, to keep mid grey in place.
vec3 hableCurve(vec3 x) {
    const float A = 0.15, B = 0.50, C = 0.10, D = 0.20, E = 0.02, F = 0.30;
    return (x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F) - E / F;
}

vec3 tonemapFilmic(vec3 color) {
    const float WHITE = 11.2;
    return clamp(hableCurve(color * 3.2) / hableCurve(vec3(WHITE)), 0.0, 1.0);
}

vec3 tonemap(vec3 color, int tonemapper) {
    color = max(color, 0.0);
    if (tonemapper == TONEMAP_ACES) {
        return tonemapAces(color);
    } else if (tonemapper == TONEMAP_FILMIC) {
        return tonemapFilmic(color);
    } else if (tonemapper == TONEMAP_REINHARD) {
        return color / (1.0 + color);
    }
    return clamp(color, 0.0, 1.0);
}

// The sRGB transfer function (linear 0..1 to what the display expects).
vec3 linearToSrgb(vec3 color) {
    color = clamp(color, 0.0, 1.0);
    vec3 low = color * 12.92;
    vec3 high = 1.055 * pow(color, vec3(1.0 / 2.4)) - 0.055;
    return mix(high, low, vec3(lessThanEqual(color, vec3(0.0031308))));
}
