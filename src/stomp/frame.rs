use crate::stomp::header::Headers;
use flate2::read::GzDecoder;
use std::{error::Error, io::Read, str::from_utf8};

// https://stomp.github.io/stomp-specification-1.2.html

/// Represents a STOMP frame.
pub struct Frame {
	pub command: String,
	pub headers: Vec<(String, String)>,
	pub body: Option<String>,
}

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

/// Attempts to parse the first STOMP frame in a byte buffer.
pub fn parse(buffer: &mut Vec<u8>) -> Result<Option<(Frame, usize)>, Box<dyn Error>> {
	// TODO: This implementation does not account for optional CR before each LF

	// Can't continue until we have at least a NT + LF
	if buffer.len() < 2 {
		return Ok(None); // Wait for more data
	}

	// Locate the double LF between headers & body
	let seperator_position = buffer.windows(2).position(|bytes| bytes == [b'\n', b'\n']);
	if seperator_position.is_none() {
		return Ok(None); // Wait for more data
	}

	// Extract the command from the first line
	let command_end_position = buffer.iter().position(|&byte| byte == b'\n').unwrap();
	let command = from_utf8(&buffer[..command_end_position])?
		.trim_end() // Strip trailing CR/LF
		.to_string();

	// Extract the headers hereafter until the double LF
	let headers_start_position = command_end_position + 1;
	let headers_end_position = seperator_position.unwrap() + 1;
	if headers_end_position > buffer.len() {
		return Ok(None); // Wait for more data
	}
	let headers = from_utf8(&buffer[headers_start_position..headers_end_position])?
		.lines()
		.filter_map(|line| {
			// Skip empty lines
			if line.is_empty() {
				return None;
			}

			// Headers are colon delimited key-value pairs
			let (name, value) = line.split_once(":")?;

			// Ignore headers with no name
			if name.is_empty() {
				return None;
			}

			// Force name to lowercase
			let name = name.to_lowercase();

			// Apply transformations to value
			let value = value
				.replace("\\r", "\r")
				.replace("\\n", "\n")
				.replace("\\c", ":")
				.replace("\\\\", "\\");

			// Force name to lowercase
			Some((name, value.to_string()))
		})
		.collect::<Vec<(String, String)>>();

	// Find the size of the body
	let content_length = headers.iter().find_map(|(name, value)| {
		if name.eq(Headers::ContentLength.as_str()) {
			return value.parse::<usize>().ok();
		}

		None
	});

	// Frame is finished if we don't have a body
	if content_length.is_none() {
		// Ensure we're terminated with a NT + LF
		if buffer.len() < headers_end_position + 2 {
			return Ok(None); // Wait for more data
		}
		if buffer[headers_end_position + 1] != 0x00 {
			return Err("Frame not null terminated".into());
		}
		if buffer[headers_end_position + 2] != b'\n' {
			return Err("Frame not terminated with a new line".into());
		}

		// Return the frame & the position of where this frame ends
		return Ok(Some((
			Frame {
				command,
				headers,
				body: None,
			},
			headers_end_position + 1, // Skip the double LF
		)));
	}

	// Decompress the body
	let body_start_position = headers_end_position + 1; // Move past the double LF
	let body_length = content_length.unwrap();
	let body_end_position = body_start_position + body_length;
	if body_end_position > buffer.len() {
		return Ok(None); // Wait for more data
	}
	let mut decompressor = GzDecoder::new(&buffer[body_start_position..body_end_position]);
	let mut body = String::new();
	decompressor.read_to_string(&mut body)?;

	// Ensure we're terminated with a NT + LF
	if buffer.len() < body_end_position + 1 {
		return Ok(None); // Wait for more data
	}
	if buffer[body_end_position] != 0x00 {
		return Err("Frame not null terminated".into());
	}
	if buffer[body_end_position + 1] != b'\n' {
		return Err("Frame not terminated with a new line".into());
	}

	// Return the frame & the position of where this frame ends
	Ok(Some((
		Frame {
			command,
			headers,
			body: Some(body),
		},
		body_end_position + 1, // Skip the NT + LF
	)))
}
