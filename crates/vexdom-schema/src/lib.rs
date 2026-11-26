//! The Bridge wire (ADR-025): `schema/vexdom.capnp` and its generated module. Wasm-safe.

#[allow(clippy::all, unused_qualifications, unsafe_code)]
pub mod vexdom_capnp {
    include!(concat!(env!("OUT_DIR"), "/vexdom_capnp.rs"));
}

pub use vexdom_capnp::*;

/// Reader limits both ends use: generous enough for a full-page screenshot or snapshot.
pub fn reader_options() -> capnp::message::ReaderOptions {
    let mut options = capnp::message::ReaderOptions::new();
    options
        .traversal_limit_in_words(Some(128 * 1024 * 1024 / 8))
        .nesting_limit(128);
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_result_round_trips() {
        let mut message = capnp::message::Builder::new_default();
        let mut error = message.init_root::<action_result::Builder>().init_error();
        error.set_code("UNKNOWN_REF");
        error.set_message("Unknown ref: e5. Take a new snapshot.");
        let mut bytes = Vec::new();
        capnp::serialize::write_message(&mut bytes, &message).unwrap();
        let read = capnp::serialize::read_message(&mut bytes.as_slice(), reader_options()).unwrap();
        let root = read.get_root::<action_result::Reader>().unwrap();
        match root.which().unwrap() {
            action_result::Error(error) => {
                let error = error.unwrap();
                assert_eq!(error.get_code().unwrap().to_str().unwrap(), "UNKNOWN_REF");
            }
            action_result::Ok(_) => panic!("expected an error"),
        }
    }
}
