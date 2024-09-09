#version 330 core // Or a compatible version for your OpenGL context

layout (location = 0) in vec3 aPos; // Position attribute

out vec4 outColor; // Output color to the fragment shader

uniform mat4 model;
uniform mat4 view;
uniform mat4 projection;

void main()
{
    gl_Position = projection * view * model * vec4(aPos, 1.0); // Apply transformations
    
    // Calculate the dot product between the vertex normal and the view direction
    float intensity = dot(normalize(vec3(model * vec4(aPos, 1.0))), normalize(vec3(view * vec4(0.0, 0.0, -1.0, 0.0))));
    
    // Invert the intensity to light all faces
    intensity = 0.9 - intensity;
    
    // Darken the color based on the intensity
    outColor = vec4(0.3 * intensity, 0.6 * intensity, 0.4 * intensity, 0.1); // Pass the color to the fragment shader
}