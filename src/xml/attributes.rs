/// Holds the key-value pairs.
pub struct Attributes {
	pub map: Vec<(String, String)>,
}

impl Attributes {
	/// Checks if an attribute exists.
	pub fn has(&self, name: &str) -> bool {
		self.map
			.iter()
			.any(|(attribute_name, _)| attribute_name.eq(name))
	}

	/// Gets the value of an attribute.
	pub fn get(&self, name: &str) -> Option<String> {
		self.map.iter().find_map(|(attribute_name, value)| {
			if attribute_name.eq(name) {
				return Some(value.to_string());
			}

			None
		})
	}

	/// Gets the value of an attribute, ignoring case of the name.
	pub fn get_case_insensitive(&self, name: &str) -> Option<String> {
		self.map.iter().find_map(|(attribute_name, value)| {
			if attribute_name.eq_ignore_ascii_case(name) {
				return Some(value.to_string());
			}

			None
		})
	}
}

/// Parses a string of attributes into a map of key-value pairs.
pub fn parse(text: &str) -> Attributes {
	let map = text
		.split(" ")
		.filter_map(|attribute| {
			// Ignore empty attributes
			if attribute.is_empty() {
				return None;
			}

			// Attributes are equals delimited key-value pairs
			let (name, value) = attribute.split_once("=")?;

			// Ignore attributes with no name
			if name.is_empty() {
				return None;
			}

			// Strip quotes around the value
			let value = value.trim_matches('"');

			// Ignore attributes with no value
			if value.is_empty() {
				return None;
			}

			Some((name.to_string(), value.to_string()))
		})
		.collect();

	Attributes { map }
}
