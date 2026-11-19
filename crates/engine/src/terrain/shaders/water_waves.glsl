// Waves of the water surface, shared by its vertex and fragment shader.
//
// A sum of Gerstner waves with fixed wavelengths and directions. A body of
// water only carries the waves that fit into it: each wave fades in once
// the water around it (`bodySize`, its typical width) is several of its
// wavelengths across, so ponds stay calm, lakes get a chop and the open sea
// long swells. Because the waves themselves never change, only their
// strength, neighbouring points always agree on the phase and the surface
// does not stretch where the body size changes. Toward the shore (`depth`,
// the water under the rest position) every wave flattens out, so at the
// waterline the surface sits at the water level and meets the ground there.

uniform float uTime;

const int WAVE_COUNT = 6;
// Must match `WAVE_LENGTHS` in `water.rs`.
const float WAVE_LENGTHS[WAVE_COUNT] = float[](58.0, 37.0, 23.0, 14.5, 9.0, 5.8);
// How far each wave has run: its angular frequency times the time, wrapped
// to 0..2π on the CPU (in double precision, so the waves stay smooth however
// long the game runs).
uniform float waveTimePhase[WAVE_COUNT];
// Angle of each wave against the wind, and its phase.
const float WAVE_ANGLES[WAVE_COUNT] = float[](0.0, 0.42, -0.36, 0.85, -0.7, 0.22);
const float WAVE_PHASES[WAVE_COUNT] = float[](0.0, 1.7, 4.1, 2.6, 5.3, 0.9);
const vec2 WIND = vec2(0.8, 0.6);
// Amplitude over wavelength of every wave, and how sharp the crests get
// (1 = the sharpest the sum allows without the surface looping).
const float WAVE_STEEPNESS = 0.012;
const float WAVE_CHOP = 0.8;

float waveHash(vec2 p) {
    p = fract(p * vec2(233.34, 851.73));
    p += dot(p, p + 23.45);
    return fract(p.x * p.y);
}

float waveNoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(waveHash(i), waveHash(i + vec2(1.0, 0.0)), u.x),
               mix(waveHash(i + vec2(0.0, 1.0)), waveHash(i + vec2(1.0, 1.0)), u.x), u.y);
}

// Waves come in groups: each wave's height swells and fades over a few
// wavelengths, so the sum never looks like a regular pattern of stripes.
float waveGroup(int i, vec2 xz) {
    float lambda = WAVE_LENGTHS[i];
    return mix(0.3, 1.3, waveNoise(xz / (2.5 * lambda) + vec2(float(i) * 17.3, float(i) * 5.1)));
}

vec2 waveDirection(int i) {
    float c = cos(WAVE_ANGLES[i]);
    float s = sin(WAVE_ANGLES[i]);
    return vec2(WIND.x * c - WIND.y * s, WIND.x * s + WIND.y * c);
}

// Strength of wave `i` (0..1) in a body of `bodySize` at `depth`.
float waveWeight(int i, float bodySize, float depth) {
    float lambda = WAVE_LENGTHS[i];
    float fits = smoothstep(4.0 * lambda, 12.0 * lambda, bodySize);
    float shore = smoothstep(0.0, min(0.2 * lambda, 5.0), depth);
    return fits * shore;
}

// The tallest the waves at a point get (sum of their amplitudes).
float waveAmplitude(float bodySize, float depth) {
    float amplitude = 0.0;
    for (int i = 0; i < WAVE_COUNT; i++) {
        amplitude += WAVE_STEEPNESS * WAVE_LENGTHS[i] * waveWeight(i, bodySize, depth);
    }
    return amplitude;
}

struct WaveSample {
    vec3 offset;     // displacement of the rest position
    vec3 normal;
    float jacobian;  // < 1 where the surface bunches up at a crest
};

WaveSample sampleWaves(vec2 xz, float bodySize, float depth) {
    vec3 offset = vec3(0.0);
    // Derivatives of the displaced position along x and z.
    vec3 dx = vec3(1.0, 0.0, 0.0);
    vec3 dz = vec3(0.0, 0.0, 1.0);
    for (int i = 0; i < WAVE_COUNT; i++) {
        float weight = waveWeight(i, bodySize, depth);
        if (weight <= 0.0) {
            continue;
        }
        float lambda = WAVE_LENGTHS[i];
        float k = 6.2831853 / lambda;
        vec2 d = waveDirection(i);
        float a = WAVE_STEEPNESS * lambda * weight * waveGroup(i, xz);
        // Every wave gets an equal share of the chop: their crests can all
        // line up without the surface folding over itself.
        float q = WAVE_CHOP / (6.2831853 * WAVE_STEEPNESS * 1.3 * float(WAVE_COUNT));
        float theta = k * dot(d, xz) - waveTimePhase[i] + WAVE_PHASES[i];
        float s = sin(theta);
        float c = cos(theta);
        offset += vec3(q * a * d.x * c, a * s, q * a * d.y * c);
        float wa = k * a;
        dx += vec3(-q * wa * d.x * d.x * s, wa * d.x * c, -q * wa * d.x * d.y * s);
        dz += vec3(-q * wa * d.x * d.y * s, wa * d.y * c, -q * wa * d.y * d.y * s);
    }
    WaveSample result;
    result.offset = offset;
    result.normal = normalize(cross(dz, dx));
    result.jacobian = dx.x * dz.z - dx.z * dz.x;
    return result;
}
