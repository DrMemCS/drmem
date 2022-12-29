use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    #[cfg(feature = "grpc")]
    {
        let idl_src = &["proto/drmem.proto"];
        let dirs = &["proto"];

        println!("cargo:rerun-if-changed=build.rs");

        // Only build the server bindings. The DrMem project doesn't
        // include any client applications. If one wants to write a
        // client, they can take the `.proto` file and generate the
        // appropriate bindings.

        tonic_build::configure()
            .build_client(false)
            .build_server(true)
            .protoc_arg("--experimental_allow_proto3_optional")
            .compile(idl_src, dirs)?;

        // recompile protobufs only if any of the proto files changes.

        for file in idl_src {
            println!("cargo:rerun-if-changed={}", file);
        }
    }

    Ok(())
}
