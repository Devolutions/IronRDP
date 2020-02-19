use lazy_static::lazy_static;

use super::*;

lazy_static! {
    static ref REGION_FOR_RECTANGLES_INTERSECTION: Region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 11,
            bottom: 9,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 1,
                right: 5,
                bottom: 3,
            },
            Rectangle {
                left: 7,
                top: 1,
                right: 8,
                bottom: 3,
            },
            Rectangle {
                left: 9,
                top: 1,
                right: 11,
                bottom: 3,
            },
            Rectangle {
                left: 7,
                top: 3,
                right: 11,
                bottom: 4,
            },
            Rectangle {
                left: 3,
                top: 4,
                right: 6,
                bottom: 6,
            },
            Rectangle {
                left: 7,
                top: 4,
                right: 11,
                bottom: 6,
            },
            Rectangle {
                left: 1,
                top: 6,
                right: 3,
                bottom: 8,
            },
            Rectangle {
                left: 4,
                top: 6,
                right: 5,
                bottom: 8,
            },
            Rectangle {
                left: 6,
                top: 6,
                right: 10,
                bottom: 8,
            },
            Rectangle {
                left: 4,
                top: 8,
                right: 5,
                bottom: 9,
            },
            Rectangle {
                left: 6,
                top: 8,
                right: 10,
                bottom: 9,
            },
        ],
    };
}

