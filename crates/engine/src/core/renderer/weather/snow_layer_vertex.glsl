// The snow layer (`weather/snow_layer.rs`): one ring of the grid around
// the camera, raised off the ground (the ground map) by the snow's depth,
// less what feet pressed down. `weather_maps.glsl` comes before this.

layout(location = 0) in vec2 aGrid; // cell corner, 0..cells on each side

uniform mat4 viewProjection;
// World x, z of the ring's centre, metres between vertices, cells a side.
uniform vec2 levelCenter;
uniform float spacing;
uniform float cells;
// 1 for the outermost ring, which sinks into the ground at its edge.
uniform int outermost;
// No snow at or below the water's surface.
uniform float waterLine;
// Snow kept under a footprint's bottom (metres).
uniform float minUnderPrint;

out vec3 WorldPos;
out vec3 GroundNormal;
// How much snow lies here, 0..1 (0 where the layer is sunk out of sight).
out float Cover;
// 1 where the ground map knows the ground; less is not drawn.
out float Valid;

// How far a sunk layer goes under the ground, so it never fights the
// terrain's own surface for the same depth.
const float SINK = 0.08;

void main() {
    vec2 g = aGrid - cells * 0.5;
    vec2 p = levelCenter + g * spacing;
    // Toward its edge a ring turns into the next coarser grid (every
    // other vertex slides onto its neighbour), so the rings meet without
    // cracks.
    vec2 off = abs(p - levelCenter) / (cells * 0.5 * spacing);
    float edge = max(off.x, off.y);
    float morph = 0.0;
    if (outermost == 0) {
        morph = smoothstep(0.75, 0.95, edge);
        p -= mod(aGrid, 2.0) * spacing * morph;
    }

    float ground;
    Valid = groundHeight(p, ground) ? 1.0 : 0.0;
    // The ground's slope, from a little either side.
    const float e = 0.25;
    float x0, x1, z0, z1;
    bool around = groundHeight(p - vec2(e, 0.0), x0) && groundHeight(p + vec2(e, 0.0), x1)
        && groundHeight(p - vec2(0.0, e), z0) && groundHeight(p + vec2(0.0, e), z1);
    GroundNormal = around ? normalize(vec3(x0 - x1, 2.0 * e, z0 - z1)) : vec3(0.0, 1.0, 0.0);

    // Snow lies under open sky, above the water, on what is not too steep
    // (as on the terrain, see `snowShare`).
    // The same in every pass: the shadow maps see the layer where the
    // camera does.
    float cover = snowCover * skyExposureMap(vec3(p.x, ground, p.y), GroundNormal)
        * smoothstep(waterLine, waterLine + 0.4, ground);
    float n = weatherNoise(p * 1.7) * 0.6 + weatherNoise(p * 6.0) * 0.4;
    float threshold = mix(0.92, 0.45, cover) + (n - 0.5) * 0.2;
    float share = smoothstep(threshold - 0.08, threshold + 0.08, GroundNormal.y)
        * smoothstep(0.0, 0.2, cover);
    // The outermost ring sinks into the ground at its edge, where the
    // terrain's own snow takes over.
    float fade = outermost == 1 ? 1.0 - smoothstep(0.55, 0.92, edge) : 1.0;
    Cover = share * fade;

    // The trails blurred to what this grid can hold (1.5 times its spacing,
    // more where it turns into the next ring), so coarse rings make soft
    // dents instead of sharp facets; the fragments add the detail back
    // through their normals.
    float left = snowDepth - snowDentBlurred(p, 1.5 * spacing * (1.0 + morph));
    float top = max(left, min(minUnderPrint, snowDepth)) * Cover;
    WorldPos = vec3(p.x, ground + top - (1.0 - Cover) * SINK, p.y);
    gl_Position = viewProjection * vec4(WorldPos, 1.0);
}
