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
    vec2 coordinates = vec2(gl_FragCoord.x, screen_size.y - gl_FragCoord.y) ;
    vec2 main_tex_coord = (coordinates  / screen_size) * stride_scale;
    // Query the main view
    float main_y_channel = texture2D(main_y_texture, main_tex_coord).x;
    float main_u_channel = texture2D(main_u_texture, main_tex_coord).x;
    float main_v_channel = texture2D(main_v_texture, main_tex_coord).x;
    
    coordinates = coordinates - half_offset;

    float offset = floor(mod(coordinates.y, 16.0) * 0.5);
    float start_y = offset + floor(coordinates.y / 16.0) * 16.0;
    // Auxiliary view
    vec2 aux_tex_coord = vec2(coordinates.x, start_y) + half_offset;
    vec2 aux_tex_coord_next = aux_tex_coord + vec2(1.0, 0.0);

    vec2 top_half = aux_tex_coord/screen_size * stride_scale;
    vec2 top_half_next = aux_tex_coord_next/screen_size * stride_scale;
    vec2 bottom_half = (aux_tex_coord + vec2(0.0, 8.0))/screen_size * stride_scale;
    vec2 bottom_half_next = (aux_tex_coord_next + vec2(0.0, 8.0))/screen_size * stride_scale;

    float aux_b4 = texture2D(aux_y_texture, top_half).x;
    float aux_b5 = texture2D(aux_y_texture, bottom_half).x;
    float next_u = texture2D(aux_y_texture, top_half_next).x;
    float next_v = texture2D(aux_y_texture, bottom_half_next).x;
    vec2 aux_uv_additional = vec2(next_u, next_v);

    float aux_b6 = texture2D(aux_u_texture, main_tex_coord).x;
    float aux_b7 = texture2D(aux_v_texture, main_tex_coord).x;

    float is_x_odd = mod(coordinates.x, 2.0);
    float is_y_odd = mod(coordinates.y, 2.0);
    float is_xy_even = (1.0 - is_x_odd) * (1.0 - is_y_odd);

    vec2 aux_uv_main = vec2(aux_b4, aux_b5);
    vec2 aux_uv_secondary = vec2(aux_b6, aux_b7);
    
    vec2 uv_channels = is_y_odd * aux_uv_main + (1.0 - is_y_odd) * is_x_odd * aux_uv_secondary;

    // Apply the reverse filter when both (x, y) are even based on [MS-RDPEGFX] rule
    vec2 main_uv=vec2(main_u_channel, main_v_channel);
    vec2 uv_augmented  = clamp(main_uv * 4.0 - aux_uv_main - aux_uv_secondary - aux_uv_additional, vec2(0.0, 0.0), vec2(1.0, 1.0));
    vec2 uv_diff = abs(uv_augmented - main_uv);
    bvec2 uv_is_greater = greaterThan(uv_diff, CUTOFF);
    vec2 uv_is_greater_vec = vec2(uv_is_greater.x, uv_is_greater.y);
    vec2 uv_touse = uv_augmented * uv_is_greater_vec + main_uv * (vec2(1.0, 1.0) - uv_is_greater_vec);
    
    vec2 final_uv_channels = is_xy_even *  uv_touse + (1.0 - is_xy_even) * uv_channels;

    vec4 channels = vec4(main_y_channel, final_uv_channels.x, final_uv_channels.y, 1.0);
    vec3 rgb = (channels * conversion).xyz;
    gl_FragColor = vec4(rgb, 1.0);
}