use self::{declaration::Declaration, element::Element};
use std::error::Error;

pub mod attributes;
pub mod declaration;
pub mod element;

pub struct Document {
	pub declaration: Declaration,
	pub root: Element,
}

impl Document {}

/// Parses an XML document.
pub fn parse(text: &str) -> Result<Document, Box<dyn Error>> {
	// Don't bother if we have nothing
	if text.is_empty() {
		return Err("Empty XML document".into());
	}

	// Parse the XML declaration
	let (declaration, declaration_end_position) = declaration::parse(text)?;

	// Parse the root element
	let (root, root_element_end_position) = element::parse(&text[declaration_end_position..])?;

	// Sanity check
	if declaration_end_position + root_element_end_position > text.len() {
		return Err("Remaining unparsed text in XML document".into());
	}

	Ok(Document { declaration, root })
}

/*
/// Minifies an XML document.
/// Removing new lines, indentation, etc. Spacing between attributes is preserved.
pub fn minify(text: &str) -> String {
	text.chars().filter(|c| !c.is_whitespace()).collect()
}
*/
