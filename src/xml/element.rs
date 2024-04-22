use std::error::Error;

use super::attributes::{self, Attributes};

pub struct Element {
	pub name: Option<String>,
	pub value: Option<String>,
	pub attributes: Option<Attributes>,
	pub children: Option<Vec<Element>>,
}

impl Element {}

/// Parses the first element in an XML document.
pub fn parse(text: &str) -> Result<(Element, usize), Box<dyn Error>> {
	// Don't bother if we have nothing
	if text.is_empty() {
		return Err("Empty XML element".into());
	}

	// We're not an element, we're inner text
	if !text.starts_with("<") && !text.ends_with(">") {
		return Ok((
			Element {
				name: None,
				value: Some(text.to_string()),
				attributes: None,
				children: None,
			},
			text.len(),
		));
	}

	// Find where I begin
	let mut opening_tag_end_position = text.find(">").expect("Tag not terminated");

	// Back up if we're self-closing
	let is_self_closing = text[opening_tag_end_position - 1..opening_tag_end_position].eq("/");
	if is_self_closing {
		opening_tag_end_position -= 1;
	}

	let opening_tag = &text[1..opening_tag_end_position];

	// Extract the name
	let name_end_position = opening_tag.find(" ").unwrap_or(opening_tag.len());
	let name = &opening_tag[..name_end_position];

	// Parse the attributes
	let attributes = if opening_tag.contains(" ") {
		Some(attributes::parse(&opening_tag[name_end_position..]))
	} else {
		None
	};

	// Don't continue if we're self-closing, as there won't be a closing tag & children
	if is_self_closing {
		return Ok((
			Element {
				name: Some(name.to_string()),
				value: None,
				attributes,
				children: None,
			},
			opening_tag_end_position + (if is_self_closing { 2 } else { 1 }),
		));
	}

	// Find where I end
	let closing_tag_position = text
		.find(&format!("</{}>", name))
		.expect("Element not terminated");

	// Parse the top-level children
	let inner_text = &text[opening_tag_end_position + 1..closing_tag_position];
	let mut children = Vec::new();
	let mut position = 0;
	loop {
		let (child, child_end_position) = parse(&inner_text[position..])?;
		position += child_end_position;
		children.push(child);

		if position >= inner_text.len() {
			break;
		}
	}

	Ok((
		Element {
			name: Some(name.to_string()),
			value: None,
			attributes,
			children: Some(children),
		},
		closing_tag_position + name.len() + 3, // Skip past </name>
	))
}
