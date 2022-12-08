use glow::*;
use ironrdp::Rectangle;

use std::{iter::FromIterator, mem::size_of, slice::from_raw_parts, sync::Arc};

fn cast_as_bytes<T>(input: &[T]) -> &[u8] {
    unsafe { from_raw_parts(input.as_ptr() as *const u8, input.len() * size_of::<T>()) }
}

pub struct DrawingTexture {
    gl: Arc<Context>,
    texture: Texture,
    location: UniformLocation,
    height: i32,
}

impl DrawingTexture {
    unsafe fn new(gl_ref: Arc<Context>, program: Program, location: &str, height: i32) -> Self {
        let gl = &gl_ref;
        let location = gl.get_uniform_location(program, location).unwrap();
        let texture = gl.create_texture().unwrap();
        gl.bind_texture(TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(TEXTURE_2D, TEXTURE_MAG_FILTER, NEAREST as i32);
        gl.tex_parameter_i32(TEXTURE_2D, TEXTURE_MIN_FILTER, NEAREST as i32);
        gl.tex_parameter_i32(TEXTURE_2D, TEXTURE_WRAP_S, CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(TEXTURE_2D, TEXTURE_WRAP_T, CLAMP_TO_EDGE as i32);
        gl.bind_texture(TEXTURE_2D, None);
        DrawingTexture {
            gl: gl_ref.clone(),
            texture,
            location,
            height,
        }
    }

    /// # Safety
    ///
    /// TODO: Safety notes
    pub unsafe fn bind_texture(&self, gl: &Context, pixels: &[u8], stride: i32) {
        gl.bind_texture(TEXTURE_2D, Some(self.texture));
        gl.tex_image_2d(
            TEXTURE_2D,
            0,
            glow::R8 as i32,
            stride,
            self.height,
            0,
            glow::RED,
            UNSIGNED_BYTE,
            Some(pixels),
        );
        gl.bind_texture(TEXTURE_2D, None);
    }
}

impl Drop for DrawingTexture {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_texture(self.texture);
        }
    }
}

pub struct DrawingTextures {
    y: DrawingTexture,
    u: DrawingTexture,
    v: DrawingTexture,
    height: i32,
    index: i32,
}

impl DrawingTextures {
    /// # Safety
    ///
    /// TODO: Safety notes
    pub unsafe fn new(
        gl_ref: Arc<Context>,
        program: Program,
        height: i32,
        y_location: &str,
        u_location: &str,
        v_location: &str,
        index: i32,
    ) -> Self {
        let y = DrawingTexture::new(gl_ref.clone(), program, y_location, height);
        let u = DrawingTexture::new(gl_ref.clone(), program, u_location, height / 2);
        let v = DrawingTexture::new(gl_ref, program, v_location, height / 2);

        DrawingTextures { y, u, v, height, index }
    }

    /// # Safety
    ///
    /// TODO: Safety notes
    pub unsafe fn bind(&self, gl: &Context, data: &[u8], stride_0: usize, stride_1: usize) {
        let luma = stride_0 * self.height as usize;
        let chroma = stride_1 * (self.height as usize / 2);
        let y_pixels = &data[0..luma];
        let u_pixels = &data[luma..luma + chroma];
        let v_pixels = &data[luma + chroma..];

        self.y.bind_texture(gl, y_pixels, stride_0 as i32);
        self.u.bind_texture(gl, u_pixels, stride_1 as i32);
        self.v.bind_texture(gl, v_pixels, stride_1 as i32);

        self.activate(gl);
    }