#[test]
fn union_rectangle_sets_extents_and_single_rectangle_for_empty_region() {
    let mut region = Region::new();

    let input_rectangle = Rectangle {
        left: 5,
        top: 1,
        right: 9,
        bottom: 2,
    };

    let expected_region = Region {
        extents: input_rectangle.clone(),
        rectangles: vec![input_rectangle.clone()],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_places_new_rectangle_higher_relative_to_band() {
    let existing_band_rectangle = Rectangle {
        left: 2,
        top: 3,
        right: 7,
        bottom: 7,
    };
    let mut region = Region {
        extents: existing_band_rectangle.clone(),
        rectangles: vec![existing_band_rectangle.clone()],
    };

    let input_rectangle = Rectangle {
        left: 5,
        top: 1,
        right: 9,
        bottom: 2,
    };

    let expected_region = Region {
        extents: Rectangle {
            left: 2,
            top: 1,
            right: 9,
            bottom: 7,
        },
        rectangles: vec![input_rectangle.clone(), existing_band_rectangle],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_places_new_rectangle_lower_relative_to_band() {
    let existing_band_rectangle = Rectangle {
        left: 2,
        top: 3,
        right: 7,
        bottom: 7,
    };
    let mut region = Region {
        extents: existing_band_rectangle.clone(),
        rectangles: vec![existing_band_rectangle.clone()],
    };

    let input_rectangle = Rectangle {
        left: 1,
        top: 8,
        right: 4,
        bottom: 10,
    };

    let expected_region = Region {
        extents: Rectangle {
            left: 1,
            top: 3,
            right: 7,
            bottom: 10,
        },
        rectangles: vec![existing_band_rectangle, input_rectangle.clone()],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_does_not_add_new_rectangle_which_is_inside_a_band() {
    let existing_band_rectangle = Rectangle {
        left: 2,
        top: 3,
        right: 7,
        bottom: 7,
    };
    let mut region = Region {
        extents: existing_band_rectangle.clone(),
        rectangles: vec![existing_band_rectangle.clone()],
    };

    let input_rectangle = Rectangle {
        left: 5,
        top: 4,
        right: 6,
        bottom: 5,
    };

    let expected_region = Region {
        extents: existing_band_rectangle.clone(),
        rectangles: vec![existing_band_rectangle],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_cuts_new_rectangle_top_part_which_crosses_band_on_top() {
    let existing_band_rectangle = Rectangle {
        left: 2,
        top: 3,
        right: 7,
        bottom: 7,
    };
    let mut region = Region {
        extents: existing_band_rectangle.clone(),
        rectangles: vec![existing_band_rectangle],
    };

    let input_rectangle = Rectangle {
        left: 1,
        top: 2,
        right: 4,
        bottom: 4,
    };

    let expected_region = Region {
        extents: Rectangle {
            left: 1,
            top: 2,
            right: 7,
            bottom: 7,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 2,
                right: 4,
                bottom: 3,
            },
            Rectangle {
                left: 1,
                top: 3,
                right: 7,
                bottom: 4,
            },
            Rectangle {
                left: 2,
                top: 4,
                right: 7,
                bottom: 7,
            },
        ],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_cuts_new_rectangle_lower_part_which_crosses_band_on_bottom() {
    let existing_band_rectangle = Rectangle {
        left: 2,
        top: 3,
        right: 7,
        bottom: 7,
    };
    let mut region = Region {
        extents: existing_band_rectangle.clone(),
        rectangles: vec![existing_band_rectangle],
    };

    let input_rectangle = Rectangle {
        left: 5,
        top: 6,
        right: 9,
        bottom: 8,
    };

    let expected_region = Region {
        extents: Rectangle {
            left: 2,
            top: 3,
            right: 9,
            bottom: 8,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 6,
            },
            Rectangle {
                left: 2,
                top: 6,
                right: 9,
                bottom: 7,
            },
            Rectangle {
                left: 5,
                top: 7,
                right: 9,
                bottom: 8,
            },
        ],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_cuts_new_rectangle_higher_and_lower_part_which_crosses_band_on_top_and_bottom() {
    let existing_band_rectangle = Rectangle {
        left: 2,
        top: 3,
        right: 7,
        bottom: 7,
    };
    let mut region = Region {
        extents: existing_band_rectangle.clone(),
        rectangles: vec![existing_band_rectangle],
    };

    let input_rectangle = Rectangle {
        left: 3,
        top: 1,
        right: 5,
        bottom: 11,
    };

    let expected_region = Region {
        extents: Rectangle {
            left: 2,
            top: 1,
            right: 7,
            bottom: 11,
        },
        rectangles: vec![
            Rectangle {
                left: 3,
                top: 1,
                right: 5,
                bottom: 3,
            },
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 3,
                top: 7,
                right: 5,
                bottom: 11,
            },
        ],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_inserts_new_rectangle_in_band_of_3_rectangles_without_merging_with_rectangles() {
    let mut region = Region {
        extents: Rectangle {
            left: 2,
            top: 3,
            right: 15,
            bottom: 7,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 8,
                top: 3,
                right: 9,
                bottom: 7,
            },
            Rectangle {
                left: 12,
                top: 3,
                right: 15,
                bottom: 7,
            },
        ],
    };

    let input_rectangle = Rectangle {
        left: 10,
        top: 3,
        right: 11,
        bottom: 7,
    };
    let expected_region = Region {
        extents: Rectangle {
            left: 2,
            top: 3,
            right: 15,
            bottom: 7,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 8,
                top: 3,
                right: 9,
                bottom: 7,
            },
            Rectangle {
                left: 10,
                top: 3,
                right: 11,
                bottom: 7,
            },
            Rectangle {
                left: 12,
                top: 3,
                right: 15,
                bottom: 7,
            },
        ],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_inserts_new_rectangle_in_band_of_3_rectangles_with_merging_with_side_rectangles()
{
    let mut region = Region {
        extents: Rectangle {
            left: 2,
            top: 3,
            right: 15,
            bottom: 7,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 8,
                top: 3,
                right: 10,
                bottom: 7,
            },
            Rectangle {
                left: 13,
                top: 3,
                right: 15,
                bottom: 7,
            },
        ],
    };

    let input_rectangle = Rectangle {
        left: 9,
        top: 3,
        right: 14,
        bottom: 7,
    };
    let expected_region = Region {
        extents: Rectangle {
            left: 2,
            top: 3,
            right: 15,
            bottom: 7,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 8,
                top: 3,
                right: 15,
                bottom: 7,
            },
        ],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_inserts_new_rectangle_in_band_of_3_rectangles_with_merging_with_side_rectangles_on_board(
) {
    let mut region = Region {
        extents: Rectangle {
            left: 2,
            top: 3,
            right: 15,
            bottom: 7,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 8,
                top: 3,
                right: 10,
                bottom: 7,
            },
            Rectangle {
                left: 13,
                top: 3,
                right: 15,
                bottom: 7,
            },
        ],
    };

    let input_rectangle = Rectangle {
        left: 10,
        top: 3,
        right: 13,
        bottom: 7,
    };
    let expected_region = Region {
        extents: Rectangle {
            left: 2,
            top: 3,
            right: 15,
            bottom: 7,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 8,
                top: 3,
                right: 13,
                bottom: 7,
            },
            Rectangle {
                left: 13,
                top: 3,
                right: 15,
                bottom: 7,
            },
        ],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn union_rectangle_inserts_new_rectangle_between_two_bands() {
    let mut region = Region {
        extents: Rectangle {
            left: 1,
            top: 3,
            right: 7,
            bottom: 10,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 1,
                top: 8,
                right: 4,
                bottom: 10,
            },
        ],
    };

    let input_rectangle = Rectangle {
        left: 3,
        top: 4,
        right: 4,
        bottom: 9,
    };
    let expected_region = Region {
        extents: Rectangle {
            left: 1,
            top: 3,
            right: 7,
            bottom: 10,
        },
        rectangles: vec![
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 3,
                top: 7,
                right: 4,
                bottom: 8,
            },
            Rectangle {
                left: 1,
                top: 8,
                right: 4,
                bottom: 10,
            },
        ],
    };

    region.union_rectangle(input_rectangle);
    assert_eq!(expected_region, region);
}

#[test]
fn simplify_does_not_change_two_different_bands_with_multiple_rectangles() {
    let mut region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 7,
            bottom: 3,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 1,
                right: 2,
                bottom: 2,
            },
            Rectangle {
                left: 3,
                top: 1,
                right: 4,
                bottom: 2,
            },
            Rectangle {
                left: 5,
                top: 1,
                right: 6,
                bottom: 2,
            },
            Rectangle {
                left: 1,
                top: 2,
                right: 2,
                bottom: 3,
            },
            Rectangle {
                left: 3,
                top: 2,
                right: 4,
                bottom: 3,
            },
            Rectangle {
                left: 5,
                top: 2,
                right: 7,
                bottom: 3,
            },
        ],
    };
    let expected_region = region.clone();

    region.simplify();
    assert_eq!(expected_region, region);
}

#[test]
fn simplify_does_not_change_two_different_bands_with_one_rectangle() {
    let mut region = Region {
        extents: Rectangle {
            left: 2,
            top: 1,
            right: 7,
            bottom: 11,
        },
        rectangles: vec![
            Rectangle {
                left: 3,
                top: 1,
                right: 5,
                bottom: 3,
            },
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
        ],
    };
    let expected_region = region.clone();

    region.simplify();
    assert_eq!(expected_region, region);
}

#[test]
fn simplify_does_not_change_three_different_bands_with_one_rectangle() {
    let mut region = Region {
        extents: Rectangle {
            left: 2,
            top: 1,
            right: 7,
            bottom: 11,
        },
        rectangles: vec![
            Rectangle {
                left: 3,
                top: 1,
                right: 5,
                bottom: 3,
            },
            Rectangle {
                left: 2,
                top: 3,
                right: 7,
                bottom: 7,
            },
            Rectangle {
                left: 3,
                top: 7,
                right: 5,
                bottom: 11,
            },
        ],
    };
    let expected_region = region.clone();

    region.simplify();
    assert_eq!(expected_region, region);
}

#[test]
fn simplify_merges_bands_with_identical_internal_rectangles() {
    let mut region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 7,
            bottom: 3,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 1,
                right: 2,
                bottom: 2,
            },
            Rectangle {
                left: 3,
                top: 1,
                right: 4,
                bottom: 2,
            },
            Rectangle {
                left: 5,
                top: 1,
                right: 6,
                bottom: 2,
            },
            Rectangle {
                left: 1,
                top: 2,
                right: 2,
                bottom: 3,
            },
            Rectangle {
                left: 3,
                top: 2,
                right: 4,
                bottom: 3,
            },
            Rectangle {
                left: 5,
                top: 2,
                right: 6,
                bottom: 3,
            },
        ],
    };
    let expected_region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 7,
            bottom: 3,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 1,
                right: 2,
                bottom: 3,
            },
            Rectangle {
                left: 3,
                top: 1,
                right: 4,
                bottom: 3,
            },
            Rectangle {
                left: 5,
                top: 1,
                right: 6,
                bottom: 3,
            },
        ],
    };

    region.simplify();
    assert_eq!(expected_region, region);
}

