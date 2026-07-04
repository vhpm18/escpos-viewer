pub fn pretty_hex(data: &[u8]) -> String {
    let mut output = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        output.push_str(&format!("{:04x}: ", i * 16));
        for b in chunk {
            output.push_str(&format!("{:02x} ", b));
        }
        output.push('\n');
    }
    output
}
