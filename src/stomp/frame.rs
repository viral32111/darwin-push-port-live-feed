// https://stomp.github.io/stomp-specification-1.2.html#Augmented_BNF

/// Creates a STOMP frame.
pub fn create(command: &str, headers: Option<Vec<(&str, &str)>>, body: Option<&str>) -> String {
	// Just command & body if there aren't any headers
	if headers.is_none() {
		return format!("{}\n\n{}\0", command, body.unwrap_or(""));
	}

	// Convert headers into colon delimited key-value pairs between line feeds
	let header_lines = headers
		.unwrap()
		.iter()
		.map(|(name, value)| format!("{}:{}", name, value))
		.collect::<Vec<String>>()
		.join("\n");

	// Include headers in the frame
	return format!("{}\n{}\n\n{}\0", command, header_lines, body.unwrap_or(""));
}

/*
pub struct Frame {
	pub command: String,
	pub headers: Option<Vec<(String, String)>>,
	pub body: Option<String>,
}

/// Parses a STOMP frame.
pub fn parse(bytes: Vec<u8>) -> Result<Frame, Box<dyn Error>> {
	let text = String::from_utf8_lossy(&bytes);
	//println!("Parsing frame: '{}'", text);

	let headers_index = text.find("\n").ok_or("No headers in STOMP frame")? + 1;
	let body_index = text.find("\n\n").ok_or("No body in STOMP frame")? + 2;

	let command = text[0..headers_index].trim_end();

	let headers = text[headers_index..body_index]
		.lines()
		.filter(|line| !line.is_empty()) // Skip empty lines
		.map(|line| {
			let parts = line.splitn(2, ":").collect::<Vec<&str>>();

			if parts.len() != 2 {
				return (String::from(""), String::from(""));
			}

			return (parts[0].to_string().to_lowercase(), parts[1].to_string());
		})
		.filter(|(name, _)| !name.is_empty()) // Skip headers with no name
		.collect::<Vec<(String, String)>>();

	// TODO: Body might be gzip compressed binary blob

	return Ok(Frame {
		command: command.to_string(),
		headers: Some(headers),
		body: None,
	});
}
*/
