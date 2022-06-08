fn main() {
    tonic_build::configure()
        .out_dir("src/generated")
        .compile(
            &["proto/service.proto", "proto/compact_formats.proto"],
            &["proto"],
        )
        .unwrap();

    create_c_bindings();
}

#[allow(dead_code)]
fn create_c_bindings() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let config = cbindgen::Config {
        language: cbindgen::Language::C,
        ..Default::default()
    };

    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file("binding.h");
}
