pub enum Headers {
	ContentLength,
}

impl Headers {
	/// Converts the header to its name.
	pub fn as_str(&self) -> &'static str {
		match self {
			Headers::ContentLength => "content-length",
		}
	}
}
