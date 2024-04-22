pub enum Headers {
	ContentLength,
	ContentType,
}

impl Headers {
	/// Converts the header to its name.
	pub fn as_str(&self) -> &'static str {
		match self {
			Headers::ContentLength => "content-length",
			Headers::ContentType => "content_hyphen_type", // bruh
		}
	}
}
