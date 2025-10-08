#![allow(clippy::arithmetic_side_effects)]

use anyhow::Context as _;
use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp::session::image::DecodedImage;

pub(crate) fn extract_partial_image(
    image: &DecodedImage,
    region: InclusiveRectangle,
) -> anyhow::Result<(InclusiveRectangle, Vec<u8>)> {
    // PERF: needs actual benchmark to find a better heuristic
    if region.height() > 64 || region.width() > 512 {
        extract_whole_rows(image, region)
    } else {
        extract_smallest_rectangle(image, region)
    }
}

// Faster for low-height and smaller images
fn extract_smallest_rectangle(
    image: &DecodedImage,
    region: InclusiveRectangle,
) -> anyhow::Result<(InclusiveRectangle, Vec<u8>)> {
    let pixel_size = usize::from(image.pixel_format().bytes_per_pixel());

    let image_width = usize::from(image.width());
    let image_stride = image_width * pixel_size;

    let region_top = usize::from(region.top);
    let region_left = usize::from(region.left);
    let region_width = usize::from(region.width());
    let region_height = usize::from(region.height());
    let region_stride = region_width * pixel_size;

    let dst_buf_size = region_width * region_height * pixel_size;
    let mut dst = vec![0; dst_buf_size];

    let src = image.data();

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
        let target_slice = dst
            .get_mut(target_begin..target_end)
            .expect("slice index cannot be out of bounds");

        target_slice.copy_from_slice(src_slice);
    }

    Ok((region, dst))
}

// Faster for high-height and bigger images
fn extract_whole_rows(
    image: &DecodedImage,
    region: InclusiveRectangle,
) -> anyhow::Result<(InclusiveRectangle, Vec<u8>)> {
    let pixel_size = usize::from(image.pixel_format().bytes_per_pixel());

    let image_width = usize::from(image.width());
    let image_stride = image_width * pixel_size;

    let region_top = usize::from(region.top);
    let region_bottom = usize::from(region.bottom);

    let src = image.data();

    let src_begin = region_top * image_stride;
    let src_end = (region_bottom + 1) * image_stride;

    let dst = src
        .get(src_begin..src_end)
        .with_context(|| {
            format!(
                "invalid region {region:?} for image with dimensions {}x{}",
                image.width(),
                image.height()
            )
        })?
        .to_vec();

    let wider_region = InclusiveRectangle {
        left: 0,
        top: region.top,
        right: image.width() - 1,
        bottom: region.bottom,
    };

    Ok((wider_region, dst))
}
