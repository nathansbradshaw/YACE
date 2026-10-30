//! Archive compatibility for the patched Candle dependency.
use candle_core::{DType, Tensor, pickle::PthTensors};
use std::path::Path;

#[test]
fn npz_array_names_and_values_survive_the_zip_upgrade() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/candle-core/tests/test.npz");
    let arrays = Tensor::read_npz(path).unwrap();
    assert_eq!(
        arrays
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["x", "x_plus_one"]
    );
    assert_eq!(
        arrays[1]
            .1
            .to_dtype(DType::U8)
            .unwrap()
            .to_vec1::<u8>()
            .unwrap(),
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
}

#[test]
fn pickle_arrays_keep_their_layout_and_values() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("vendor/candle-core/tests/fortran_tensor_3d.pth");
    let arrays = PthTensors::new(path, None).unwrap();
    let tensor = arrays.get("tensor_fortran").unwrap().unwrap();
    assert_eq!(tensor.dims3().unwrap(), (2, 3, 4));
    let values: Vec<_> = tensor
        .to_vec3::<i64>()
        .unwrap()
        .into_iter()
        .flatten()
        .flatten()
        .collect();
    assert_eq!(values, (1..=24).collect::<Vec<_>>());
}
