// The horizon's depth (see `terrain::shaders::horizon_depth_fragment`):
// none where the chunks are loaded, nor within `horizonHole` metres of the
// camera at `horizonSink.xy` (under the ground).

in vec3 WorldPos;

uniform int isHorizon;
uniform vec3 horizonSink;
uniform float horizonHole;

void main() {
    if (isHorizon == 1 && (columnLoaded(WorldPos.xz) || length(WorldPos.xz - horizonSink.xy) < horizonHole)) {
        discard;
    }
}
