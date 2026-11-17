// The water surface: the grid at the water level, moved by the waves.
// `water_waves.glsl` comes before this.

layout (location = 0) in vec3 position;
layout (location = 1) in float depth;
layout (location = 2) in float bodySize;
// Metres per second the water flows (x, z): 0 but in rivers.
layout (location = 3) in vec2 flow;

out vec3 WorldPos;
out vec2 RestXZ;       // where the point sits without waves (for normals)
out float Depth;       // water under the rest position, < 0 under land
out float BodySize;
out float Crest;       // 0..1, how much the waves bunch up here
out float WaveHeight;  // -1..1, height relative to the local amplitude
out vec2 Flow;

uniform mat4 model;
uniform mat4 viewProjection;
// The far water (see `terrain::horizon`): flat (its grid is far too coarse
// for the waves to move it), a little under its level, and left out where
// the chunks are loaded (see the fragment program). Its rivers have a
// negative `bodySize`.
uniform int isFarWater;

void main()
{
    vec3 rest = (model * vec4(position, 1.0)).xyz;
    float size = abs(bodySize);
    WaveSample waves = sampleWaves(rest.xz, size, depth);
    vec3 world = rest + waves.offset;
    if (isFarWater == 1) {
        world = rest - vec3(0.0, 0.15, 0.0);
        waves.offset = vec3(0.0);
    }
    gl_Position = viewProjection * vec4(world, 1.0);
    WorldPos = world;
    RestXZ = rest.xz;
    Depth = depth;
    BodySize = size;
    Flow = flow;
    Crest = smoothstep(0.62, 0.35, waves.jacobian);
    float amplitude = waveAmplitude(size, depth);
    WaveHeight = amplitude > 1e-3 ? clamp(waves.offset.y / amplitude, -1.0, 1.0) : 0.0;
}
