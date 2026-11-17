
// The scattered models beyond the loaded chunks (see
// `terrain::far_scatter`): a card per model, standing upright on its spot
// and turned toward the viewer, showing the picture taken from nearest the
// side it is seen from (see `scatter::impostor`).

// Corner of the card: 0..1 across, 0..1 up.
layout (location = 0) in vec2 corner;
// Where the model stands (x, y, z) and how it is turned (radians).
layout (location = 1) in vec4 model;
// How much larger than the model it is drawn (thinned out far away, see
// `far_scatter::DENSE_REACH`).
layout (location = 2) in float scale;

uniform mat4 viewProjection;
uniform vec3 viewer;
// Columns whose chunks carry their own models (see `columns.glsl`).
uniform sampler2D planted;
uniform ivec2 plantedOrigin;
// The pictures: how far they reach sideways, from where to where up.
uniform vec3 card;
uniform int frames;

out vec3 WorldPos;
out vec2 Uv;
flat out float Frame;
flat out float Turn;

void main() {
    if (columnIn(planted, plantedOrigin, model.xz)) {
        // Outside the view: nothing drawn.
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        return;
    }
    vec2 toModel = model.xz - viewer.xz;
    float distance = length(toModel);
    vec2 f = distance > 1e-3 ? toModel / distance : vec2(0.0, 1.0);
    vec3 right = vec3(-f.y, 0.0, f.x);
    // Toward the viewer, in the model's own frame (turned back by its turn).
    vec2 back = -f;
    float c = cos(model.w), s = sin(model.w);
    vec2 local = vec2(c * back.x - s * back.y, s * back.x + c * back.y);
    float step = 6.28318531 / float(frames);
    int frame = int(round(atan(local.y, local.x) / step));
    Frame = float(((frame % frames) + frames) % frames);
    Turn = model.w;

    vec3 world = model.xyz + scale * (right * ((corner.x * 2.0 - 1.0) * card.x)
        + vec3(0.0, mix(card.y, card.z, corner.y), 0.0));
    gl_Position = viewProjection * vec4(world, 1.0);
    WorldPos = world;
    Uv = corner;
}
