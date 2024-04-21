use std::error::Error;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::str::from_utf8;
use std::thread::{spawn, JoinHandle};
use std::time::Duration;

mod frame;

// https://stomp.github.io/stomp-specification-1.2.html

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Connection {
	open_host: String,

	tcp_stream: TcpStream,
	recieve_thread: Option<JoinHandle<()>>,
}

impl Connection {
	// Sends the CONNECT frame to the STOMP server.
	pub fn authenticate(&mut self, username: &str, password: &str) -> Result<(), Box<dyn Error>> {
		let headers = vec![
			("accept-version", "1.2"),
			("host", self.open_host.as_str()),
			("heart-beat", "0,0"), // TODO: Implement heart-beating
			("login", username),
			("passcode", password),
		];

		let frame = frame::create("CONNECT", Some(headers), None);

		self.tcp_stream.write_all(frame.as_bytes())?;

		return Ok(());
	}

	/// Subscribes to a topic on the STOMP server.
	pub fn subscribe(&mut self, identifier: u32, topic: &str) -> Result<(), Box<dyn Error>> {
		let id = identifier.to_string();

		let headers = vec![
			("id", id.as_str()),
			("destination", topic),
			("ack", "auto"), // TODO: Implement acknowledgements
		];

		let frame = frame::create("SUBSCRIBE", Some(headers), None);

		self.tcp_stream.write_all(frame.as_bytes())?;

		return Ok(());
	}

	/// Waits for the connection to close.
	pub fn wait(&mut self) -> Result<(), Box<dyn Error>> {
		// Don't bother if the thread no longer exists
		if self.recieve_thread.is_none() {
			return Ok(());
		}

		// Yoink the thread handle & wait for it to finish
		let result = self.recieve_thread.take().unwrap().join();
		if result.is_err() {
			return Err("Unable to join recieve thread".into());
		}

		return Ok(());
	}

	/// Closes the connection to the STOMP server.
	pub fn close(&mut self) -> Result<(), Box<dyn Error>> {
		self.tcp_stream.shutdown(Shutdown::Both)?;

		self.wait()?;

		return Ok(());
	}
}

/// Establishes a connection to a STOMP server.
pub fn open(
	host: &str,
	port: u16,
	timeout: Option<Duration>,
) -> Result<Connection, Box<dyn Error>> {
	// Convert the host name & port number into a usable socket address
	let address = format!("{}:{}", host, port)
		.to_socket_addrs()?
		.last()
		.expect(format!("Unable to convert '{}:{}' to socket address", host, port).as_str());

	// Open a TCP stream to the this address
	let tcp_stream = TcpStream::connect_timeout(&address, timeout.unwrap_or(DEFAULT_TIMEOUT))?;

	// Configure this stream
	tcp_stream.set_nodelay(true)?;
	//tcp_stream.set_nonblocking(true)?;
	tcp_stream.set_write_timeout(timeout.or(Some(DEFAULT_TIMEOUT)))?;

	// Spawn a thread to listen for incoming bytes
	let tcp_stream_clone = tcp_stream.try_clone()?;
	let recieve_thread = spawn(move || {
		let result = recieve_bytes(tcp_stream_clone); // Blocks until the TCP stream is closed
		if result.is_err() {
			let reason = result.err().unwrap_or("Unknown error".into()).to_string();
			eprintln!("Unable to recieve bytes: {}", reason);
		}
	});

	// Give the caller a handle to this connection
	return Ok(Connection {
		open_host: host.to_string(),

		tcp_stream,
		recieve_thread: Some(recieve_thread),
	});
}

