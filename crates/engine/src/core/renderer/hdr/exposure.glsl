// How much the scene light is scaled by before tone mapping: the manual
// exposure, times the eye adaptation's when it is on. Comes after
// `#version`; see `Hdr::bind_exposure`.

uniform float exposureScale;      // 2^EV
uniform int autoExposure;
uniform sampler2D adaptedLuminance; // 1×1, log2 of the adapted luminance
uniform float exposureKey;        // where the average luminance ends up

float exposure() {
    if (autoExposure == 0) {
        return exposureScale;
    }
    float adapted = exp2(texelFetch(adaptedLuminance, ivec2(0), 0).r);
    return exposureScale * exposureKey / adapted;
}
