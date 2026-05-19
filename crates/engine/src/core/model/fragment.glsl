#version 330 core

in vec3 Normal;
in vec3 toLightVector;
in vec2 TexCoords;

uniform sampler2D texture_diffuse;
uniform sampler2D texture_normals;
uniform sampler2D texture_shininess;
uniform sampler2D texture_specular;

out vec4 FragColor;

void main()
{
    vec4 albedo = texture(texture_diffuse, TexCoords);
    // Alpha-cutout for foliage cards: leaves rely on the diffuse map's alpha
    // channel to punch holes in otherwise rectangular polygons. Without the
    // discard, transparent texels render as plain (lit) colour and the leaf
    // cards look like solid white quads.
    if (albedo.a < 0.5) {
        discard;
    }

    vec3 unitNormal = normalize(Normal * texture(texture_normals, TexCoords).rgb);
    vec3 unitToLightVector = normalize(toLightVector);
    float intensity = dot(unitNormal, unitToLightVector);

    float sunAlt = unitToLightVector.y;
    float dayFactor = smoothstep(-0.10, 0.20, sunAlt);
    float ambient = mix(0.08, 0.5, dayFactor);
    float diffuse = max(intensity, 0.0) * dayFactor;
    float brightness = ambient + diffuse * 0.6;
    vec3 color = brightness * albedo.rgb;

    FragColor = vec4(color, 1.0);
}