#![feature(custom_derive, plugin, fnbox)]
#![plugin(rocket_codegen)]
// Limit for error_chain
#![recursion_limit = "1024"]

#[macro_use]
extern crate clap;
#[macro_use]
extern crate error_chain;
extern crate rocket;
extern crate rocket_contrib;
#[macro_use]
extern crate serde_derive;
extern crate serde;

use std::fs::File;
use std::io::{Cursor, Read};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::result::Result;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use clap::{App, AppSettings, Arg, SubCommand};
use rocket::{config, Data, State};
use rocket::http::{ContentType, Status};
use rocket::response::{self, content, Redirect, Responder, Response};
use rocket::request::{Form, FromRequest, Request, Outcome};
use rocket_contrib::Template;

#[allow(unused_doc_comment)]
mod errors {
	// Create the Error, ErrorKind, ResultExt, and Result types
	error_chain! {
		foreign_links {
			Io(::std::io::Error);
		}
	}
}
use errors::*;

struct Config {
	address: IpAddr,
	port: u16,
}

#[derive(Serialize)]
struct Xkcd {
}

impl Xkcd {
	fn new() -> Xkcd {
		Xkcd {
		}
	}
}

struct WebFile {
	path: PathBuf,
	content: Vec<u8>,
}

impl WebFile {
	fn new<P: Into<PathBuf>>(path: P) -> Option<WebFile> {
		let path = path.into();
		path.to_str().and_then(|s| get_web_file(s)).map(|content| {
			WebFile {
				path: path,
				content: content,
			}
		})
	}
}

impl<'r> Responder<'r> for WebFile {
	fn respond_to(self, _: &Request) -> Result<Response<'r>, Status> {
		let mut response = Response::build();
		self.path.extension()
			.and_then(|e| e.to_str())
			.and_then(|e| ContentType::from_extension(e))
			.map(|ct| response.header(ct));
		response.sized_body(Cursor::new(self.content)).ok()
	}
}

fn validate<T: FromStr>(val: String) -> std::result::Result<(), String>
	where T::Err: std::fmt::Display {
	T::from_str(val.as_str()).map(|_| ()).map_err(|e| format!("{}", e))
}

fn get_web_file(path: &str) -> Option<Vec<u8>> {
	File::open(path).ok().and_then(|mut file| {
		let mut content = Vec::new();
		file.read_to_end(&mut content).ok().map(|_| content)
	})
}

fn main() {
	let fun: Box<std::boxed::FnBox()> = {
		// Parse command line options
		let args = App::new("xkcd")
			.version(crate_version!())
			.author(crate_authors!())
			.about("xkcd offline clone")
			// Recursively for all subcommands
			.global_settings(&[
				AppSettings::ColoredHelp,
				AppSettings::VersionlessSubcommands,
			])
			.setting(AppSettings::SubcommandRequiredElseHelp)
			.subcommand(SubCommand::with_name("server")
				.about("start the local xkcd server")
				.arg(Arg::with_name("address").short("a").long("address")
					.validator(validate::<IpAddr>)
					.default_value("0.0.0.0")
					.help("The address for the server to listen"))
				.arg(Arg::with_name("port").short("p").long("port")
					.validator(validate::<u16>)
					.default_value("8080")
					.help("The port for the server to listen")))
			.subcommand(SubCommand::with_name("update")
				.about("Update the data from xkcd"))
			.get_matches();

		if let Some(server_cmd) = args.subcommand_matches("server") {
			let config = Config {
				address: IpAddr::from_str(server_cmd.value_of("address").unwrap())
					.unwrap(),
				port: u16::from_str(server_cmd.value_of("port").unwrap()).unwrap(),
			};
			Box::new(|| start_server(config, Xkcd::new()))
		} else if let Some(_) = args.subcommand_matches("update") {
			Box::new(|| println!("Updating xkcd"))
		} else {
			Box::new(|| unreachable!("Invalid subcommand"))
		}
	};
	fun()
}

fn start_server(config: Config, xkcd: Xkcd) {
	// Enable logging
	rocket::logger::init(rocket::config::LoggingLevel::Normal);
	let rocket_config = config::Config::build(config::Environment::active()
			.unwrap())
		.address(config.address.to_string())
		.port(config.port)
		.unwrap();
	let r = rocket::custom(rocket_config, false).mount("/", routes![
		handle_index,
		handle_static,
		handle_search,
	])
		.manage(Arc::new(Mutex::new(xkcd)))
		.attach(Template::fairing());
	r.launch();
}

#[get("/static/<file..>")]
fn handle_static<'r>(file: PathBuf) -> Option<WebFile> {
	WebFile::new(Path::new("static").join(file))
}

#[get("/")]
fn handle_index(xkcd: State<Arc<Mutex<Xkcd>>>) -> Template {
	let xkcd = xkcd.lock().unwrap();
	let context = &*xkcd;
	Template::render("index", context)
}

#[derive(FromForm)]
struct SearchForm {
	q: String,
}

#[get("/search?<search>")]
fn handle_search(xkcd: State<Arc<Mutex<Xkcd>>>, search: SearchForm) -> Template {
	let xkcd = xkcd.lock().unwrap();
	let context = &*xkcd;
	Template::render("search", context)
}
