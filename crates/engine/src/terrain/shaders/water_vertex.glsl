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
// for the waves to move it), and sunk under the loaded chunks, within
// `farSink.z` metres of the camera at `farSink.xy` (x, z), so the chunks'
// water lies over it.
uniform int isFarWater;
uniform vec3 farSink;

void main()
{
    vec3 rest = (model * vec4(position, 1.0)).xyz;
    WaveSample waves = sampleWaves(rest.xz, bodySize, depth);
    vec3 world = rest + waves.offset;
    if (isFarWater == 1) {
        float d = length(rest.xz - farSink.xy);
        world = rest - vec3(0.0, 0.15 + 4.0 * (1.0 - smoothstep(farSink.z - 160.0, farSink.z - 20.0, d)), 0.0);
        waves.offset = vec3(0.0);
    }
    gl_Position = viewProjection * vec4(world, 1.0);
    WorldPos = world;
    RestXZ = rest.xz;
    Depth = depth;
    BodySize = bodySize;
    Flow = flow;
    Crest = smoothstep(0.62, 0.35, waves.jacobian);
    float amplitude = waveAmplitude(bodySize, depth);
    WaveHeight = amplitude > 1e-3 ? clamp(waves.offset.y / amplitude, -1.0, 1.0) : 0.0;
}