    unsafe fn activate(&self, gl: &Context) {
        gl.uniform_1_i32(Some(&self.y.location), self.index);
        gl.active_texture(TEXTURE0 + self.index as u32);
        gl.bind_texture(TEXTURE_2D, Some(self.y.texture));

        gl.uniform_1_i32(Some(&self.u.location), self.index + 1);
        gl.active_texture(TEXTURE0 + 1 + self.index as u32);
        gl.bind_texture(TEXTURE_2D, Some(self.u.texture));

        gl.uniform_1_i32(Some(&self.v.location), self.index + 2);
        gl.active_texture(TEXTURE0 + 2 + self.index as u32);
        gl.bind_texture(TEXTURE_2D, Some(self.v.texture));
        gl.active_texture(TEXTURE0 + 3 + self.index as u32);
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ShaderType {
    Avc420,
    Avc444,
    Avc444v2,
    TextureShader,
}

impl ShaderType {
    fn get_fragment_shader_source(&self) -> String {
        let mut source = String::new();
        match self {
            ShaderType::Avc444 => {
                source.push_str(include_str!("../shaders/avc444.frag"));
            }
            ShaderType::Avc444v2 => {
                source.push_str(include_str!("../shaders/avc444v2.frag"));
            }
            ShaderType::Avc420 => {
                source.push_str(include_str!("../shaders/avc420.frag"));
            }
            ShaderType::TextureShader => {
                source.push_str(include_str!("../shaders/texture_shader.frag"));
            }
        }
        source
    }

    fn get_vertex_shader_source(&self) -> String {
        let mut source = String::new();
        match self {
            ShaderType::Avc420 | ShaderType::Avc444 | ShaderType::Avc444v2 => {
                source.push_str(include_str!("../shaders/avc.vert"));
            }
            ShaderType::TextureShader => {
                source.push_str(include_str!("../shaders/texture_shader.vert"));
            }
        }
        source
    }

    unsafe fn create_shader(&self, gl: &Context, shader_version: &str) -> crate::Result<Program> {
        let vertex_shader_source = self.get_vertex_shader_source();
        let fragment_shader_source = self.get_fragment_shader_source();

        let shader_sources = [
            (glow::VERTEX_SHADER, vertex_shader_source),
            (glow::FRAGMENT_SHADER, fragment_shader_source),
        ];

        let program = gl.create_program()?;
        let mut shaders = Vec::with_capacity(shader_sources.len());

        for (shader_type, shader_source) in shader_sources.iter() {
            let shader = gl.create_shader(*shader_type)?;
            gl.shader_source(shader, &format!("{}\n{}", shader_version, shader_source));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(crate::Error::from(gl.get_shader_info_log(shader)));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }

        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(crate::Error::from(gl.get_program_info_log(program)));
        }

        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }

        Ok(program)
    }
}

pub struct OffscreenBuffer {
    gl: Arc<Context>,
    texture: Texture,
    frame_buffer: Framebuffer,
}

impl Drop for OffscreenBuffer {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_texture(self.texture);
        }
    }
}

impl OffscreenBuffer {
    unsafe fn new(gl_ref: Arc<Context>, width: i32, height: i32) -> crate::Result<OffscreenBuffer> {
        let gl = &gl_ref;
        let texture = gl.create_texture()?;
        gl.bind_texture(TEXTURE_2D, Some(texture));
        gl.tex_image_2d(TEXTURE_2D, 0, RGB as i32, width, height, 0, RGB, UNSIGNED_BYTE, None);

        let frame_buffer = gl.create_framebuffer()?;
        gl.bind_framebuffer(FRAMEBUFFER, Some(frame_buffer));
        gl.framebuffer_texture_2d(FRAMEBUFFER, COLOR_ATTACHMENT0, TEXTURE_2D, Some(texture), 0);
        gl.bind_framebuffer(FRAMEBUFFER, None);
        Ok(OffscreenBuffer {
            gl: gl_ref.clone(),
            texture,
            frame_buffer,
        })
    }

    unsafe fn activate(&self) {
        self.gl.bind_framebuffer(FRAMEBUFFER, Some(self.frame_buffer));
    }

    unsafe fn deactivate(&self) {
        self.gl.bind_framebuffer(FRAMEBUFFER, None);
    }
}

