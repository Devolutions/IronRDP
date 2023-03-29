precision lowp float;
  
varying vec2 v_texCoord;
uniform sampler2D screen_texture;

void main(void) {
    vec4 color = texture2D(screen_texture, v_texCoord);
    gl_FragColor = color;
}