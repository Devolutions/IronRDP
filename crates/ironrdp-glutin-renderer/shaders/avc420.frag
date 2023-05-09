precision lowp float;

uniform vec2 screen_size;
uniform vec2 stride_scale;
uniform sampler2D main_y_texture;
uniform sampler2D main_u_texture;
uniform sampler2D main_v_texture;

uniform sampler2D aux_y_texture;
uniform sampler2D aux_u_texture;
uniform sampler2D aux_v_texture;

// YUV to RGB conversion matrix from https://github.com/mbebenita/Broadway/blob/master/Player/YUVCanvas.js
const mat4 conversion = mat4(
    1.16438,    0.00000,    1.79274,    -0.97295,
    1.16438,    -0.21325,   -0.53291,   0.30148,
    1.16438,    2.11240,    0.00000,    -1.13340,
    0,          0,          0,          1
);

void main(void) {
    // Inverted image
    vec2 coordinates = vec2(gl_FragCoord.x, screen_size.y - gl_FragCoord.y);
    
    // Scale from [0..width, 0..height] to [0..1.0, 0..1.0] range and 
    // then scale to eliminate the stride padding 
    vec2 tex_coord = ((coordinates) / screen_size) * stride_scale;
    
    float main_y_channel = texture2D(main_y_texture, tex_coord).x;
    float main_u_channel = texture2D(main_u_texture, tex_coord).x;
    float main_v_channel = texture2D(main_v_texture, tex_coord).x;
    
    vec4 channels = vec4(main_y_channel, main_u_channel, main_v_channel, 1.0);
    gl_FragColor = channels * conversion;
}