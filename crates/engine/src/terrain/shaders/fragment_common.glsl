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
// where the chunks are loaded (`columnLoaded`, see `columns.glsl`), and
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
    return isHorizon == 1 && (columnLoaded(WorldPos.xz) || length(WorldPos.xz - horizonSink.xy) < horizonHole);
}
