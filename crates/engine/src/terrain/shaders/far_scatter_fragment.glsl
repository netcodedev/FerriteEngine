// A far model's card (see `far_scatter_vertex.glsl`), lit like the models
// near by and hazed like the ground behind it. After `pbr::LIGHTING_GLSL`
// and `HAZE_GLSL`.

in vec3 WorldPos;
in vec2 Uv;
flat in float Frame;
flat in float Turn;

// The pictures (see `scatter::impostor`): colour and model-space normals,
// both premultiplied by the cut-out.
uniform sampler2DArray albedoMap;
uniform sampler2DArray normalMap;

out vec4 FragColor;

const float CROWN_OCCLUSION = 0.7;
const float CROWN_SUN = 0.75;

void main() {
    snowGradients(WorldPos);
    vec4 base = texture(albedoMap, vec3(Uv, Frame));
    // The smaller mip levels average the leaves' cut-out away: the crowns
    // would thin out with distance. Their cover scaled back up keeps them
    // as full as up close.
    float level = textureQueryLod(albedoMap, Uv).x;
    if (base.a * (1.0 + max(level, 0.0) * 0.15) < 0.5) {
        discard;
    }
    vec3 albedo = base.rgb / base.a;
    vec4 encoded = texture(normalMap, vec3(Uv, Frame));
    vec3 n = encoded.xyz / max(encoded.a, 1e-3) * 2.0 - 1.0;
    // Into the world: turned as the model is.
    float c = cos(Turn), s = sin(Turn);
    n = vec3(c * n.x + s * n.z, n.y, -s * n.x + c * n.z);
    // A crown seen from far is lit more as a whole than leaf by leaf.
    vec3 N = normalize(mix(normalize(n), vec3(0.0, 1.0, 0.0), 0.15));
    float roughness = 0.85;
    float metallic = 0.0;
    applyWeather(WorldPos, N, albedo, N, roughness, metallic);
    // The crown shades itself, as the model near by does in the shadow
    // maps: less sky and sun reach into it than onto a flat card.
    vec3 color = shadeSurface(WorldPos, N, N, albedo, roughness, metallic, CROWN_OCCLUSION, CROWN_SUN);
    FragColor = vec4(toDisplay(terrainHaze(color, WorldPos)), 1.0);
}
