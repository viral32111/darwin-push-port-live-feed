use gethostname::gethostname;
use sha2::{Digest, Sha256};

pub fn anonymous_host_id() -> String {
	let host_name = gethostname().into_string().expect("Hostname contains invalid UTF-8 characters");

	if host_name.is_empty() {
		panic!("Hostname is empty");
	}

	let digest = Sha256::digest(host_name.as_bytes());
	format!("{digest:x}")
}
