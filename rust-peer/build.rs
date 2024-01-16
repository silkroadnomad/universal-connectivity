// fn main() {
//      println!("cargo:rerun-if-changed=src/peer.proto");
//    // println!("cargo:rerun-if-changed=src/peer.proto");
//     prost_build::compile_protos(&["src/peer.proto"], &["src/"]);
// }
use std::io::Result;
fn main() -> Result<()> {
    prost_build::compile_protos(&["src/peer.proto"], &["src/"])?;
    Ok(())
}