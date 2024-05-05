use self::frame::Frame;
use std::error::Error;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{spawn, JoinHandle};
use std::time::Duration;

pub mod frame;
pub mod header;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Represents a connection to a STOMP server.
pub struct Connection {
	tcp_stream: TcpStream,
	receive_thread: Option<JoinHandle<()>>,
	host_header: String,
	pub frame_receiver: Receiver<Result<Frame, String>>, // String instead of Box<dyn Error> as the latter doesn't implement Send trait
}

impl Connection {
	// Sends the CONNECT frame to the STOMP server.
	pub fn authenticate(&mut self, username: &str, password: &str) -> Result<(), Box<dyn Error>> {
		let headers = vec![
			("accept-version", "1.2"),
			("host", self.host_header.as_str()),
			("heart-beat", "0,0"), // TODO: Implement heart-beating
			("login", username),
			("passcode", password),
		];

		let frame = frame::create("CONNECT", Some(headers), None);

		self.tcp_stream.write_all(frame.as_bytes())?;

		Ok(())
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

		Ok(())
	}

	/// Waits for the connection to close.
	pub fn wait(&mut self) -> Result<(), Box<dyn Error>> {
		// Don't bother if the thread no longer exists
		if self.receive_thread.is_none() {
			return Ok(());
		}

		// Yoink the thread handle & wait for it to finish
		let result = self.receive_thread.take().unwrap().join();
		if result.is_err() {
			return Err("Unable to join receive thread".into());
		}

		Ok(())
	}

	/// Closes the connection to the STOMP server.
	pub fn close(&mut self) -> Result<(), Box<dyn Error>> {
		self.tcp_stream.shutdown(Shutdown::Both)?;

		self.wait()?;

		Ok(())
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
	tcp_stream.set_write_timeout(timeout.or(Some(DEFAULT_TIMEOUT)))?;

	let (frame_sender, frame_receiver) = channel();

	// Spawn a thread to listen for incoming bytes
	let tcp_stream_clone = tcp_stream.try_clone()?;
	let receive_thread = spawn(move || {
		let result = receive_bytes(tcp_stream_clone, frame_sender); // Blocks until the TCP stream is closed

		if result.is_err() {
			let reason = result.err().unwrap_or("Unknown error".into()).to_string();
			panic!("Unable to receive bytes: {}", reason);
		}
	});

	// Give the caller a handle to this connection
	Ok(Connection {
		tcp_stream,
		receive_thread: Some(receive_thread),
		host_header: host.to_string(),
		frame_receiver,
	})
}

/// Continuously waits for bytes from the STOMP server.
fn receive_bytes(
	mut tcp_stream: TcpStream,
	frame_sender: Sender<Result<Frame, String>>,
) -> Result<(), Box<dyn Error>> {
	let mut receive_buffer = [0; 4096]; // 4 KiB
	let mut pending_data: Vec<u8> = Vec::new(); // Infinite

	loop {
		// Try to receive some bytes
		let received_byte_count = tcp_stream.read(&mut receive_buffer)?;
		if received_byte_count == 0 {
			return Ok(()); // Give up, there's nothing left to receive
		}

		// Append the received bytes to the unprocessed data
		pending_data.extend_from_slice(&receive_buffer[..received_byte_count]);

		// Remove any complete frames from the unprocessed data
		while let Some((frame, end_position)) = frame::parse(&mut pending_data)? {
			pending_data.drain(..end_position + 1);
			frame_sender.send(Ok(frame))?;
		}
	}
}