/// Continuously waits for bytes from the STOMP server.
fn recieve_bytes(mut tcp_stream: TcpStream) -> Result<(), Box<dyn Error>> {
	let mut recieve_buffer = [0; 4096]; // 4 KiB
	let mut pending_data: Vec<u8> = Vec::new();

	//let mut processing_threads: Vec<JoinHandle<()>> = Vec::new();

	loop {
		let recieved_byte_count = tcp_stream.read(&mut recieve_buffer)?;

		// Nothing left, TCP stream is closed
		if recieved_byte_count == 0 {
			break;
		}

		pending_data.extend_from_slice(&recieve_buffer[..recieved_byte_count]);
		println!(
			"Recieved {}/{} byte(s)",
			recieved_byte_count,
			recieve_buffer.len()
		);

		while let Some(next_read_index) = parse_and_process_frame(&mut pending_data)? {
			pending_data.drain(..next_read_index);
		}

		/*
		// Capture only the bytes we recieved
		let recieved_bytes = &recieve_buffer[0..recieved_byte_count];
		println!(
			"Recieved {}/{} byte(s)",
			recieved_byte_count,
			recieve_buffer.len()
		);

		// Find the double line feed that divides the command + headers from the body
		let seperator_index = recieved_bytes
			.windows(2)
			.position(|bytes| bytes == [b'\n', b'\n'])
			.unwrap_or(recieved_byte_count); // Assume we don't have a body

		// Parse just the command + headers as UTF-8 text divided by line feeds
		let command_and_headers = String::from_utf8(recieved_bytes[0..seperator_index].to_vec())?;
		let mut command_and_headers_lines = command_and_headers.lines();

		// Command is the first line
		let command = command_and_headers_lines
			.next()
			.ok_or("No command in STOMP frame")?
			.trim(); // Remove trailing carriage return?
		println!("Command: '{}'", command);

		// The rest of the lines are headers
		let headers = command_and_headers_lines
			.map(|line| line.trim()) // Remove leading/trailing whitespace (trailing carriage return?)
			.filter(|line| !line.is_empty()) // Skip empty lines
			.map(|line| {
				// Divide the line into a key-value pair
				let parts = line.splitn(2, ":").collect::<Vec<&str>>();
				if parts.len() != 2 {
					return (None, None);
				}

				// Skip if either the name or value is empty
				let name = parts[0].to_string().to_lowercase();
				let value = parts[1].to_string();
				if (name.is_empty()) || (value.is_empty()) {
					return (None, None);
				}

				return (Some(name), Some(value));
			})
			.filter(|(name, value)| name.is_some() && value.is_some()) // Remove options with no value
			.map(|(name, value)| (name.unwrap(), value.unwrap())) // Get value from options
			.collect::<Vec<(String, String)>>();

		// Do we have a content length header?
		let content_length_header = headers
			.iter()
			.find(|(name, _)| name == "content-length")
			.map(|(_, value)| value.as_str());
		println!(
			" Content length header: '{}'",
			content_length_header.unwrap_or("N/A")
		);
		if content_length_header.is_some() {
			let content_length = content_length_header.unwrap().parse::<usize>()?;

			// Find the start of the body
			let body_start_index = seperator_index + 2; // Skip the double line feed
			let body_end_index = body_start_index + content_length; // Without trailing NT + LF

			// Cannot parse as UTF-8, this is GZIP compressed binary data
			let body_bytes = &recieved_bytes[body_start_index..body_end_index];
			println!(" Body is {} byte(s):", body_bytes.len());

			// Print as HEX
			for byte in body_bytes {
				print!("{:02X} ", byte);
			}
			println!("");

			// GZIP body always ends with two null terminators
			if body_bytes[body_bytes.len() - 1] != 0x00 || body_bytes[body_bytes.len() - 2] != 0x00
			{
				return Err("No null terminators on GZIP body".into());
			}

			// Ensure the last two bytes are a null terminator + line feed
			if body_end_index <= recieved_byte_count && recieved_bytes[body_end_index] != 0x00 {
				// Print as HEX
				let remaining_bytes = &recieved_bytes[seperator_index..recieved_byte_count];
				println!("\n\n\nRemaining {} byte(s):", remaining_bytes.len());
				for byte in remaining_bytes {
					print!("{:02X} ", byte);
				}
				println!("\n\n");

				// Print as HEX
				//println!("\nNT byte: {:02X}\n", &recieved_bytes[body_end_index]);

				return Err("No null terminator ending STOMP frame after body".into());
			}
			if body_end_index + 1 <= recieved_byte_count
				&& recieved_bytes[body_end_index + 1] != b'\n'
			{
				// Print as HEX
				let remaining_bytes = &recieved_bytes[seperator_index..recieved_byte_count];
				println!("\n\n\nRemaining {} byte(s):", remaining_bytes.len());
				for byte in remaining_bytes {
					print!("{:02X} ", byte);
				}
				println!("\n\n");

				// Print as HEX
				//println!("\nLF byte: {:02X}\n", &recieved_bytes[body_end_index + 1]);

				return Err("No line feed ending STOMP frame after body".into());
			}

			// Is the rest of the buffer just empty?
			if body_end_index + 2 <= recieved_byte_count {
				let remaining_bytes = &recieved_bytes[body_end_index + 2..recieved_byte_count];
				if !remaining_bytes.iter().all(|&byte| byte == 0) {
					// Print as HEX
					println!("\n\n\nRemaining {} byte(s):", remaining_bytes.len());
					for byte in remaining_bytes {
						print!("{:02X} ", byte);
					}
					println!("\n\n");

					return Err(" BUFFER STILL HAS DATAAAAAAA after body!".into());
				}
			}
		} else {
			let end_of_frame_index = seperator_index + 2; // Skip the double line feed

			// Ensure the last two bytes are a null terminator + line feed
			if end_of_frame_index + 1 <= recieved_byte_count
				&& recieved_bytes[end_of_frame_index] != 0x00
			{
				// Print as HEX
				let remaining_bytes = &recieved_bytes[seperator_index..recieved_byte_count];
				println!("\n\n\nRemaining {} byte(s):", remaining_bytes.len());
				for byte in remaining_bytes {
					print!("{:02X} ", byte);
				}
				println!("\n\n");

				return Err("No null terminator ending STOMP frame".into());
			}
			if end_of_frame_index + 2 <= recieved_byte_count
				&& recieved_bytes[end_of_frame_index + 1] != b'\n'
			{
				// Print as HEX
				let remaining_bytes = &recieved_bytes[seperator_index..recieved_byte_count];
				println!("\n\n\nRemaining {} byte(s):", remaining_bytes.len());
				for byte in remaining_bytes {
					print!("{:02X} ", byte);
				}
				println!("\n\n");

				return Err("No line feed ending STOMP frame".into());
			}

			// Is the rest of the buffer just empty?
			if end_of_frame_index + 3 <= recieved_byte_count {
				let remaining_bytes = &recieved_bytes[end_of_frame_index + 3..recieved_byte_count];
				if !remaining_bytes.iter().all(|&byte| byte == 0) {
					// Print as HEX
					println!("\n\n\nRemaining {} byte(s):", remaining_bytes.len());
					for byte in remaining_bytes {
						print!("{:02X} ", byte);
					}
					println!("\n\n");

					return Err(" BUFFER STILL HAS DATAAAAAAA!".into());
				}
			}
		}
		*/

		// We have a finished STOMP frame if there's a null terminator
		/*
		let null_terminator_index = read_buffer.iter().position(|&byte| byte == 0x00);
		if null_terminator_index.is_some() {
			let null_terminator_index = null_terminator_index.unwrap();

			frame_buffer.extend_from_slice(&read_buffer[0..null_terminator_index]);

			// Process the entire frame in a separate thread
			let frame_length = frame_buffer.len();
			let frame_buffer_clone = frame_buffer.clone();
			processing_threads.push(spawn(move || {
				let result = process_frame_bytes(frame_buffer_clone, frame_length);
				if result.is_err() {
					let reason = result.err().unwrap_or("Unknown error".into()).to_string();
					eprintln!("Unable to process frame bytes: {}", reason);
				}
			}));

			// Reset for the next frame
			frame_buffer.clear();
		}

		// Append partial frame
		frame_buffer
			.extend_from_slice(&read_buffer[null_terminator_index.unwrap_or(0)..byte_count]);
		*/

		// Empty for the next read
		//recieve_buffer.fill(0);
	}

	// Wait for all processing threads to finish
	/*
	for thread in processing_threads {
		thread.join().expect("Unable to join processing thread");
	}
	*/

	return Ok(());
}

