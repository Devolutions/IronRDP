use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use glow::Context;
use ironrdp::pdu::dvc::gfx::{
    Avc420BitmapStream, Avc444BitmapStream, Codec1Type, CreateSurfacePdu, Encoding, GraphicsPipelineError, PixelFormat,
    WireToSurface1Pdu,
};
use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp::pdu::PduBufferParsing;
#[cfg(feature = "openh264")]
use openh264::decoder::Decoder;
#[cfg(feature = "openh264")]
use openh264::formats::YUVSource;
#[cfg(feature = "openh264")]
use openh264::OpenH264API;

use crate::draw::DrawingContext;
use crate::renderer::RendererError;

type Result<T> = std::result::Result<T, RendererError>;

#[derive(Clone)]
struct DataRegion {
    data: Vec<u8>,
    regions: Vec<InclusiveRectangle>,
}

impl Debug for DataRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataRegion")
            .field("data_len", &self.data.len())
            .field("regions", &self.regions)
            .finish()
    }
}

#[cfg(feature = "openh264")]
pub struct SurfaceDecoders {
    library_path: std::path::PathBuf,
    decoders: HashMap<u16, Decoder>,
}

#[cfg(feature = "openh264")]
impl SurfaceDecoders {
    pub fn new(library_path: std::path::PathBuf) -> Self {
        SurfaceDecoders {
            library_path,
            decoders: HashMap::new(),
        }
    }

    pub fn add(&mut self, id: u16) -> Result<()> {
        let api = OpenH264API::from_blob_path(&self.library_path)?;
        let decoder = Decoder::with_api_config(api, Default::default())?;
        self.decoders.insert(id, decoder);
        Ok(())
    }

    pub fn remove(&mut self, id: u16) -> Result<()> {
        self.decoders.remove(&id);
        Ok(())
    }