pub struct TextureShaderProgram {
    gl: Arc<Context>,
    program: Program,
    screen_texture_location: UniformLocation,
    vertex_buffer: Buffer,
    vertex_array: VertexArray,
}

impl TextureShaderProgram {
    unsafe fn new(
        gl_ref: Arc<Context>,
        shader_version: &str,
        width: i32,
        height: i32,
        texture_width: i32,
        texture_height: i32,
    ) -> crate::Result<TextureShaderProgram> {
        let gl = &gl_ref;
        let program = ShaderType::TextureShader.create_shader(gl, shader_version)?;
        gl.use_program(Some(program));

        let a_position = gl.get_attrib_location(program, "a_position").unwrap();
        let a_tex_coord = gl.get_attrib_location(program, "a_tex_coord").unwrap();
        let screen_texture_location = gl.get_uniform_location(program, "screen_texture").unwrap();

        // If video height is higher trim the padding on the bottom
        let y_location = if texture_height > height {
            (texture_height - height) as f32 / height as f32
        } else {
            0.0
        };

        // If video width is higher trim the padding on the right
        let x_location = if texture_width > width {
            1.0 - (texture_width - width) as f32 / width as f32
        } else {
            1.0
        };

        #[rustfmt::skip]
        let data : Vec<f32> = vec![
            -1.0,   -1.0,   0.0,            y_location,
            1.0,    -1.0,   x_location,     y_location,
            -1.0,   1.0,    0.0,            1.0,
            -1.0,   1.0,    0.0,            1.0,
            1.0,    -1.0,   x_location,     y_location,
            1.0,    1.0,    x_location,     1.0,
        ];

        let vertex_array = gl.create_vertex_array()?;
        let vertex_buffer = gl.create_buffer().unwrap();
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(ARRAY_BUFFER, Some(vertex_buffer));

        gl.enable_vertex_attrib_array(a_tex_coord as u32);
        gl.enable_vertex_attrib_array(a_position as u32);

        gl.buffer_data_u8_slice(ARRAY_BUFFER, cast_as_bytes(data.as_ref()), DYNAMIC_DRAW);
        gl.vertex_attrib_pointer_f32(a_position, 2, FLOAT, false, 16, 0);
        gl.vertex_attrib_pointer_f32(a_tex_coord, 2, FLOAT, false, 16, 8);

        Ok(TextureShaderProgram {
            gl: gl_ref.clone(),
            program,
            vertex_buffer,
            vertex_array,
            screen_texture_location,
        })
    }

    unsafe fn set_location(&self, location: Rectangle) {
        self.gl.viewport(
            location.left as i32,
            location.top as i32,
            location.right as i32,
            location.bottom as i32,
        );
    }

    unsafe fn draw_texture(&self, texture: Texture) {
        let gl = &self.gl;
        gl.use_program(Some(self.program));

        gl.uniform_1_i32(Some(&self.screen_texture_location), 0);
        gl.active_texture(TEXTURE0);
        gl.bind_texture(TEXTURE_2D, Some(texture));
        gl.generate_mipmap(TEXTURE_2D);

        gl.bind_vertex_array(Some(self.vertex_array));
        gl.bind_buffer(ARRAY_BUFFER, Some(self.vertex_buffer));
        gl.draw_arrays(glow::TRIANGLES, 0, 6);
    }
}

impl Drop for TextureShaderProgram {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_buffer(self.vertex_buffer);
            self.gl.delete_vertex_array(self.vertex_array);
        }
    }
}

pub struct AvcShaderProgram {
    gl: Arc<Context>,
    program: Program,
    main: DrawingTextures,
    aux: Option<DrawingTextures>,
    width: i32,
    height: i32,
    vertex_buffer: Buffer,
    vertex_array: VertexArray,
    a_position: u32,
    stride_scale_location: UniformLocation,
}

