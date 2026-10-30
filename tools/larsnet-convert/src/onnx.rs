//! Minimal ONNX protobuf writer for this one fixed architecture. No Python,
//! arbitrary TorchScript execution, external tensor files or graph imports.
#[derive(Default)]
struct Message(Vec<u8>);
impl Message {
    fn varint(&mut self, mut n: u64) {
        while n >= 128 {
            self.0.push(n as u8 | 128);
            n >>= 7;
        }
        self.0.push(n as u8);
    }
    fn integer(&mut self, tag: u64, n: u64) {
        self.varint(tag << 3);
        self.varint(n);
    }
    fn bytes(&mut self, tag: u64, value: &[u8]) {
        self.varint((tag << 3) | 2);
        self.varint(value.len() as u64);
        self.0.extend_from_slice(value);
    }
    fn text(&mut self, tag: u64, value: &str) {
        self.bytes(tag, value.as_bytes());
    }
}
fn ints(name: &str, values: &[u64]) -> Vec<u8> {
    let mut m = Message::default();
    m.text(1, name);
    m.integer(20, 7);
    for v in values {
        m.integer(8, *v);
    }
    m.0
}
fn integer(name: &str, value: u64) -> Vec<u8> {
    let mut m = Message::default();
    m.text(1, name);
    m.integer(20, 2);
    m.integer(3, value);
    m.0
}
fn float(name: &str, value: f32) -> Vec<u8> {
    let mut m = Message::default();
    m.text(1, name);
    m.integer(20, 1);
    m.varint((2 << 3) | 5);
    m.0.extend_from_slice(&value.to_le_bytes());
    m.0
}
fn value(name: &str) -> Vec<u8> {
    let mut shape = Message::default();
    for dim in [1, 2, 2048, 512] {
        let mut d = Message::default();
        d.integer(1, dim);
        shape.bytes(1, &d.0);
    }
    let mut tensor = Message::default();
    tensor.integer(1, 1);
    tensor.bytes(2, &shape.0);
    let mut typ = Message::default();
    typ.bytes(1, &tensor.0);
    let mut result = Message::default();
    result.text(1, name);
    result.bytes(2, &typ.0);
    result.0
}
fn node(operation: &str, inputs: &[&str], output: &str, attributes: &[Vec<u8>]) -> Vec<u8> {
    let mut m = Message::default();
    for input in inputs {
        m.text(1, input);
    }
    m.text(2, output);
    m.text(3, output);
    m.text(4, operation);
    for attribute in attributes {
        m.bytes(5, attribute);
    }
    m.0
}
fn normalization(input: &str, prefix: &str, output: &str) -> Vec<u8> {
    node(
        "BatchNormalization",
        &[
            input,
            &format!("{prefix}.weight"),
            &format!("{prefix}.bias"),
            &format!("{prefix}.running_mean"),
            &format!("{prefix}.running_var"),
        ],
        output,
        &[float("epsilon", 1e-5)],
    )
}
pub fn build(tensors: &[(String, Vec<usize>, Vec<u8>)]) -> Vec<u8> {
    let mut graph = Message::default();
    graph.text(2, "YACE LarsNet magnitude masks");
    graph.bytes(11, &value("magnitude"));
    graph.bytes(12, &value("mask"));
    for (name, shape, bytes) in tensors {
        let mut t = Message::default();
        for n in shape {
            t.integer(1, *n as u64);
        }
        t.integer(2, 1);
        t.text(8, name);
        t.bytes(9, bytes);
        graph.bytes(5, &t.0);
    }
    graph.bytes(
        1,
        &node(
            "Transpose",
            &["magnitude"],
            "input_transpose",
            &[ints("perm", &[0, 2, 1, 3])],
        ),
    );
    graph.bytes(
        1,
        &normalization("input_transpose", "input_norm", "input_normalized"),
    );
    graph.bytes(
        1,
        &node(
            "Transpose",
            &["input_normalized"],
            "input",
            &[ints("perm", &[0, 2, 1, 3])],
        ),
    );
    let mut previous = "input".to_owned();
    for i in 1..=6 {
        let p = format!("enc{i}");
        let c = format!("c{i}");
        graph.bytes(
            1,
            &node(
                "Conv",
                &[
                    &previous,
                    &format!("{p}.conv.weight"),
                    &format!("{p}.conv.bias"),
                ],
                &c,
                &[ints("strides", &[2, 2]), ints("pads", &[2, 2, 2, 2])],
            ),
        );
        if i != 6 {
            let bn = format!("{p}_bn");
            previous = format!("{p}_out");
            graph.bytes(1, &normalization(&c, &format!("{p}.bn"), &bn));
            graph.bytes(
                1,
                &node("LeakyRelu", &[&bn], &previous, &[float("alpha", 0.2)]),
            );
        }
    }
    previous = "c6".into();
    for i in 1..=6 {
        let p = format!("dec{i}");
        if i != 1 {
            let cat = format!("{p}_cat");
            graph.bytes(
                1,
                &node(
                    "Concat",
                    &[&format!("c{}", 7 - i), &previous],
                    &cat,
                    &[integer("axis", 1)],
                ),
            );
            previous = cat;
        }
        let conv = format!("{p}_conv");
        let relu = format!("{p}_relu");
        graph.bytes(
            1,
            &node(
                "ConvTranspose",
                &[
                    &previous,
                    &format!("{p}.conv_trans.weight"),
                    &format!("{p}.conv_trans.bias"),
                ],
                &conv,
                &[
                    ints("strides", &[2, 2]),
                    ints("pads", &[2, 2, 2, 2]),
                    ints("output_padding", &[1, 1]),
                ],
            ),
        );
        graph.bytes(1, &node("Relu", &[&conv], &relu, &[]));
        previous = format!("{p}_out");
        graph.bytes(1, &normalization(&relu, &format!("{p}.bn"), &previous));
    }
    graph.bytes(
        1,
        &node(
            "Conv",
            &[&previous, "mask_layer.0.weight", "mask_layer.0.bias"],
            "mask_logits",
            &[ints("pads", &[3, 3, 3, 3]), ints("dilations", &[2, 2])],
        ),
    );
    graph.bytes(1, &node("Sigmoid", &["mask_logits"], "mask", &[]));
    let mut model = Message::default();
    model.integer(1, 8);
    model.text(2, "YACE Rust LarsNet converter");
    model.text(3, env!("CARGO_PKG_VERSION"));
    model.bytes(7, &graph.0);
    let mut opset = Message::default();
    opset.integer(2, 17);
    model.bytes(8, &opset.0);
    model.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attribute_repeated_ints_use_onnx_field_eight() {
        // AttributeProto.ints =8 (floats=7): this distinguishes a valid graph
        // from one whose metadata silently disappears in a decoder.
        assert_eq!(
            ints("a", &[1, 2]),
            vec![10, 1, b'a', 160, 1, 7, 64, 1, 64, 2]
        );
    }
}
