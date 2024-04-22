use env_file_reader::read_file;
use std::error::Error;

mod stomp;
mod xml;

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

	let mut connection = stomp::open(host, port, None)?;

	connection.authenticate(username, password)?;

	connection.subscribe(0, "/topic/darwin.pushport-v16")?;
	//connection.subscribe(1, "/topic/darwin.status")?;

	connection.wait()?;
	connection.close()?;

	Ok(())
}