impl Drop for AvcShaderProgram {
    fn drop(&mut self) {
        unsafe {
            let gl = self.gl.clone();
            gl.delete_buffer(self.vertex_buffer);
            gl.delete_vertex_array(self.vertex_array);
            gl.delete_program(self.program);
        }
    }
}

impl AvcShaderProgram {
    unsafe fn update_shader_data(&self, stride_scale: f32, regions: &[Rectangle]) {
        let gl = self.gl.clone();
        // Redraw the two triangles for the region
        #[rustfmt::skip]
        let data = Vec::<f32>::from_iter(regions.iter().flat_map(|region| {
            let left = region.left as f32;
            let right = region.right as f32;
            let top = self.height as f32 - region.bottom as f32;
            let bottom = self.height as f32 - region.top as f32;
            vec![
                left, top,
                right, top,
                right, bottom,
                right, bottom,
                left, bottom,
                left, top,
            ]
        }));

        gl.bind_buffer(ARRAY_BUFFER, Some(self.vertex_buffer));
        gl.bind_vertex_array(Some(self.vertex_array));
        gl.enable_vertex_attrib_array(self.a_position as u32);
        gl.buffer_data_u8_slice(ARRAY_BUFFER, cast_as_bytes(data.as_ref()), DYNAMIC_DRAW);
        gl.vertex_attrib_pointer_f32(self.a_position, 2, FLOAT, false, 8, 0);

        let data = vec![stride_scale, 1.0];
        gl.uniform_2_f32_slice(Some(&self.stride_scale_location), data.as_slice());
    }

    unsafe fn new(
        gl_ref: Arc<Context>,
        shader_version: &str,
        width: i32,
        height: i32,
        shader_type: ShaderType,
    ) -> crate::Result<AvcShaderProgram> {
        match shader_type {
            ShaderType::Avc444 | ShaderType::Avc444v2 | ShaderType::Avc420 => {}
            _ => return Err(crate::Error::from("Invalid shader type")),
        }

        let gl = gl_ref.clone();
        let program = shader_type.create_shader(&gl, shader_version)?;
        gl.use_program(Some(program));
        let stride_scale_location = gl.get_uniform_location(program, "stride_scale").unwrap();

        let a_position = gl.get_attrib_location(program, "a_position").unwrap();

        let screen_size_location = gl.get_uniform_location(program, "screen_size").unwrap();
        let u_projection_location = gl.get_uniform_location(program, "u_projection").unwrap();

        let data: Vec<f32> = vec![width as f32, height as f32];
        gl.uniform_2_f32_slice(Some(&screen_size_location), data.as_slice());

        // Projection matrix helps us map the points (0..width, 0.height) to (-1.0..1.0, -1.0..1.0) coordinates
        #[rustfmt::skip]
        let data: Vec<f32> = vec![
            2.0 / width as f32,         0.0,                        0.0,        -1.0,
            0.0,                        2.0 / height as f32,        0.0,        -1.0,
            0.0,                        0.0,                        0.0,         0.0,
            0.0,                        0.0,                        0.0,         1.0,
        ];
        gl.uniform_matrix_4_f32_slice(Some(&u_projection_location), true, data.as_slice());

        let main = DrawingTextures::new(
            gl_ref.clone(),
            program,
            height,
            "main_y_texture",
            "main_u_texture",
            "main_v_texture",
            0,
        );

        let aux = match shader_type {
            ShaderType::Avc444 | ShaderType::Avc444v2 => Some(DrawingTextures::new(
                gl_ref.clone(),
                program,
                height,
                "aux_y_texture",
                "aux_u_texture",
                "aux_v_texture",
                3,
            )),
            ShaderType::Avc420 => None,
            ShaderType::TextureShader => unreachable!(),
        };
        // Set parameters that are not going to change accross different programs.
        // All shaders within a context share parameters and must be of the same type
        // The paraemters here are going to remain constant

        let vertex_buffer = gl.create_buffer().unwrap();
        let vertex_array = gl.create_vertex_array()?;
        Ok(AvcShaderProgram {
            gl: gl_ref,
            program,
            main,
            aux,
            width,
            height,
            vertex_buffer,
            vertex_array,
            a_position,
            stride_scale_location,
        })
    }

