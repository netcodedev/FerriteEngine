#version 330 core

layout (location = 0) in vec3 position;

uniform mat4 view;
uniform mat4 projection;
uniform vec3 color;

out vec3 fColor;

void main(){
   gl_Position = projection * view * vec4(position, 1.0);
   fColor = color;
}