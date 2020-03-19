#[cfg(test)]
mod tests;

use std::cmp::{max, min};

use crate::utils::Rectangle;

#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    extents: Rectangle,
    rectangles: Vec<Rectangle>,
}

impl Region {
    pub fn new() -> Self {
        Self {
            extents: Rectangle::empty(),
            rectangles: Vec::new(),
        }
    }

    pub fn rectangles(&self) -> &[Rectangle] {
        self.rectangles.as_slice()
    }

    pub fn extents(&self) -> &Rectangle {
        &self.extents
    }

    pub fn union_rectangle(&mut self, rectangle: Rectangle) {
        if self.rectangles.is_empty() {
            *self = Self::from(rectangle);
        } else {
            let mut dst = Vec::with_capacity(self.rectangles.len() + 1);

            handle_rectangle_higher_relative_to_extents(&rectangle, &self.extents, &mut dst);

            // treat possibly overlapping region
            let bands = split_bands(self.rectangles.as_slice());
            let mut bands = bands.as_slice();
            while let Some((band, bands_new)) = bands.split_first() {
                bands = bands_new;

                let top_inter_band = if band[0].bottom <= rectangle.top
                    || rectangle.bottom <= band[0].top
                    || rectangle_in_band(band, &rectangle)
                {
                    // `rectangle` is lower, higher, or in the current band
                    dst.extend_from_slice(band);

                    rectangle.top
                } else {
                    handle_rectangle_that_overlaps_band(&rectangle, band, &mut dst);

                    band[0].bottom
                };

                if !bands.is_empty() {
                    let next_band = bands[0];
                    handle_rectangle_between_bands(
                        &rectangle,
                        band,
                        next_band,
                        &mut dst,
                        top_inter_band,
                    );
                }
            }

            handle_rectangle_lower_relative_to_extents(&rectangle, &self.extents, &mut dst);

            self.rectangles = dst;
            self.extents = self.extents.union(&rectangle);

            self.simplify();
        }
    }

    pub fn intersect_rectangle(&self, rectangle: &Rectangle) -> Self {
        match self.rectangles.len() {
            0 => Self::new(),
            1 => self
                .extents
                .intersect(&rectangle)
                .map(Self::from)
                .unwrap_or_default(),
            _ => {
                let rectangles = self
                    .rectangles
                    .iter()
                    .take_while(|r| r.top < rectangle.bottom)
                    .map(|r| r.intersect(&rectangle))
                    .filter_map(|v| v)
                    .collect::<Vec<_>>();
                let extents = Rectangle::union_all(rectangles.as_slice());

                let mut region = Self {
                    rectangles,
                    extents,
                };
                region.simplify();

                region
            }
        }
    }

    fn simplify(&mut self) {
        /* Simplify consecutive bands that touch and have the same items
         *
         *  ====================          ====================
         *     | 1 |  | 2   |               |   |  |     |
         *  ====================            |   |  |     |
         *     | 1 |  | 2   |	   ====>    | 1 |  |  2  |
         *  ====================            |   |  |     |
         *     | 1 |  | 2   |               |   |  |     |
         *  ====================          ====================
         *
         */

        if self.rectangles.len() < 2 {
            return;
        }

        let mut current_band_start = 0;
        while current_band_start < self.rectangles.len()
            && current_band_start + get_current_band(&self.rectangles[current_band_start..]).len()
                < self.rectangles.len()
        {
            let current_band = get_current_band(&self.rectangles[current_band_start..]);
            let next_band =
                get_current_band(&self.rectangles[current_band_start + current_band.len()..]);

            if current_band[0].bottom == next_band[0].top
                && bands_internals_equal(current_band, next_band)
            {
                let first_band_len = current_band.len();
                let second_band_len = next_band.len();
                let second_band_bottom = next_band[0].bottom;
                self.rectangles.drain(
                    current_band_start + first_band_len
                        ..current_band_start + first_band_len + second_band_len,
                );
                self.rectangles
                    .iter_mut()
                    .skip(current_band_start)
                    .take(first_band_len)
                    .for_each(|r| r.bottom = second_band_bottom);
            } else {
                current_band_start += current_band.len();
            }
        }
    }
}

