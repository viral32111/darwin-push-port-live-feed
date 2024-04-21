use std::error::Error;

mod stomp;

const USERNAME: &str = "";
const PASSWORD: &str = "";

const HOST: &str = "darwin-dist-44ae45.nationalrail.co.uk";
const PORT: u16 = 61613;

fn main() -> Result<(), Box<dyn Error>> {
	let mut connection = stomp::open(HOST, PORT, None)?;

	connection.authenticate(USERNAME, PASSWORD)?;

	connection.subscribe(0, "/topic/darwin.pushport-v16")?;

	connection.wait()?;
	connection.close()?;

	return Ok(());
}

/*
fn handle_incoming_packets(mut stream: &TcpStream) {
	let mut byte_buffer = [0; 1024]; // 1 KiB

	let mut my_packet_current_index = 0;
	let mut my_packet = [0; 1024 * 1000]; // 1 MiB

	loop {
		println!("Waiting for incoming TCP packet...");
		let byte_count = stream
			.read(&mut byte_buffer)
			.expect("Failed to read from TCP stream");
		if byte_count == 0 {
			println!("Connection closed by remote host.");
			break;
		}

		// find null terminator
		let packet_end_index_result = byte_buffer.iter().position(|&byte| byte == '\n' as u8);

		// if we didnt find one, we need to read more
		if packet_end_index_result.is_none() {
			// copy entire read buffer into packet buffer starting at the current index (we have fragmented TCP packet)
			my_packet[my_packet_current_index..my_packet_current_index + byte_count]
				.copy_from_slice(&byte_buffer[..byte_count]);
			my_packet_current_index += byte_count;
			println!(
				"fragmented tcp packet of {} bytes, now at {}",
				byte_count, my_packet_current_index
			);

			// reset
			print!("Clearing read buffer...");
			byte_buffer.fill(0);
			println!(" Done.");

			continue;
		}

		// we did find the null terminator, so copy read buffer up to the null terminator into packet buffer at the current index (we have a complete TCP packet)
		let packet_end_index = packet_end_index_result.unwrap();
		println!(
			"complete tcp packet ends at index {}/{}",
			packet_end_index, byte_count
		);
		my_packet[my_packet_current_index..my_packet_current_index + packet_end_index]
			.copy_from_slice(&byte_buffer[..packet_end_index]);
		my_packet_current_index += packet_end_index;

		// process packet
		process_tcp_packet(&stream, &my_packet, my_packet_current_index)
			.expect("Failed to process TCP packet");

		// reset
		print!("Clearing packet & read buffer...");
		my_packet_current_index = 0;
		my_packet.fill(0);
		byte_buffer.fill(0);
		println!(" Done.");
	}
}

fn process_tcp_packet(
	stream: &TcpStream,
	packet: &[u8],
	length: usize,
) -> Result<(), Box<dyn Error>> {
	print!("Decoding {} byte(s) as UTF-8...", length);
	let message = String::from_utf8_lossy(&packet[..length]);
	println!(" Done.");
	//println!(" Message: '{}'", &message[..100]);

	println!("Parsing {} byte(s)...", length);
	let (command, headers, body_index) = parse_stomp_frame(&message);
	let body = &packet[body_index..]; // body aint utf-8, its gzipped
	println!(" Command: '{}'", command);
	println!(" Headers: {:?}", headers);
	println!(" Body Index: {}", body_index);

	// base64 encode body
	/*
	let engine = base64::engine::general_purpose::STANDARD;
	let encoded_body = engine.encode(body);
	println!(" Body: '{}'", encoded_body);
	*/

	println!("Processing STOMP frame...");
	process_stomp_frame(&stream, &command, headers, body)?;

	return Ok(());
}

fn process_stomp_connection(
	mut stream: &TcpStream,
	headers: Vec<(&str, &str)>,
) -> Result<(), Box<dyn Error>> {
	let session_id = headers
		.iter()
		.find(|(name, _)| *name == "session")
		.expect("Failed to find session header")
		.1;

	println!("Our session identifier is '{}'", session_id);

	print!("Sending STOMP subscribe frame...");
	let frame = create_stomp_frame(
		"SUBSCRIBE",
		vec![
			("id", "0"),
			("destination", "/topic/darwin.pushport-v16"),
			("ack", "auto"),
		],
		"",
	);
	stream
		.write_all(frame.as_bytes())
		.expect("Failed to send STOMP subscribe frame");
	println!(" Done.");

	return Ok(());
}

fn process_stomp_message(
	_stream: &TcpStream,
	headers: Vec<(&str, &str)>,
	body: &[u8],
) -> Result<(), Box<dyn Error>> {
	let message_id = headers
		.iter()
		.find(|(name, _)| *name == "message-id")
		.expect("Failed to find message-id header")
		.1
		.replace("\\c", ":")
		.replace("\\r", "\r")
		.replace("\\n", "\n")
		.replace("\\\\", "\\");

	let sequence_id = headers
		.iter()
		.find(|(name, _)| *name == "PushPortSequence")
		.expect("Failed to find sequence-id header")
		.1;

	let content_length = headers
		.iter()
		.find(|(name, _)| *name == "content-length")
		.expect("Failed to find content-length header")
		.1;

	println!(
		"Decompressing {} byte message ({}, {})...",
		content_length, message_id, sequence_id
	);

	let mut decoder = GzDecoder::new(body);
	let mut read_buffer = String::new();
	decoder
		.read_to_string(&mut read_buffer)
		.expect("Failed to decompress message");

	println!("Decompressed message: '{}'", read_buffer);

	return Ok(());
}

fn process_stomp_frame(
	stream: &TcpStream,
	command: &str,
	headers: Vec<(&str, &str)>,
	body: &[u8],
) -> Result<(), Box<dyn Error>> {
	match command {
		"CONNECTED" => {
			process_stomp_connection(stream, headers)?;
			return Ok(());
		}

		"MESSAGE" => {
			process_stomp_message(stream, headers, body)?;
			return Ok(());
		}

		"RECEIPT" => {
			println!("Received receipt from STOMP server");
			return Ok(());
		}

		"ERROR" => {
			println!("Received error from STOMP server");
			return Ok(());
		}

		_ => {
			println!("Received unknown command from STOMP server");
			return Err("Unknown STOMP command".into());
		}
	}
}

fn parse_stomp_frame(frame: &str) -> (String, Vec<(&str, &str)>, usize) {
	let lines: Vec<&str> = frame.lines().collect();

	let command = lines[0].to_string();

	let seperator_index = lines
		.iter()
		.position(|&line| line.is_empty())
		.unwrap_or(lines.len());

	let headers = lines[1..seperator_index]
		.iter()
		.map(|line| {
			let mut parts = line.splitn(2, ":");

			let name = parts
				.next()
				.expect("Failed to parse STOMP frame header name");

			let value = parts
				.next()
				.expect("Failed to parse STOMP frame header value");

			(name, value)
		})
		.collect();

	//let body = lines[seperator_index + 1..].join("\n");
	let body_index = frame
		.find("\n\n")
		.expect("Failed to find body start in STOMP frame")
		+ 2;

	return (command, headers, body_index);
}

fn create_stomp_frame(command: &str, headers: Vec<(&str, &str)>, body: &str) -> String {
	let mapped_headers = headers
		.iter()
		.map(|(name, value)| format!("{}:{}", name, value))
		.collect::<Vec<String>>()
		.join("\n");

	let frame = format!("{}\n{}\n\n{}\0", command, mapped_headers, body);

	return frame;
}
 */
