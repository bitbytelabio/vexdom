fn main() {
    println!("cargo:rerun-if-changed=schema/vexdom.capnp");
    capnpc::CompilerCommand::new()
        .src_prefix("schema")
        .file("schema/vexdom.capnp")
        .run()
        .expect("compiling schema/vexdom.capnp needs the `capnp` tool on PATH");
}
