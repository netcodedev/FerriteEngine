// Vertex side of a material: what the shading stage gets, and the wind.

out vec3 Normal;
out vec3 WorldPos;
out vec2 TexCoords;
out vec4 VertexColor;
out vec3 ObjectPos;
flat out vec3 PivotPos;
flat out vec3 Tint;

uniform vec3 cameraPosition;
// How much this draw's models sway (a scatter output's wind; 1 for
// models placed as entities).
uniform float windScale;

// The wind's offset of a vertex at `worldPos` of a model standing at
// `pivot`: along the wind by the square of the height over the pivot in
// units of `height` (the top sways, the base stays; two sines and gusts
// rolling across), and leaves fluttering along their normal.
vec3 materialWind(vec3 worldPos, vec3 pivot, vec3 normal, float amount, float mask, float flutter, float flutterMask, float height) {
    vec3 offset = vec3(0.0);
    float a = windScale * amount * mask;
    if (a > 0.0) {
        float H = max(height, 0.01);
        float h = max(worldPos.y - pivot.y, 0.0) / H;
        float t = wind.w;
        float phase = dot(pivot.xz, vec2(0.71, 0.37));
        float gust = 0.5 + 0.5 * sin(dot(pivot.xz, wind.xy) * 0.08 - t * 0.9);
        float sway = sin(t * 1.9 + phase) * 0.5 + sin(t * 3.7 + phase * 1.7) * 0.2 + gust * 0.7;
        offset.xz = wind.xy * (a * wind.z * 0.35 * H * h * h * sway);
    }
    float f = windScale * flutter * flutterMask;
    if (f > 0.0) {
        float t = wind.w;
        float phase = dot(worldPos, vec3(2.1, 1.7, 2.9));
        offset += normal * (f * wind.z * 0.03 * (sin(t * 9.0 + phase) + 0.5 * sin(t * 13.7 + phase * 1.3)));
    }
    return offset;
}
