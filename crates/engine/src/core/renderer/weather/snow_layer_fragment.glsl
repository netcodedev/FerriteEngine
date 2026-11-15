// The snow layer's surface (`weather/snow_layer.rs`): lit like the rest of
// the scene, its slope from the ground and the trails pressed into it.
// Comes after `LIGHTING_GLSL`, the cloud field and `cloudShadow`.

in vec3 WorldPos;
in vec3 GroundNormal;
in float Cover;
in float Valid;
in float Lift;

out vec4 FragColor;

// The next finer ring draws inside this square (centre x, z and half its
// side); this ring leaves it to it.
uniform int hasFiner;
uniform vec2 finerCenter;
uniform float finerHalf;

// How far the trails press the snow down at `p`, smoothed (metres).
float trailDent(vec2 p) {
    return snowDentBlurred(p, 0.08);
}

// How much of the sun reaches the bottom of a print at `p`, `depth`
// metres below the top, past its own walls: the ray toward the sun is
// followed up to the top through the smooth height field.
float printShadow(vec2 p, float depth, vec3 L) {
    if (depth <= 0.003 || L.y <= 0.02) {
        return 1.0;
    }
    vec2 run = L.xz / max(L.y, 0.12);
    float lit = 1.0;
    const int STEPS = 10;
    for (int i = 1; i <= STEPS; ++i) {
        float rise = depth * float(i) / float(STEPS);
        float surface = depth - trailDent(p + run * rise) * Cover;
        lit = min(lit, 1.0 - smoothstep(rise - 0.004, rise + 0.02, surface));
    }
    return lit;
}

void main() {
    if (Valid < 0.999 || Cover < 0.01) {
        discard;
    }
    vec2 inner = abs(WorldPos.xz - finerCenter);
    if (hasFiner == 1 && max(inner.x, inner.y) < finerHalf) {
        discard;
    }
    // Shadow maps and the ambient occlusion's depth need only the depth.
    if (isShadowPass == 1) {
        FragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }
    // The trails are shaded from their smooth height field alone, as with
    // smooth shading on a rounded mesh: neither the triangles nor the
    // shadow map (which holds the layer without trails) show in the light.
    vec2 p = WorldPos.xz;
    // Slope of the pressed snow, on the ground's, from the trails blurred
    // a little (one mipmap, chosen here, not by the screen: slopes of the
    // bilinear map are flat facets).
    const float e = 0.06;
    float dx = trailDent(p + vec2(e, 0.0)) - trailDent(p - vec2(e, 0.0));
    float dz = trailDent(p + vec2(0.0, e)) - trailDent(p - vec2(0.0, e));
    vec3 N = normalize(GroundNormal + vec3(dx, 0.0, dz) / (2.0 * e) * Cover);

    vec3 albedo = SNOW_ALBEDO;
    float roughness = 1.0;
    float metallic = 0.0;
    applySnow(WorldPos, N, 1.0, albedo, N, roughness, metallic);
    // Pressed snow is packed: a little grey and smoother. Inside a print
    // the light is blocked by its own walls: the sun along its height
    // field, the sky by how far it lies below what is around it.
    float depth = trailDent(p) * Cover;
    float around = snowDentBlurred(p, 0.4) * Cover;
    float pressed = smoothstep(0.0, 0.03, depth);
    albedo *= 1.0 - 0.12 * pressed;
    roughness = mix(roughness, 0.6, pressed);
    float ao = 1.0 - 0.55 * smoothstep(0.0, 0.12, depth - around * 0.5);

    // Lit as the untouched top above it: that is what the shadow maps
    // hold, so the print is not shadowed by the snow it is pressed into.
    vec3 top = WorldPos + vec3(0.0, Lift, 0.0);
    float sun = cloudShadow(top) * printShadow(p, depth, normalize(sunDirection));
    vec3 color = shadeSurface(top, N, GroundNormal, albedo, roughness, metallic, ao, sun);
    FragColor = vec4(toDisplay(color), 1.0);
}
