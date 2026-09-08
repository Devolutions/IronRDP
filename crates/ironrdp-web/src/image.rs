#![allow(clippy::arithmetic_side_effects)]

use anyhow::Context as _;
use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp::session::image::DecodedImage;
use ironrdp_core::WriteBuf;

/// Copies the dirty `region` into `buffer` from its current cursor (clear it between regions).
/// The returned rect may be wider than `region`: the whole-rows path widens to full image width.
pub(crate) fn extract_partial_image(
    image: &DecodedImage,
    region: InclusiveRectangle,
    buffer: &mut WriteBuf,
) -> anyhow::Result<InclusiveRectangle> {
    if region.left > region.right
        || region.top > region.bottom
        || region.right >= image.width()
        || region.bottom >= image.height()
    {
        anyhow::bail!(
            "invalid region {region:?} for image with dimensions {}x{}",
            image.width(),
            image.height()
        );
    }

    // PERF: needs actual benchmark to find a better heuristic
    if region.height() > 64 || region.width() > 512 {
        extract_whole_rows(image, region, buffer)
    } else {
        extract_smallest_rectangle(image, region, buffer)
    }
}

// Faster for low-height and smaller images
fn extract_smallest_rectangle(
    image: &DecodedImage,
    region: InclusiveRectangle,
    buffer: &mut WriteBuf,
) -> anyhow::Result<InclusiveRectangle> {
    let pixel_size = usize::from(image.pixel_format().bytes_per_pixel());

    let image_width = usize::from(image.width());
    let image_stride = image_width * pixel_size;

    let region_top = usize::from(region.top);
    let region_left = usize::from(region.left);
    let region_width = usize::from(region.width());
    let region_height = usize::from(region.height());
    let region_stride = region_width * pixel_size;

    let dst_buf_size = region_width * region_height * pixel_size;

    let src = image.data();

    let dst = buffer.unfilled_to(dst_buf_size);

    for row in 0..region_height {
        let src_begin = image_stride * (region_top + row) + region_left * pixel_size;
        let src_end = src_begin + region_stride;
        let src_slice = src.get(src_begin..src_end).with_context(|| {
            format!(
                "invalid region {region:?} for image with dimensions {}x{}",
                image.width(),
                image.height()
            )
        })?;

        let target_begin = region_stride * row;
        let target_end = target_begin + region_stride;
        let target_slice = &mut dst[target_begin..target_end];

        target_slice.copy_from_slice(src_slice);
    }

    buffer.advance(dst_buf_size);

    Ok(region)
}

// Faster for high-height and bigger images
fn extract_whole_rows(
    image: &DecodedImage,
    region: InclusiveRectangle,
    buffer: &mut WriteBuf,
) -> anyhow::Result<InclusiveRectangle> {
    let pixel_size = usize::from(image.pixel_format().bytes_per_pixel());

    let image_width = usize::from(image.width());
    let image_stride = image_width * pixel_size;

    let region_top = usize::from(region.top);
    let region_bottom = usize::from(region.bottom);

    let src = image.data();

    let src_begin = region_top * image_stride;
    let src_end = (region_bottom + 1) * image_stride;
    let len = src_end - src_begin;
    let src_slice = src.get(src_begin..src_end).with_context(|| {
        format!(
            "invalid region {region:?} for image with dimensions {}x{}",
            image.width(),
            image.height()
        )
    })?;

    buffer.unfilled_to(len).copy_from_slice(src_slice);
    buffer.advance(len);

    Ok(InclusiveRectangle {
        left: 0,
        top: region.top,
        right: image.width() - 1,
        bottom: region.bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp::graphics::image_processing::PixelFormat;

    fn region(left: u16, top: u16, right: u16, bottom: u16) -> InclusiveRectangle {
        InclusiveRectangle {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn extracts_small_rectangle_into_reused_buffer() {
        let image = DecodedImage::new(PixelFormat::RgbA32, 4, 3);
        let region = region(1, 1, 2, 2);
        let mut buffer = WriteBuf::new();
        buffer.write_slice(&[0xaa, 0xbb]);

        assert_eq!(
            extract_partial_image(&image, region.clone(), &mut buffer).unwrap(),
            region
        );

        let mut expected = vec![0xaa, 0xbb];
        expected.extend_from_slice(&image.data()[20..28]);
        expected.extend_from_slice(&image.data()[36..44]);
        assert_eq!(buffer.filled(), expected);
    }

    #[test]
    fn extracts_whole_rows_into_reused_buffer() {
        let image = DecodedImage::new(PixelFormat::RgbA32, 4, 65);
        let region = region(1, 0, 2, 64);
        let mut buffer = WriteBuf::new();
        buffer.write_slice(&[0xaa, 0xbb]);

        assert_eq!(
            extract_partial_image(&image, region, &mut buffer).unwrap(),
            InclusiveRectangle {
                left: 0,
                top: 0,
                right: 3,
                bottom: 64,
            }
        );

        let mut expected = vec![0xaa, 0xbb];
        expected.extend_from_slice(image.data());
        assert_eq!(buffer.filled(), expected);
    }

    #[test]
    fn rejects_invalid_regions_without_advancing_buffer() {
        let image = DecodedImage::new(PixelFormat::RgbA32, 4, 65);
        let mut buffer = WriteBuf::new();
        buffer.write_slice(&[0xaa, 0xbb]);

        for region in [
            region(3, 0, 2, 0),
            region(0, 2, 0, 1),
            region(0, 64, 0, 65),
            region(0, 0, 4, 64),
        ] {
            assert!(extract_partial_image(&image, region, &mut buffer).is_err());
            assert_eq!(buffer.filled(), [0xaa, 0xbb]);
        }

        let empty_image = DecodedImage::new(PixelFormat::RgbA32, 0, 0);
        assert!(extract_partial_image(&empty_image, region(0, 0, 0, 0), &mut buffer).is_err());
        assert_eq!(buffer.filled(), [0xaa, 0xbb]);
    }
}
