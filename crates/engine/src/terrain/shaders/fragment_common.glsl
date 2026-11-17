// What every terrain fragment program gets (see
// `terrain::shaders::fragment_prelude`): the standard vertex program's
// outputs, the horizon's uniforms and the sea's level.

in vec3 Normal;
in vec3 WorldPos;
// Display space.
in vec3 GroundColor;
// The source's own numbers (see `terrain::source::SurfaceSample`).
in vec4 Data0;
in vec4 Data1;
in vec4 Data2;

out vec4 FragColor;

// The horizon (see `terrain::horizon` and the vertex program) is left out
// within `horizonHole` metres of the camera at `horizonSink.xy` (x, z).
uniform int isHorizon;
uniform vec3 horizonSink;
uniform float horizonHole;

// The sea's level (see `TerrainSource::sea_level`); far below the world
// without a sea.
uniform float waterLine;

// Whether this fragment of the horizon lies where chunks are drawn
// instead: leave it out (`discard`).
bool hiddenHorizon() {
    return isHorizon == 1 && length(WorldPos.xz - horizonSink.xy) < horizonHole;
}

// Haze: none up to `HAZE_START` metres, then thickening over
// `HAZE_DISTANCE`, toward the sky's light.
const float TERRAIN_HAZE_START = 600.0;
const float TERRAIN_HAZE_DISTANCE = 9000.0;

// `color` (linear) as the air between the camera and `p` hazes it.
vec3 terrainHaze(vec3 color, vec3 p) {
    vec3 toPoint = p - cameraPosition;
    float far = length(toPoint);
    vec3 along = normalize(vec3(toPoint.x, max(toPoint.y, 0.0) * 0.3, toPoint.z));
    float haze = 1.0 - exp(-max(far - TERRAIN_HAZE_START, 0.0) / TERRAIN_HAZE_DISTANCE);
    return mix(color, skyLight(along), haze);
}
