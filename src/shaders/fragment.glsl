#version 330 core // Or a compatible version for your OpenGL context

in vec4 outColor; // Input color from the vertex shader
out vec4 FragColor; // Output color of the fragment

void main()
{
    FragColor = outColor; // Set the fragment color
}