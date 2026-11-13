// Cloud pass: raymarches the cloud slab at reduced resolution into a
// texture the sky blends in (rgb: scattered light, a: opacity).

in vec2 ndcPos;
out vec4 FragColor;

void main() {
    vec3 dir = viewRay(ndcPos);
    FragColor = sampleClouds(dir, dayFactorOf(sunDirection), vec3(0.0));
}
