use super::attributes;
use std::error::Error;

/// Represents an XML declaration.
pub struct Declaration {
	pub version: String,
	pub encoding: String,
	pub standalone: String,
}

/// Parses the attributes within an XML declaration.
pub fn parse(text: &str) -> Result<(Declaration, usize), Box<dyn Error>> {
	// Don't bother if we do not begin properly
	if !text.starts_with("<?xml") {
		return Err("XML declaration missing".into());
	}

	// Find the end
	let declaration_end_position = text.find("?>").expect("XML declaration not terminated") + 2;
	let declaration = &text[..declaration_end_position];

	// Parse the XML declaration
	let attributes = attributes::parse(&declaration[5..declaration.len() - 2]);
	let version = attributes
		.get("version")
		.expect("XML declaration missing version attribute");
	let encoding = attributes
		.get("encoding")
		.expect("XML declaration missing encoding attribute");
	let standalone = attributes
		.get("standalone")
		.expect("XML declaration missing standalone attribute");

	// Return the attributes & where it ends
	Ok((
		Declaration {
			version,
			encoding,
			standalone,
		},
		declaration_end_position,
	))
}
