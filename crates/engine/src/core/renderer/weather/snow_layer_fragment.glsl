// The snow layer's surface (`weather/snow_layer.rs`): lit like the rest of
// the scene, its slope from the ground and the trails pressed into it.
// Comes after `LIGHTING_GLSL`, the cloud field and `cloudShadow`.

in vec3 WorldPos;
in vec3 GroundNormal;
in float Cover;
in float Valid;

out vec4 FragColor;

// The next finer ring draws inside this square (centre x, z and half its
// side); this ring leaves it to it.
uniform int hasFiner;
uniform vec2 finerCenter;
uniform float finerHalf;

void main() {
    if (Valid < 0.999 || Cover < 0.01) {
        discard;
    }
    vec2 inner = abs(WorldPos.xz - finerCenter);
    if (hasFiner == 1 && max(inner.x, inner.y) < finerHalf) {
        discard;
    }
    // Slope of the pressed snow, on the ground's.
    const float e = 0.05;
    float dx = snowDent(WorldPos.xz + vec2(e, 0.0)) - snowDent(WorldPos.xz - vec2(e, 0.0));
    float dz = snowDent(WorldPos.xz + vec2(0.0, e)) - snowDent(WorldPos.xz - vec2(0.0, e));
    vec3 Ng = normalize(GroundNormal + vec3(dx, 0.0, dz) / (2.0 * e) * Cover);

    vec3 albedo = SNOW_ALBEDO;
    vec3 N = Ng;
    float roughness = 1.0;
    float metallic = 0.0;
    applySnow(WorldPos, Ng, 1.0, albedo, N, roughness, metallic);
    // Pressed snow is packed: a little grey, smoother, shaded inside.
    vec2 trail = snowTrail(WorldPos.xz) * Cover;
    albedo *= 1.0 - 0.15 * trail.r;
    roughness = mix(roughness, 0.4, trail.r);
    float ao = 1.0 - 0.35 * trail.r * smoothstep(0.0, 0.25, snowDepth);

    vec3 color = shadeSurface(WorldPos, N, Ng, albedo, roughness, metallic, ao, cloudShadow(WorldPos));
    FragColor = vec4(toDisplay(color), 1.0);
}
