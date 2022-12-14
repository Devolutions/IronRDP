precision mediump float;

uniform mat4 u_projection;
attribute vec2 a_position;

void main(){
    gl_Position = u_projection * vec4(a_position, 0.0, 1.0);
}