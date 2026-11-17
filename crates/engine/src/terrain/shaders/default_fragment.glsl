// The engine's own ground (see `terrain::hills`): the vertices' colour, rock
// on steep slopes, lit like models, wet and snowed on by the weather.

void main() {
    if (hiddenHorizon()) {
        discard;
    }
    if (isShadowPass == 1) {
        FragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }
    vec3 Ng = normalize(Normal);
    float slope = 1.0 - abs(Ng.y);
    vec3 rock = vec3(0.40, 0.37, 0.33);
    vec3 albedo = pow(mix(GroundColor, rock, smoothstep(0.35, 0.6, slope)), vec3(2.2));
    vec3 N = Ng;
    float roughness = 0.9;
    float metallic = 0.0;
    float ao = 1.0;
    if (wetness + snowCover > 0.0) {
        float exposure = skyExposure(WorldPos, Ng) * smoothstep(waterLine, waterLine + 0.4, WorldPos.y);
        applyRain(WorldPos, Ng, exposure, albedo, N, roughness, metallic);
        float snow = snowCover * exposure * (1.0 - smoothstep(0.5, 0.8, slope));
        albedo = mix(albedo, vec3(0.8), snow);
        roughness = mix(roughness, 0.6, snow);
    }
    vec3 color = shadeSurface(WorldPos, N, Ng, albedo, roughness, metallic, ao, cloudShadow(WorldPos));
    FragColor = vec4(toDisplay(terrainHaze(color, WorldPos)), 1.0);
}
