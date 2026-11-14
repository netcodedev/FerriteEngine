// One step of the snow trail map (`weather/snow.rs`): what was there,
// moved along with the camera and filled in a little by new snow, with the
// footprints of this frame pressed in. r: pressed down 0..1, g: snow
// pushed up around a print 0..1.

uniform sampler2D previous;
// Texels the map moved since the last step.
uniform ivec2 shift;
// How much new snow fills the trails this step.
uniform float refill;
// World x, z of the map's corner, and metres per texel.
uniform vec2 origin;
uniform float texel;

// Must match `MAX_FOOTPRINTS` in `weather/mod.rs`.
const int MAX_STAMPS = 32;
uniform int stampCount;
// x, z of a print's centre and its heel-to-toe direction.
uniform vec4 stamps[MAX_STAMPS];
uniform float stampLength[MAX_STAMPS];

out vec2 result;

void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    ivec2 from = at + shift;
    ivec2 size = textureSize(previous, 0);
    vec2 trail = vec2(0.0);
    if (all(greaterThanEqual(from, ivec2(0))) && all(lessThan(from, size))) {
        trail = texelFetch(previous, from, 0).rg;
    }
    trail = max(trail - refill, 0.0);

    vec2 p = origin + (vec2(at) + 0.5) * texel;
    for (int i = 0; i < stampCount; ++i) {
        vec2 d = p - stamps[i].xy;
        vec2 forward = stamps[i].zw;
        vec2 side = vec2(-forward.y, forward.x);
        float along = dot(d, forward) / (stampLength[i] * 0.5);
        // A sole: narrow at the heel, wider at the ball of the foot.
        float width = stampLength[i] * 0.19 * mix(0.8, 1.1, clamp(along * 0.5 + 0.5, 0.0, 1.0));
        float e = length(vec2(along, dot(d, side) / width));
        float press = 1.0 - smoothstep(0.8, 1.0, e);
        float rim = smoothstep(0.85, 1.05, e) * (1.0 - smoothstep(1.05, 1.7, e));
        trail.r = max(trail.r, press);
        trail.g = max(trail.g, rim * 0.8 * (1.0 - trail.r));
    }
    result = trail;
}
