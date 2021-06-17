fn main() {
    tonic_build::configure()
        .out_dir("src/generated")
        .compile(
            &["proto/service.proto", "proto/compact_formats.proto"],
            &["proto"],
        )
        .unwrap();
}
