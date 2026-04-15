use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::NotFound, e)
    })?;
    std::env::set_var("PROTOC", protoc);

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(
            &[
                manifest_dir.join("spec_to_proof.proto"),
                manifest_dir.join("nlp.proto"),
                manifest_dir.join("proof_service.proto"),
            ],
            &[manifest_dir],
        )?;
    Ok(())
}