/*
fn process_frame_bytes(frame: Vec<u8>, count: usize) -> Result<(), Box<dyn Error>> {
	let frame = frame::parse(frame)?;

	match frame.command.as_str() {
		"CONNECTED" => {
			if frame.headers.is_none() {
				return Err("No headers in CONNECTED frame".into());
			}

			let headers = frame.headers.unwrap();

			let server_name = &headers
				.iter()
				.find(|(name, _)| name == "server")
				.ok_or("No server header in CONNECTED frame")?
				.1;
			let protocol_version = &headers
				.iter()
				.find(|(name, _)| name == "version")
				.ok_or("No version header in CONNECTED frame")?
				.1;
			let heartbeat_policy = &headers
				.iter()
				.find(|(name, _)| name == "heart-beat")
				.ok_or("No heart-beat header in CONNECTED frame")?
				.1;
			let session_identifier = &headers
				.iter()
				.find(|(name, _)| name == "session")
				.ok_or("No session header in CONNECTED frame")?
				.1;

			println!(
				"Heart-beating '{}' with server '{}' using STOMP v{}.",
				heartbeat_policy, server_name, protocol_version
			);
			println!("Session ID: '{}'", session_identifier);
		}

		_ => {
			println!("{} byte(s) -> '{}'", count, frame.command);

			if frame.headers.is_some() {
				for (name, value) in frame.headers.unwrap() {
					println!("\t{}: '{}'", name, value);
				}
			}
		}
	}

	return Ok(());
}
*/
fn parse_and_process_frame(
	pending_data: &mut Vec<u8>,
) -> Result<Option<usize>, Box<dyn std::error::Error>> {
	let min_frame_length = 2; // Minimum bytes needed to have a complete frame including 'NT LF'
	if pending_data.len() >= min_frame_length
		&& pending_data
			.windows(2)
			.position(|bytes| bytes == [b'\n', b'\n'])
			.is_some()
	{
		let command_end_index = pending_data.iter().position(|&b| b == b'\n').unwrap();
		let command = from_utf8(&pending_data[..command_end_index])?.trim();

		let headers_start_index = command_end_index + 1;
		let header_end_index = pending_data
			.windows(2)
			.position(|bytes| bytes == [b'\n', b'\n'])
			.unwrap() + 1;
		let content_length =
			find_content_length(&pending_data[headers_start_index..header_end_index])?;

		let body_start_index = header_end_index + 1; // Move past the double new line

		if let Some(length) = content_length {
			let body_end_index = body_start_index + length;
			if body_end_index + 1 <= pending_data.len()
				&& pending_data[body_end_index] == 0x00
				&& pending_data[body_end_index + 1] == b'\n'
			{
				process_frame(
					command,
					&pending_data[headers_start_index..header_end_index],
					&pending_data[body_start_index..body_end_index],
				)?;
				// Consider frame end after NT LF
				return Ok(Some(body_end_index + 2));
			} else {
				return Ok(None); // Wait for more data including NT LF
			}
		} else {
			process_frame(
				command,
				&pending_data[headers_start_index..header_end_index],
				&[],
			)?;
			// No body, consider frame end just after headers if ends with NT LF
			if header_end_index + 1 < pending_data.len()
				&& pending_data[header_end_index + 1] == 0x00
				&& pending_data[header_end_index + 2] == b'\n'
			{
				return Ok(Some(header_end_index + 3));
			} else {
				return Ok(None); // Wait for NT LF
			}
		}
	}
	Ok(None) // Not enough data for even the minimum frame
}

fn find_content_length(headers: &[u8]) -> Result<Option<usize>, Box<dyn std::error::Error>> {
	let headers_str = from_utf8(headers)?;
	Ok(headers_str.lines().find_map(|line| {
		line.split_once(':').and_then(|(key, value)| {
			if key
				.trim_end_matches('\r')
				.eq_ignore_ascii_case("content-length")
			{
				value.trim().parse::<usize>().ok()
			} else {
				None
			}
		})
	}))
}

fn process_frame(
	command: &str,
	headers: &[u8],
	body: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
	println!("Command: {}", command);
	println!("Headers: {:?}", from_utf8(headers));
	if !body.is_empty() {
		println!("Body: {} byte(s)", body.len());
	} else {
		println!("No body!");
	}
	println!("");
	Ok(())
}
