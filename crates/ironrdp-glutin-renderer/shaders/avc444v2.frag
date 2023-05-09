precision lowp float;

uniform vec2 screen_size;
uniform vec2 stride_scale;
uniform sampler2D main_y_texture;
uniform sampler2D main_u_texture;
uniform sampler2D main_v_texture;

uniform sampler2D aux_y_texture;
uniform sampler2D aux_u_texture;
uniform sampler2D aux_v_texture;

const vec2 CUTOFF = vec2(30.0/255.0, 30.0/255.0);

// YUV to RGB conversion matrix from https://github.com/mbebenita/Broadway/blob/master/Player/YUVCanvas.js
const mat4 conversion = mat4(
    1.16438,    0.00000,    1.79274,    -0.97295,
    1.16438,    -0.21325,   -0.53291,   0.30148,
    1.16438,    2.11240,    0.00000,    -1.13340,
    0,          0,          0,          1
);

const vec2 half_offset = vec2(0.5, 0.5);

void main(void) {
    
    vec2 coordinates = vec2(gl_FragCoord.x, screen_size.y - gl_FragCoord.y);
    
    // Query the main view
    vec2 main_tex_coord = (coordinates / screen_size) * stride_scale;
    float main_y_channel = texture2D(main_y_texture, main_tex_coord).x;
    float main_u_channel = texture2D(main_u_texture, main_tex_coord).x;
    float main_v_channel = texture2D(main_v_texture, main_tex_coord).x;
   
    coordinates = coordinates - half_offset;
    float left_x = coordinates.x * 0.5;
    float right_x = (screen_size.x + coordinates.x) * 0.5;

    // Auxiliary view
    // Left
    vec2 left_half = (vec2(left_x, coordinates.y) + half_offset)/screen_size * stride_scale;
    float aux_ub4 = texture2D(aux_y_texture, left_half).x;
    float aux_ub6 = texture2D(aux_u_texture, left_half).x;
    float aux_ub8 = texture2D(aux_v_texture, left_half).x;

    // Right
    vec2 right_half = (vec2(right_x, coordinates.y) + half_offset)/screen_size * stride_scale;
    float aux_vb5 = texture2D(aux_y_texture, right_half).x;
    float aux_vb7 = texture2D(aux_u_texture, right_half).x;
    float aux_vb9 = texture2D(aux_v_texture, right_half).x;

    // Create aux view
    vec2 aux_uv_main = vec2(aux_ub4, aux_vb5);
    vec2 aux_uv_left = vec2(aux_ub6, aux_vb7);
    vec2 aux_uv_right = vec2(aux_ub8, aux_vb9);

    float is_x_odd = mod(coordinates.x, 2.0);
    float is_y_odd = mod(coordinates.y, 2.0);
    float is_xy_even = (1.0 - is_x_odd) * (1.0 - is_y_odd);
    float is_x_mod_4 = float(mod(coordinates.x, 4.0) < 1.0);
    
    // If x is odd then  b4,b5 have data 
    //  else if y is odd then  b6,b7 have data when x is divisible by 4 
    //  else b8,b9 have data
    vec2 uv_channels = is_x_odd * aux_uv_main + (1.0 - is_x_odd) * is_y_odd * (is_x_mod_4 * aux_uv_left + (1.0 - is_x_mod_4) * aux_uv_right);

    // Apply the reverse filter when both (x, y) are even based on [MS-RDPEGFX] rule
    vec2 main_uv=vec2(main_u_channel, main_v_channel);
    vec2 uv_augmented  = clamp(main_uv * 4.0 - aux_uv_main - aux_uv_left - aux_uv_right, vec2(0.0, 0.0), vec2(1.0, 1.0));
    vec2 uv_diff = abs(uv_augmented - main_uv);
    bvec2 uv_is_greater = greaterThan(uv_diff, CUTOFF);
    vec2 uv_is_greater_vec = vec2(uv_is_greater.x, uv_is_greater.y);
    vec2 uv_touse = uv_augmented * uv_is_greater_vec + main_uv * (vec2(1.0, 1.0) - uv_is_greater_vec);
    
    vec2 final_uv_channels = is_xy_even *  uv_touse + (1.0 - is_xy_even) * uv_channels;
    vec4 channels = vec4(main_y_channel, final_uv_channels.x, final_uv_channels.y, 1.0);
    vec3 rgb = (channels * conversion).xyz;
    gl_FragColor = vec4(rgb, 1.0);
}