#[test]
fn simplify_merges_three_bands_with_identical_internal_rectangles() {
    let mut region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 7,
            bottom: 3,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 1,
                right: 2,
                bottom: 2,
            },
            Rectangle {
                left: 3,
                top: 1,
                right: 4,
                bottom: 2,
            },
            Rectangle {
                left: 5,
                top: 1,
                right: 6,
                bottom: 2,
            },
            Rectangle {
                left: 1,
                top: 2,
                right: 2,
                bottom: 3,
            },
            Rectangle {
                left: 3,
                top: 2,
                right: 4,
                bottom: 3,
            },
            Rectangle {
                left: 5,
                top: 2,
                right: 6,
                bottom: 3,
            },
            Rectangle {
                left: 1,
                top: 3,
                right: 2,
                bottom: 4,
            },
            Rectangle {
                left: 3,
                top: 3,
                right: 4,
                bottom: 4,
            },
            Rectangle {
                left: 5,
                top: 3,
                right: 6,
                bottom: 4,
            },
        ],
    };
    let expected_region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 7,
            bottom: 3,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 1,
                right: 2,
                bottom: 4,
            },
            Rectangle {
                left: 3,
                top: 1,
                right: 4,
                bottom: 4,
            },
            Rectangle {
                left: 5,
                top: 1,
                right: 6,
                bottom: 4,
            },
        ],
    };

    region.simplify();
    assert_eq!(expected_region, region);
}

