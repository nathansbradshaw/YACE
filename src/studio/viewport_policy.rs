//! Browser viewport dimensions are CSS pixels, independent of device identity.
pub fn cramped(width: f64, height: f64) -> bool {
    width.is_finite()
        && height.is_finite()
        && width > 0.
        && height > 0.
        && (width < 1024. || height < 600.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_dimension_below_the_desktop_boundary_needs_more_space() {
        assert!(!cramped(1024., 600.));
        assert!(cramped(1023., 600.));
        assert!(cramped(1024., 599.));
        assert!(cramped(800., 500.));
        assert!(!cramped(1280., 720.));
        assert!(!cramped(1920., 1080.));
    }

    #[test]
    fn unavailable_or_invalid_dimensions_do_not_block_the_editor() {
        for (width, height) in [
            (0., 600.),
            (1024., 0.),
            (f64::NAN, 600.),
            (1024., f64::INFINITY),
        ] {
            assert!(!cramped(width, height));
        }
    }
}
