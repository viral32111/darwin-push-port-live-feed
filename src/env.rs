use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Env {
	pub dpplf_host: String,

	#[serde(default = "defaults::dpplf_port")]
	pub dpplf_port: u16,

	pub dpplf_username: String,
	pub dpplf_password: String,

	#[serde(default = "defaults::redis_url")]
	pub redis_url: String,

	#[serde(default = "defaults::redis_prefix")]
	pub redis_prefix: String,

	#[serde(default = "defaults::redis_timeout")]
	pub redis_timeout: u64,

	#[serde(default = "defaults::redis_ttl")]
	pub redis_ttl: u64,

	#[serde(default = "defaults::dpplf_data_directory")]
	pub dpplf_data_directory: String,
}

mod defaults {
	pub fn dpplf_port() -> u16 {
		61613
	}

	pub fn redis_url() -> String {
		"redis://127.0.0.1:6379/0".into()
	}

	pub fn redis_prefix() -> String {
		"darwin".into()
	}

	pub fn redis_timeout() -> u64 {
		5
	}

	pub fn redis_ttl() -> u64 {
		604800 // 7 days
	}

	pub fn dpplf_data_directory() -> String {
		"/var/tmp/dpplf".into()
	}
}

pub fn load(dotenv_path: Option<&str>, verbose: bool) -> Result<Env, Box<dyn std::error::Error>> {
	let path = dotenv_path.unwrap_or(".env");

	match dotenvy::from_filename(path) {
		Ok(_) => {}

		Err(dotenvy::Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {} // Ignore if .env file is not found (not local dev)
		Err(err) => return Err(err.into()),
	}

	let config = envy::from_env::<Env>()?;

	if verbose {
		print_table(&config);
	}

	Ok(config)
}

fn print_table(config: &Env) {
	let mut table = comfy_table::Table::new();
	table.load_preset(comfy_table::presets::ASCII_FULL_CONDENSED);
	table.set_header(["Name", "Value"]);

	let rows: &[(&str, String)] = &[
		("DPPLF_HOST", config.dpplf_host.clone()),
		("DPPLF_PORT", config.dpplf_port.to_string()),
		("DPPLF_USERNAME", config.dpplf_username.clone()),
		(
			"DPPLF_PASSWORD",
			if config.dpplf_password.is_empty() {
				String::new()
			} else {
				"*".repeat(fastrand::usize(16..32))
			},
		),
		("REDIS_URL", config.redis_url.clone()),
		("REDIS_PREFIX", config.redis_prefix.clone()),
		("REDIS_TIMEOUT", config.redis_timeout.to_string()),
		("REDIS_STALE_AFTER_SECONDS", config.redis_ttl.to_string()),
		("DPPLF_DATA_DIRECTORY", config.dpplf_data_directory.clone()),
	];

	for (key, value) in rows {
		table.add_row([key.to_string(), value.clone()]);
	}

	println!("{table}");
}
