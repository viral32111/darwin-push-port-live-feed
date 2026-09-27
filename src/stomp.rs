use anyhow::{Context, Result, bail};
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};
use tokio::time::timeout;

const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct Frame {
	pub command: String,
	pub headers: Vec<(String, String)>,
	pub body: Vec<u8>,
}

impl Frame {
	fn new(command: impl Into<String>) -> Self {
		Frame {
			command: command.into(),
			headers: Vec::new(),
			body: Vec::new(),
		}
	}

	fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
		self.headers.push((name.into(), value.into()));
		self
	}

	pub fn get_header(&self, name: &str) -> Option<&str> {
		self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
	}

	fn serialize(&self) -> Vec<u8> {
		let mut out = Vec::new();

		out.extend_from_slice(self.command.as_bytes());
		out.push(b'\n');

		for (name, value) in &self.headers {
			out.extend_from_slice(name.as_bytes());
			out.push(b':');
			out.extend_from_slice(value.as_bytes());
			out.push(b'\n');
		}

		if !self.body.is_empty() {
			out.extend_from_slice(format!("content-length:{}\n", self.body.len()).as_bytes());
		}

		out.push(b'\n');
		out.extend_from_slice(&self.body);
		out.push(b'\0');

		out
	}
}

type SubscriptionMap = Arc<Mutex<HashMap<String, mpsc::Sender<Frame>>>>;

pub struct Client {
	write: tokio::net::tcp::OwnedWriteHalf,
	subscriptions: SubscriptionMap,
	next_subscription_id: u32,
}

pub struct Subscription {
	receiver: mpsc::Receiver<Frame>,
}

impl Stream for Subscription {
	type Item = Frame;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
		self.receiver.poll_recv(cx)
	}
}

async fn read_frame(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Result<Option<Frame>> {
	let command = loop {
		let mut line = String::new();

		if reader.read_line(&mut line).await? == 0 {
			return Ok(None);
		}

		let trimmed = line.trim_end_matches(['\r', '\n']);
		if !trimmed.is_empty() {
			break trimmed.to_string();
		}
	};

	let mut headers = Vec::new();
	loop {
		let mut line = String::new();

		if reader.read_line(&mut line).await? == 0 {
			bail!("EOF while reading headers");
		}

		let trimmed = line.trim_end_matches(['\r', '\n']);
		if trimmed.is_empty() {
			break;
		}

		if let Some((name, value)) = trimmed.split_once(':') {
			headers.push((name.to_string(), value.to_string()));
		}
	}

	let content_length = headers
		.iter()
		.find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
		.and_then(|(_, value)| value.trim().parse::<usize>().ok());

	let body = if let Some(length) = content_length {
		let mut buffer = vec![0u8; length];

		reader.read_exact(&mut buffer).await.context("reading body")?;

		let mut null = [0u8; 1];
		reader.read_exact(&mut null).await.context("reading NULL terminator")?;

		buffer
	} else {
		let mut buffer = Vec::new();

		reader.read_until(b'\0', &mut buffer).await.context("reading body until NULL")?;

		if buffer.last() == Some(&b'\0') {
			buffer.pop();
		}

		buffer
	};

	Ok(Some(Frame {
		command,
		headers,
		body,
	}))
}

impl Client {
	pub async fn connect(
		address: &str,
		hostname: &str,
		username: &str,
		password: &str,
		client_id: &str,
	) -> Result<Self> {
		let stream = TcpStream::connect(address).await.context(format!("Unable to connect to '{address}'"))?;

		stream.set_nodelay(true).context("Setting TCP_NODELAY")?;

		let (read_half, write_half) = stream.into_split();
		let mut reader = BufReader::new(read_half);
		let mut write = write_half;

		let connect = Frame::new("CONNECT")
			.header("accept-version", "1.2")
			.header("host", hostname)
			.header("login", username)
			.header("passcode", password)
			.header("client-id", client_id)
			.header("heart-beat", "0,0");

		timeout(WRITE_TIMEOUT, write.write_all(&connect.serialize()))
			.await
			.context("Write timeout sending CONNECT")?
			.context("Sending CONNECT")?;

		let response = read_frame(&mut reader).await?.context("No response from server")?;
		match response.command.as_str() {
			"CONNECTED" => {}
			"ERROR" => {
				bail!("Server returned ERROR: {}", String::from_utf8_lossy(&response.body))
			}
			other => bail!("Expected CONNECTED, got: {other}"),
		}

		let subscriptions: SubscriptionMap = Arc::new(Mutex::new(HashMap::new()));
		let subscriptions_clone = subscriptions.clone();

		tokio::spawn(async move {
			loop {
				match read_frame(&mut reader).await {
					Ok(Some(frame)) if frame.command == "MESSAGE" => {
						if let Some(subscription_id) = frame.get_header("subscription").map(str::to_string) {
							let subscriptions = subscriptions_clone.lock().await;

							if let Some(tx) = subscriptions.get(&subscription_id) {
								let _ = tx.send(frame).await;
							}
						}
					}

					Ok(Some(frame)) if frame.command == "ERROR" => {
						eprintln!("STOMP ERROR: {}", String::from_utf8_lossy(&frame.body));
					}

					Ok(Some(_)) => {}
					Ok(None) => {
						eprintln!("STOMP connection closed by server");
						break;
					}

					Err(error) => {
						eprintln!("STOMP read error: {error:#}");
						break;
					}
				}
			}

			subscriptions_clone.lock().await.clear();
		});

		Ok(Client {
			write,
			subscriptions,
			next_subscription_id: 0,
		})
	}

	pub async fn subscribe(&mut self, destination: &str, extra_headers: Vec<(String, String)>) -> Result<Subscription> {
		let subscription_id = format!("sub-{}", self.next_subscription_id);
		self.next_subscription_id += 1;

		let mut frame = Frame::new("SUBSCRIBE")
			.header("id", &subscription_id)
			.header("destination", destination)
			.header("ack", "auto");

		for (name, value) in extra_headers {
			frame = frame.header(name, value);
		}

		timeout(WRITE_TIMEOUT, self.write.write_all(&frame.serialize()))
			.await
			.context("Write timeout sending SUBSCRIBE")?
			.context("Sending SUBSCRIBE")?;

		let (transmit, receive) = mpsc::channel(1024);
		self.subscriptions.lock().await.insert(subscription_id, transmit);

		Ok(Subscription {
			receiver: receive,
		})
	}
}