#[test]
fn simplify_merges_two_pairs_of_bands_with_identical_internal_rectangles() {
    let mut region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 7,
            bottom: 5,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 1,
                right: 2,
                bottom: 2,
            },
            Rectangle {
                left: 3,
                top: 1,
                right: 4,
                bottom: 2,
            },
            Rectangle {
                left: 5,
                top: 1,
                right: 6,
                bottom: 2,
            },
            Rectangle {
                left: 1,
                top: 2,
                right: 2,
                bottom: 3,
            },
            Rectangle {
                left: 3,
                top: 2,
                right: 4,
                bottom: 3,
            },
            Rectangle {
                left: 5,
                top: 2,
                right: 6,
                bottom: 3,
            },
            Rectangle {
                left: 2,
                top: 3,
                right: 3,
                bottom: 4,
            },
            Rectangle {
                left: 4,
                top: 3,
                right: 5,
                bottom: 4,
            },
            Rectangle {
                left: 6,
                top: 3,
                right: 7,
                bottom: 4,
            },
            Rectangle {
                left: 2,
                top: 4,
                right: 3,
                bottom: 5,
            },
            Rectangle {
                left: 4,
                top: 4,
                right: 5,
                bottom: 5,
            },
            Rectangle {
                left: 6,
                top: 4,
                right: 7,
                bottom: 5,
            },
        ],
    };
    let expected_region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 7,
            bottom: 5,
        },
        rectangles: vec![
            Rectangle {
                left: 1,
                top: 1,
                right: 2,
                bottom: 3,
            },
            Rectangle {
                left: 3,
                top: 1,
                right: 4,
                bottom: 3,
            },
            Rectangle {
                left: 5,
                top: 1,
                right: 6,
                bottom: 3,
            },
            Rectangle {
                left: 2,
                top: 3,
                right: 3,
                bottom: 5,
            },
            Rectangle {
                left: 4,
                top: 3,
                right: 5,
                bottom: 5,
            },
            Rectangle {
                left: 6,
                top: 3,
                right: 7,
                bottom: 5,
            },
        ],
    };

    region.simplify();
    assert_eq!(expected_region, region);
}

