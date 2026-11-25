// Ray traced sun shadows, one ray per pixel: from the surface the view
// shows toward a random point on the sun's disc (a new one every frame,
// so the frames blended later give soft shadows). Opaque geometry stops
// the ray; cut-out geometry (leaf cards) stops it where its opacity
// texture (a layer of `opacityAtlas`, found through the instance's record
// in `cutouts`) is at least the cut-off, as the drawn leaves are cut out.
// A cut-out without a record lets a share of the light through instead.
// Writes the light that arrives (r, 1 = all) and how far along the ray
// the first thing in the way was (g, metres; 0 when nothing was), which
// tells the filter how wide the penumbra is.
// `rays.glsl` comes before this.

in vec2 uv;
out vec4 FragColor;

uniform vec3 toLight;
// Angular radius of the light's disc (radians).
uniform float lightRadius;
uniform float maxDistance;
// From these distances (metres from the camera, horizontally) on, the
// ground is meshed 2, 4 and 8 metres apart (1 nearer; `terrain::chunk`).
uniform vec3 groundStepsFrom;

void main() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    float depth = texelFetch(sceneDepth, texel, 0).r;
    if (depth >= 1.0) {
        // Sky.
        FragColor = vec4(1.0, 0.0, 0.0, 1.0);
        return;
    }
    vec3 position = relativeAt(texel);
    vec3 normal = normalAt(texel, position);

    // A random direction within the light's disc.
    vec3 l = normalize(toLight);
    vec3 tangent = normalize(abs(l.y) < 0.99 ? cross(l, vec3(0.0, 1.0, 0.0)) : cross(l, vec3(1.0, 0.0, 0.0)));
    vec3 bitangent = cross(l, tangent);
    float radius = lightRadius * sqrt(noise(gl_FragCoord.xy, 0));
    float angle = 6.2831853 * noise(gl_FragCoord.xy, 1);
    vec3 direction = normalize(l + (tangent * cos(angle) + bitangent * sin(angle)) * radius);

    // Off the surface by more the further it is (depth precision).
    float distanceToEye = length(position);
    vec3 origin = eye + position + normal * (0.02 + distanceToEye * 0.002);

    // The ground does not shadow itself within a few of its facets.
    float across = length(position.xz);
    float step = across < groundStepsFrom.x ? 1.0 : across < groundStepsFrom.y ? 2.0 : across < groundStepsFrom.z ? 4.0 : 8.0;
    float blocker;
    float light = traceShadowPastGround(origin, direction, maxDistance, 2.5 * step, blocker);
    FragColor = vec4(light, blocker, 0.0, 1.0);
}