impl Default for Region {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Rectangle> for Region {
    fn from(r: Rectangle) -> Self {
        Self {
            extents: r.clone(),
            rectangles: vec![r],
        }
    }
}

fn handle_rectangle_higher_relative_to_extents(
    rectangle: &Rectangle,
    extents: &Rectangle,
    dst: &mut Vec<Rectangle>,
) {
    if rectangle.top < extents.top {
        dst.push(Rectangle {
            top: rectangle.top,
            bottom: min(extents.top, rectangle.bottom),
            left: rectangle.left,
            right: rectangle.right,
        });
    }
}

fn handle_rectangle_lower_relative_to_extents(
    rectangle: &Rectangle,
    extents: &Rectangle,
    dst: &mut Vec<Rectangle>,
) {
    if extents.bottom < rectangle.bottom {
        dst.push(Rectangle {
            top: max(extents.bottom, rectangle.top),
            bottom: rectangle.bottom,
            left: rectangle.left,
            right: rectangle.right,
        });
    }
}

fn handle_rectangle_that_overlaps_band(
    rectangle: &Rectangle,
    band: &[Rectangle],
    dst: &mut Vec<Rectangle>,
) {
    /* rect overlaps the band:
                         |    |  |    |
       ====^=================|    |==|    |=========================== band
       |   top split     |    |  |    |
       v                 | 1  |  | 2  |
       ^                 |    |  |    |  +----+   +----+
       |   merge zone    |    |  |    |  |    |   | 4  |
       v                 +----+  |    |  |    |   +----+
       ^                         |    |  | 3  |
       |   bottom split          |    |  |    |
       ====v=========================|    |==|    |===================
                                 |    |  |    |

        possible cases:
        1) no top split, merge zone then a bottom split. The band will be splitted
           in two
        2) not band split, only the merge zone, band merged with rect but not splitted
        3) a top split, the merge zone and no bottom split. The band will be split
           in two
        4) a top split, the merge zone and also a bottom split. The band will be
           splitted in 3, but the coalesce algorithm may merge the created bands
    */

    let band_top = band[0].top;
    let band_bottom = band[0].bottom;

    if band_top < rectangle.top {
        // split current band by the current band top and `rectangle.top` (case 3, 4)
        copy_band(band, dst, band_top, rectangle.top);
    }

    // split the merge zone (all cases)
    copy_band_with_union(
        band,
        dst,
        max(rectangle.top, band_top),
        min(rectangle.bottom, band_bottom),
        &rectangle,
    );

    // split current band by the `rectangle.bottom` and the current band bottom (case 1, 4)
    if rectangle.bottom < band_bottom {
        copy_band(band, dst, rectangle.bottom, band_bottom);
    }
}

fn handle_rectangle_between_bands(
    rectangle: &Rectangle,
    band: &[Rectangle],
    next_band: &[Rectangle],
    dst: &mut Vec<Rectangle>,
    top_inter_band: u16,
) {
    /* test if a piece of rect should be inserted as a new band between
     * the current band and the next one. band n and n+1 shouldn't touch.
     *
     * ==============================================================
     *                                                        band n
     *            +------+                    +------+
     * ===========| rect |====================|      |===============
     *            |      |    +------+        |      |
     *            +------+    | rect |        | rect |
     *                        +------+        |      |
     * =======================================|      |================
     *                                        +------+         band n+1
     * ===============================================================
     *
     */

    let band_bottom = band[0].bottom;

    let next_band_top = next_band[0].top;
    if next_band_top != band_bottom
        && band_bottom < rectangle.bottom
        && rectangle.top < next_band_top
    {
        dst.push(Rectangle {
            top: top_inter_band,
            bottom: min(next_band_top, rectangle.bottom),
            left: rectangle.left,
            right: rectangle.right,
        });
    }
}

fn rectangle_in_band(band: &[Rectangle], rectangle: &Rectangle) -> bool {
    // part of `rectangle` is higher or lower
    if rectangle.top < band[0].top || band[0].bottom < rectangle.bottom {
        return false;
    }

    for source_rectangle in band {
        if source_rectangle.left <= rectangle.left {
            if rectangle.right <= source_rectangle.right {
                return true;
            }
        } else {
            // as the band is sorted from left to right,
            // once we've seen an item that is after `rectangle.left`
            // we are sure that the result is false
            return false;
        }
    }

    false
}

fn copy_band_with_union(
    mut band: &[Rectangle],
    dst: &mut Vec<Rectangle>,
    band_top: u16,
    band_bottom: u16,
    union_rectangle: &Rectangle,
) {
    /* merges a band with the given rect
     * Input:
     *                   unionRect
     *               |                |
     *               |                |
     * ==============+===============+================================
     *   |Item1|  |Item2| |Item3|  |Item4|    |Item5|            Band
     * ==============+===============+================================
     *    before     |    overlap     |          after
     *
     * Resulting band:
     *   +-----+  +----------------------+    +-----+
     *   |Item1|  |      Item2           |    |Item3|
     *   +-----+  +----------------------+    +-----+
     *
     *  We first copy as-is items that are before Item2, the first overlapping
     *  item.
     *  Then we find the last one that overlap unionRect to agregate Item2, Item3
     *  and Item4 to create Item2.
     *  Finally Item5 is copied as Item3.
     *
     *  When no unionRect is provided, we skip the two first steps to just copy items
     */

    let items_before_union_rectangle = band
        .iter()
        .map(|r| Rectangle {
            top: band_top,
            bottom: band_bottom,
            left: r.left,
            right: r.right,
        })
        .take_while(|r| r.right < union_rectangle.left);
    let items_before_union_rectangle_len = items_before_union_rectangle
        .clone()
        .map(|_| 1)
        .sum::<usize>();
    dst.extend(items_before_union_rectangle);
    band = &band[items_before_union_rectangle_len..];

    // treat items overlapping with `union_rectangle`
    let left = min(
        band.first().map(|r| r.left).unwrap_or(union_rectangle.left),
        union_rectangle.left,
    );
    let mut right = union_rectangle.right;
    while !band.is_empty() {
        if band[0].right >= union_rectangle.right {
            if band[0].left < union_rectangle.right {
                right = band[0].right;
                band = &band[1..];
            }
            break;
        }
        band = &band[1..];
    }
    dst.push(Rectangle {
        top: band_top,
        bottom: band_bottom,
        left,
        right,
    });

    // treat remaining items on the same band
    copy_band(band, dst, band_top, band_bottom);
}

fn copy_band(band: &[Rectangle], dst: &mut Vec<Rectangle>, band_top: u16, band_bottom: u16) {
    dst.extend(band.iter().map(|r| Rectangle {
        top: band_top,
        bottom: band_bottom,
        left: r.left,
        right: r.right,
    }));
}

fn split_bands(mut rectangles: &[Rectangle]) -> Vec<&[Rectangle]> {
    let mut bands = Vec::new();
    while !rectangles.is_empty() {
        let band = get_current_band(rectangles);
        rectangles = &rectangles[band.len()..];
        bands.push(band);
    }

    bands
}

fn get_current_band(rectangles: &[Rectangle]) -> &[Rectangle] {
    let band_top = rectangles[0].top;

    for i in 1..rectangles.len() {
        if rectangles[i].top != band_top {
            return &rectangles[..i];
        }
    }

    rectangles
}

fn bands_internals_equal(first_band: &[Rectangle], second_band: &[Rectangle]) -> bool {
    if first_band.len() != second_band.len() {
        return false;
    }

    for (first_band_rect, second_band_rect) in first_band.iter().zip(second_band.iter()) {
        if first_band_rect.left != second_band_rect.left
            || first_band_rect.right != second_band_rect.right
        {
            return false;
        }
    }

    true
}
