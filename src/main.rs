use env_file_reader::read_file;
use std::{error::Error, process::exit};
use viral32111_stomp::{frame::Frame, header::Headers, open};
use viral32111_xml::{element::Element, parse};

/*
service:2024:05:03:G79740:location:ABWDXR:departure:staff = 19:54:00
service:2024:05:03:G79740:location:ABWDXR:departure:public = 19:54:00
service:2024:05:03:G79740:location:ABWDXR:departure:estimate = 19:54:00
service:2024:05:03:G79740:location:ABWDXR:platform = 3
service:2024:05:03:G79740:location:HTRWAPT:arrival:staff = 20:50:30
service:2024:05:03:G79740:location:HTRWAPT:arrival:working = 20:51:00
service:2024:05:03:G79740:location:HTRWAPT:arrival:estimate = 20:49:00
service:2024:05:03:G79740:location:HTRWAPT:departure:staff = 20:52:00
service:2024:05:03:G79740:location:HTRWAPT:departure:public = 20:52:00
service:2024:05:03:G79740:location:HTRWAPT:departure:estimate = 20:52:00
service:2024:05:03:G79740:location:HTRWAPT:platform = 1
*/

fn main() -> Result<(), Box<dyn Error>> {
	let environment_variables = read_file(".env")?;

	let host = environment_variables
		.get("DARWIN_HOST")
		.expect("Environment variable 'DARWIN_HOST' not present in .env file");
	let port = environment_variables
		.get("DARWIN_PORT")
		.expect("Environment variable 'DARWIN_PORT' not present in .env file")
		.parse::<u16>()?;
	let username = environment_variables
		.get("DARWIN_USERNAME")
		.expect("Environment variable 'DARWIN_USERNAME' not present in .env file");
	let password = environment_variables
		.get("DARWIN_PASSWORD")
		.expect("Environment variable 'DARWIN_PASSWORD' not present in .env file");

	let mut connection = open(host, port, None)?;

	connection.authenticate(username, password)?;

	connection.subscribe(0, "/topic/darwin.pushport-v16")?;
	//connection.subscribe(1, "/topic/darwin.status")?;

	for frame in connection.frame_receiver.iter() {
		match frame {
			Ok(frame) => {
				print_frame(frame)?;
			}
			Err(error) => {
				eprintln!("Frame receiver error: {}", error);
			}
		}
	}

	connection.wait()?;
	connection.close()?;

	Ok(())
}

/// Displays a STOMP frame in the console.
fn print_frame(frame: Frame) -> Result<(), Box<dyn std::error::Error>> {
	if frame.command == "CONNECTED" {
		return Ok(());
	}

	if frame.command == "MESSAGE" && frame.body.is_some() {
		let body = frame.body.unwrap();

		let content_type = frame.headers.iter().find_map(|(name, value)| {
			if name.eq(Headers::ContentType.as_str()) {
				return Some(value.to_string());
			}

			None
		});

		if content_type.is_some() && content_type.unwrap().eq("application/xml") {
			let document = parse(&body)?;

			if document.root.name.is_some()
				&& document.root.name.as_ref().unwrap() == "Pport"
				&& document.root.attributes.is_some()
				&& document.root.children.is_some()
			{
				let timestamp = document
					.root
					.attributes
					.as_ref()
					.unwrap()
					.get("ts")
					.unwrap();
				println!("\n@ {}", timestamp);

				let pport = document.root.children.as_ref().unwrap();
				if pport.len() != 1 {
					println!("Pport has more than one child?");
					recursively_print_element_children(&document.root, 0)?;

					exit(0);
				}

				let ur = pport.first().unwrap();
				if ur.name.is_some() && ur.name.as_ref().unwrap() == "uR" {
					let children = ur.children.as_ref().unwrap();

					for child in children.iter() {
						recursively_print_element_children(child, 0)?;
					}
				}
			} else {
				println!("XML declaration:");
				println!(" Version: {}", document.declaration.version);
				println!(" Encoding: {}", document.declaration.encoding);
				println!(" Standalone: {}", document.declaration.standalone);

				println!("");
				recursively_print_element_children(&document.root, 0)?;

				exit(0);
			}
		}
	} else {
		println!("{}", frame.command);

		for (name, value) in frame.headers.clone() {
			println!("{}: {}", name, value);
		}

		println!("");

		if frame.body.is_some() {
			println!("{}", frame.body.unwrap());
		}

		exit(0);
	}

	Ok(())
}

fn recursively_print_element_children(element: &Element, depth: u32) -> Result<(), Box<dyn Error>> {
	if element.name.is_some() {
		println!(
			"{}<{}>",
			" ".repeat(depth as usize),
			element.name.as_ref().unwrap()
		);
	}

	if element.value.is_some() {
		println!(
			"{}{}",
			" ".repeat(depth as usize),
			element.value.as_ref().unwrap()
		);
	}

	if element.attributes.is_some() {
		let attributes = element.attributes.as_ref().unwrap();

		for (name, value) in attributes.map.iter() {
			println!(" {}{}: {}", " ".repeat(depth as usize), name, value);
		}
	}

	if element.children.is_some() {
		let children = element.children.as_ref().unwrap();

		for child in children.iter() {
			recursively_print_element_children(child, depth + 1)?;
		}
	}

	Ok(())
}
