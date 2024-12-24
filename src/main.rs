use env_file_reader::read_file;
use std::{error::Error, path::Path, process::exit};
use viral32111_stomp::{frame::Frame, header::Headers, open};
use viral32111_xml::parse;

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
	if !Path::new(".env").exists() {
		eprintln!("The '.env' file does not exist in the current directory!");
		exit(1);
	}

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
				handle_stomp_frame(frame)?;
			}
			Err(error) => {
				eprintln!("Unable to receive STOMP frame! ({})\nAre the 'DARWIN_USERNAME' and 'DARWIN_PASSWORD' environment variables correct?", error);
			}
		}
	}

	connection.wait()?;
	connection.close()?;

	Ok(())
}

fn handle_stomp_frame(frame: Frame) -> Result<(), Box<dyn Error>> {
	if frame.command == "CONNECTED" {
		println!("Connected to STOMP server!");
		return Ok(());
	}

	if frame.command == "MESSAGE" && frame.body.is_some() {
		let body = frame.body.unwrap();

		let content_type_option = frame.headers.iter().find_map(|(name, value)| {
			if name.eq(Headers::ContentType.as_str()) {
				return Some(value.to_string());
			}

			None
		});

		if content_type_option.is_none() {
			return Err("No content type specified in message!".into());
		}

		let content_type = content_type_option.unwrap();
		if !content_type.eq("application/xml") {
			return Err(format!("Unexpected content type '{}'", content_type).into());
		}

		handle_stomp_xml_body(body)?;

		return Ok(());
	}

	// Dump unknown STOMP frames
	println!("{}", frame.command);
	for (name, value) in frame.headers.clone() {
		println!("{}: {}", name, value);
	}
	println!("");
	if frame.body.is_some() {
		println!("{}", frame.body.unwrap());
	}

	Ok(())
}

fn handle_stomp_xml_body(body: String) -> Result<(), Box<dyn std::error::Error>> {
	let document = parse(&body)?;

	let root_name = document.root.name.as_ref();
	if root_name.is_none() {
		return Err("Root element has no name!".into());
	}
	let root_name = root_name.unwrap();

	let root_attributes = document.root.attributes.as_ref();
	if root_attributes.is_none() {
		return Err("Root element has no attributes!".into());
	}

	let root_children = document.root.children.as_ref();
	if root_children.is_none() {
		return Err("Root element has no children!".into());
	}

	if root_name != "Pport" {
		return Err("Root element is not 'Pport'!".into());
	}

	let timestamp_iso8601 = root_attributes.as_ref().unwrap().get("ts").unwrap();

	std::fs::write(format!("data/{}.xml", timestamp_iso8601), body)?;

	let pport_elements = root_children.as_ref().unwrap();
	if pport_elements.len() != 1 {
		return Err("Pport element has more than 1 child!".into());
	}

	let ur = pport_elements.first().unwrap();
	let ur_name = ur.name.as_ref();
	if ur_name.is_none() {
		return Err("Pport -> uR element has no name!".into());
	}
	let ur_name = ur_name.unwrap();
	if ur_name != "uR" {
		return Err("Pport -> uR element is not 'uR'!".into());
	}

	// let ur_children = ur.children.as_ref().unwrap();

	Ok(())
}