    /// # Safety
    ///
    /// TODO: Safety notes
    pub unsafe fn draw(
        &self,
        main: &[u8],
        aux: Option<&[u8]>,
        stride_0: usize,
        stride_1: usize,
        regions: &Vec<Rectangle>,
    ) {
        let gl = self.gl.clone();
        gl.use_program(Some(self.program));
        gl.viewport(0, 0, self.width, self.height);

        self.main.bind(&gl, main, stride_0, stride_1);
        self.main.activate(&gl);

        if let Some(aux) = aux {
            let aux_texture = self.aux.as_ref().unwrap();

            aux_texture.bind(&gl, aux, stride_0, stride_1);
            aux_texture.activate(&gl);
        }

        // Textures are set with stride widths
        // Map appropriately on the texture
        let stride_scale = self.width as f32 / stride_0 as f32;
        self.update_shader_data(stride_scale, regions);

        // For now there are assumptions that the stirde_1 is 1/2 stride_0
        if stride_1 != stride_0 / 2 {
            panic!("Program cannot handle stride mismatch");
        }
        // Each region is 6 verticies (two triangles)
        gl.draw_arrays(glow::TRIANGLES, 0, regions.len() as i32 * 6);
    }
}

pub struct DrawingContext {
    avc_420: AvcShaderProgram,
    avc_444: AvcShaderProgram,
    texture_shader: TextureShaderProgram,
    offscreen_buffer: OffscreenBuffer,
}

impl DrawingContext {
    /// # Safety
    ///
    /// TODO: Safety notes
    pub unsafe fn new(
        gl_ref: Arc<Context>,
        shader_version: &str,
        width: i32,
        height: i32,
        is_v2: bool,
        video_width: i32,
        video_height: i32,
    ) -> crate::Result<Self> {
        let avc_444_shader_type = if is_v2 {
            ShaderType::Avc444v2
        } else {
            ShaderType::Avc444
        };

        let texture_shader =
            TextureShaderProgram::new(gl_ref.clone(), shader_version, width, height, video_width, video_height)?;

        let avc_420 = AvcShaderProgram::new(
            gl_ref.clone(),
            shader_version,
            video_width,
            video_height,
            ShaderType::Avc420,
        )?;
        let avc_444 = AvcShaderProgram::new(
            gl_ref.clone(),
            shader_version,
            video_width,
            video_height,
            avc_444_shader_type,
        )?;
        let offscreen_buffer = OffscreenBuffer::new(gl_ref, video_width, video_height)?;
        Ok(DrawingContext {
            avc_420,
            avc_444,
            texture_shader,
            offscreen_buffer,
        })
    }

    /// # Safety
    ///
    /// TODO: Safety notes
    pub unsafe fn draw(
        &self,
        main: &[u8],
        aux: Option<&[u8]>,
        stride_0: usize,
        stride_1: usize,
        regions: &Vec<Rectangle>,
    ) {
        let program = if aux.is_some() { &self.avc_444 } else { &self.avc_420 };
        // Draw to an offscreen buffer so that we can reutilize it on next frame paint
        self.offscreen_buffer.activate();
        program.draw(main, aux, stride_0, stride_1, regions);

        self.offscreen_buffer.deactivate();
    }

    /// # Safety
    ///
    /// TODO: Safety notes
    pub unsafe fn draw_cached(&self, location: Rectangle) {
        self.texture_shader.set_location(location);
        self.texture_shader.draw_texture(self.offscreen_buffer.texture);
    }

    pub fn info(&self) {}
}

impl Drop for DrawingContext {
    fn drop(&mut self) {}
}