    pub fn decode_wire_to_surface_1_pdu(&mut self, pdu: &WireToSurface1Pdu) -> Result<DataBuffer> {
        let decoder = self
            .decoders
            .get_mut(&pdu.surface_id)
            .ok_or(RendererError::InvalidSurfaceId(pdu.surface_id))?;
        match pdu.codec_id {
            ironrdp::pdu::dvc::gfx::Codec1Type::Avc420 => {
                let packet = Avc420BitmapStream::from_buffer_consume(&mut pdu.bitmap_data.as_slice())
                    .map_err(GraphicsPipelineError::from)?;
                let yuv = decoder.decode(packet.data)?.ok_or(RendererError::DecodeError)?;
                let dimensions = yuv.dimensions();
                let strides = yuv.strides();
                let regions = packet.rectangles;
                let data = convert_yuv_to_buffer(&yuv);
                let data1 = DataRegion { data, regions };
                Ok(DataBuffer {
                    main: Some(data1),
                    aux: None,
                    stride0: strides.0,
                    stride1: strides.1,
                    operation: Encoding::LUMA,
                    codec: pdu.codec_id,
                    dimensions,
                })
            }
            ironrdp::pdu::dvc::gfx::Codec1Type::Avc444 | ironrdp::pdu::dvc::gfx::Codec1Type::Avc444v2 => {
                let packet = Avc444BitmapStream::from_buffer_consume(&mut pdu.bitmap_data.as_slice())
                    .map_err(GraphicsPipelineError::from)?;
                let yuv = decoder.decode(packet.stream1.data)?.ok_or(RendererError::DecodeError)?;
                let dimensions = yuv.dimensions();
                let strides = yuv.strides();
                let regions = packet.stream1.rectangles;
                let data = convert_yuv_to_buffer(&yuv);
                let data1 = DataRegion { data, regions };

                let data2 = if packet.encoding == Encoding::LUMA_AND_CHROMA {
                    let aux = packet.stream2.unwrap();
                    let yuv = decoder.decode(aux.data)?.ok_or(RendererError::DecodeError)?;
                    let data = convert_yuv_to_buffer(&yuv);
                    let regions = aux.rectangles;
                    Some(DataRegion { data, regions })
                } else {
                    None
                };
                let data_buffer = match packet.encoding {
                    Encoding::LUMA_AND_CHROMA => DataBuffer {
                        main: Some(data1),
                        aux: data2,
                        stride0: strides.0,
                        stride1: strides.1,
                        operation: packet.encoding,
                        codec: pdu.codec_id,
                        dimensions,
                    },
                    Encoding::LUMA => DataBuffer {
                        main: Some(data1),
                        aux: None,
                        stride0: strides.0,
                        stride1: strides.1,
                        operation: packet.encoding,
                        codec: pdu.codec_id,
                        dimensions,
                    },
                    Encoding::CHROMA => DataBuffer {
                        main: None,
                        aux: Some(data1),
                        stride0: strides.0,
                        stride1: strides.1,
                        operation: packet.encoding,
                        codec: pdu.codec_id,
                        dimensions,
                    },
                    _ => unreachable!("Unknown encoding type"),
                };
                Ok(data_buffer)
            }
            _ => Err(RendererError::UnsupportedCodec(pdu.codec_id)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataBuffer {
    operation: Encoding,
    main: Option<DataRegion>,
    aux: Option<DataRegion>,
    stride0: usize,
    stride1: usize,
    codec: Codec1Type,
    dimensions: (usize, usize),
}

pub struct Surface {
    id: u16,
    _pixel_format: PixelFormat,
    context: Option<DrawingContext>,
    location: Option<InclusiveRectangle>,
    data_cache: Option<DataRegion>,
    shader_version: String,
    gl: Arc<Context>,
    width: u16,
    height: u16,
}

impl Surface {
    pub fn new(
        id: u16,
        pixel_format: PixelFormat,
        gl: Arc<Context>,
        shader_version: &str,
        width: u16,
        height: u16,
    ) -> Result<Self> {
        Ok(Surface {
            id,
            _pixel_format: pixel_format,
            context: None,
            location: None,
            data_cache: None,
            gl,
            width,
            height,
            shader_version: shader_version.to_string(),
        })
    }

    pub fn set_location(&mut self, location: InclusiveRectangle) {
        self.location = Some(location);
    }

    fn draw_scene(&mut self, data: DataBuffer) -> Result<()> {
        let stride0 = data.stride0;
        let stride1 = data.stride1;
        let (main_data, main_regions) = if let Some(data) = data.main.as_ref() {
            self.data_cache = Some(data.clone());
            (data.data.as_slice(), &data.regions)
        } else {
            let cache = self.data_cache.as_ref().unwrap();
            (cache.data.as_slice(), &cache.regions)
        };
        let (aux_data, regions) = if data.operation == Encoding::CHROMA || data.operation == Encoding::LUMA_AND_CHROMA {
            let aux = data.aux.as_ref().unwrap();
            (Some(aux.data.as_slice()), &aux.regions)
        } else {
            (None, main_regions)
        };
        unsafe {
            let context = if let Some(context) = self.context.as_mut() {
                context
            } else {
                self.context = Some(
                    DrawingContext::new(
                        self.gl.clone(),
                        &self.shader_version,
                        self.width as i32,
                        self.height as i32,
                        data.codec == Codec1Type::Avc444v2,
                        data.dimensions.0 as i32,
                        data.dimensions.1 as i32,
                    )
                    .expect("Initiliazation of context failed"),
                );
                self.context.as_mut().unwrap()
            };
            match data.operation {
                Encoding::LUMA_AND_CHROMA => {
                    context.draw(main_data, aux_data, stride0, stride1, regions);
                }
                Encoding::LUMA => {
                    context.draw(main_data, None, stride0, stride1, regions);
                }
                Encoding::CHROMA => {
                    context.draw(main_data, aux_data, stride0, stride1, regions);
                }
                _ => {
                    error!("Unknown operation type");
                }
            }
        }
        Ok(())
    }

    fn draw_cached(&self) {
        if let Some(context) = self.context.as_ref() {
            let location = if let Some(location) = self.location.as_ref() {
                location.clone()
            } else {
                InclusiveRectangle {
                    left: 0,
                    top: 0,
                    right: self.width - 1,
                    bottom: self.height - 1,
                }
            };

            unsafe {
                context.draw_cached(location);
            }
        }
    }
}

pub struct Surfaces {
    surfaces: HashMap<u16, Surface>,
}

impl Surfaces {
    pub(crate) fn new() -> Self {
        Surfaces {
            surfaces: HashMap::new(),
        }
    }

    fn get_surface(&mut self, id: u16) -> Result<&mut Surface> {
        self.surfaces.get_mut(&id).ok_or(RendererError::InvalidSurfaceId(id))
    }

    pub(crate) fn create_surface(
        &mut self,
        pdu: CreateSurfacePdu,
        gl: Arc<Context>,
        shader_version: &str,
    ) -> Result<()> {
        let surface = Surface::new(
            pdu.surface_id,
            pdu.pixel_format,
            gl,
            shader_version,
            pdu.width,
            pdu.height,
        )?;
        self.surfaces.insert(surface.id, surface);
        Ok(())
    }

    pub(crate) fn delete_surface(&mut self, id: u16) {
        self.surfaces.remove(&id);
    }

    pub(crate) fn draw_scene(&mut self, id: u16, data: DataBuffer) -> Result<()> {
        let surface = self.get_surface(id)?;
        surface.draw_scene(data)
    }

    pub(crate) fn flush_output(&mut self) {
        for (_id, surface) in self.surfaces.iter_mut() {
            surface.draw_cached();
        }
    }

    pub(crate) fn map_surface_to_scaled_output(
        &mut self,
        pdu: ironrdp::pdu::dvc::gfx::MapSurfaceToScaledOutputPdu,
    ) -> Result<()> {
        let surface = self.get_surface(pdu.surface_id)?;
        surface.set_location(InclusiveRectangle {
            left: pdu.output_origin_x as u16,
            top: pdu.output_origin_y as u16,
            right: pdu.target_width as u16,
            bottom: pdu.target_height as u16,
        });
        Ok(())
    }
}

/// Copy YUV planes into a contiguous buffer. OpenH264 documentation says that
/// decoded data must be copied out if not used immediately.
#[cfg(feature = "openh264")]
fn convert_yuv_to_buffer(yuv: &impl YUVSource) -> Vec<u8> {
    let y = yuv.y();
    let u = yuv.u();
    let v = yuv.v();
    let total_len = y.len() + u.len() + v.len();
    let mut data = vec![0; total_len];
    data[..y.len()].copy_from_slice(y);
    data[y.len()..y.len() + u.len()].copy_from_slice(u);
    data[y.len() + u.len()..].copy_from_slice(v);
    data
}