#[test]
fn intersect_rectangle_returns_empty_region_for_not_intersecting_rectangle() {
    let region = &*REGION_FOR_RECTANGLES_INTERSECTION;
    let expected_region = Region {
        extents: Rectangle {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rectangles: vec![],
    };
    let input_rectangle = Rectangle {
        left: 5,
        top: 2,
        right: 6,
        bottom: 3,
    };

    let actual_region = region.intersect_rectangle(&input_rectangle);
    assert_eq!(expected_region, actual_region);
}

#[test]
fn intersect_rectangle_returns_empty_region_for_empty_intersection_region() {
    let region = Region {
        extents: Rectangle {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rectangles: vec![],
    };
    let expected_region = region.clone();
    let input_rectangle = Rectangle {
        left: 5,
        top: 2,
        right: 6,
        bottom: 3,
    };

    let actual_region = region.intersect_rectangle(&input_rectangle);
    assert_eq!(expected_region, actual_region);
}

#[test]
fn intersect_rectangle_returns_part_of_rectangle_that_overlaps_for_region_with_one_rectangle() {
    let region = Region {
        extents: Rectangle {
            left: 1,
            top: 1,
            right: 5,
            bottom: 3,
        },
        rectangles: vec![Rectangle {
            left: 1,
            top: 1,
            right: 5,
            bottom: 3,
        }],
    };
    let expected_region = Region {
        extents: Rectangle {
            left: 2,
            top: 2,
            right: 3,
            bottom: 3,
        },
        rectangles: vec![Rectangle {
            left: 2,
            top: 2,
            right: 3,
            bottom: 3,
        }],
    };
    let input_rectangle = Rectangle {
        left: 2,
        top: 2,
        right: 3,
        bottom: 3,
    };

    let actual_region = region.intersect_rectangle(&input_rectangle);
    assert_eq!(expected_region, actual_region);
}

#[test]
fn intersect_rectangle_returns_region_with_parts_of_rectangles_that_intersect_input_rectangle() {
    let region = &*REGION_FOR_RECTANGLES_INTERSECTION;
    let expected_region = Region {
        extents: Rectangle {
            left: 3,
            top: 2,
            right: 8,
            bottom: 5,
        },
        rectangles: vec![
            Rectangle {
                left: 3,
                top: 2,
                right: 5,
                bottom: 3,
            },
            Rectangle {
                left: 7,
                top: 2,
                right: 8,
                bottom: 3,
            },
            Rectangle {
                left: 7,
                top: 3,
                right: 8,
                bottom: 4,
            },
            Rectangle {
                left: 3,
                top: 4,
                right: 6,
                bottom: 5,
            },
            Rectangle {
                left: 7,
                top: 4,
                right: 8,
                bottom: 5,
            },
        ],
    };
    let input_rectangle = Rectangle {
        left: 3,
        top: 2,
        right: 8,
        bottom: 5,
    };

    let actual_region = region.intersect_rectangle(&input_rectangle);
    assert_eq!(expected_region, actual_region);
}

#[test]
fn intersect_rectangle_returns_region_with_exact_sizes_of_rectangle_that_overlaps_it() {
    let region = &*REGION_FOR_RECTANGLES_INTERSECTION;
    let expected_region = Region {
        extents: Rectangle {
            left: 4,
            top: 6,
            right: 5,
            bottom: 8,
        },
        rectangles: vec![Rectangle {
            left: 4,
            top: 6,
            right: 5,
            bottom: 8,
        }],
    };
    let input_rectangle = Rectangle {
        left: 4,
        top: 6,
        right: 5,
        bottom: 8,
    };

    let actual_region = region.intersect_rectangle(&input_rectangle);
    assert_eq!(expected_region, actual_region);
}
