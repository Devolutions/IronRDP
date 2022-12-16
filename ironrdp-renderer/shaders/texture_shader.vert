precision mediump float;

attribute vec2 a_position;
attribute vec2 a_tex_coord;
varying vec2 v_texCoord;

void main(){
    v_texCoord = a_tex_coord;
    gl_Position = vec4(a_position, 0.0, 1.0);
}