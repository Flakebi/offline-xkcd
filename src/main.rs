#![feature(custom_derive, plugin, fnbox)]
#![plugin(rocket_codegen)]
// Limit for error_chain
#![recursion_limit = "1024"]

extern crate bit_vec;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate error_chain;
extern crate rand;
extern crate regex;
extern crate reqwest;
extern crate rocket;
extern crate rocket_contrib;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use std::fs::File;
use std::io::{BufReader, Cursor};
use std::io::prelude::*;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::result::Result;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use bit_vec::BitVec;
use clap::{App, AppSettings, Arg, SubCommand};
use rand::Rng;
use regex::Regex;
use rocket::{config, State};
use rocket::http::{ContentType, Status};
use rocket::response::{Redirect, Responder, Response};
use rocket::request::Request;
use rocket_contrib::Template;

#[allow(unused_doc_comment)]
mod errors {
	// Create the Error, ErrorKind, ResultExt, and Result types
	error_chain! {
		foreign_links {
			Io(::std::io::Error);
			Reqwest(::reqwest::Error);
			Serde(::serde_json::Error);
		}
	}
}
use errors::*;

const CHAR_WHITELIST: &[char] = &['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '-', '_', '.'];
/// The maximum number of matches for a search request.
const MAX_MATCHES: usize = 150;

struct Config {
	address: IpAddr,
	port: u16,
}

#[derive(Serialize, Deserialize, Debug)]
struct Comic {
	num: usize,
	year: String,
	month: String,
	day: String,
	link: String,
	news: String,
	transcript: String,
	safe_title: String,
	title: String,
	alt: String,
	img: String,
}

impl Comic {
	fn get_img_path(&self) -> String {
		let start = self.img.rfind('/').map(|i| i + 1).unwrap_or(0);
		// Limit to allowed chars
		let img = &self.img[start..];
		let mut res = String::new();
		for c in img.chars() {
			if CHAR_WHITELIST.contains(&c) {
				res.push(c);
			}
		}
		res
	}
}

#[derive(Serialize, Debug)]
struct Xkcd {
	comics: Vec<Option<Comic>>,
}

impl Xkcd {
	fn new() -> Xkcd {
		println!("Loading local comics");
		let mut comics = Vec::new();
		let file_count = Path::new("data").read_dir().map(|it| it.count()).unwrap_or(0);
		// Load comics
		for i in 0..file_count {
			if let Ok(content) = File::open(format!("data/{}.json", i))
			.and_then(|f| {
				let mut reader = BufReader::new(f);
				let mut content = String::new();
				reader.read_to_string(&mut content).map(|_| content)
			}) {
				print!("\rLoading comic {}", i);
				std::io::stdout().flush().unwrap();
				let comic = serde_json::from_str(&content).unwrap();
				comics.push(Some(comic));
			} else {
				comics.push(None);
			}
		}
		// Remove trailing Nones
		let mut i = comics.len();
		while comics[i - 1].is_none() {
			i -= 1;
		}
		comics.drain(i..);
		println!("\rAll local comics loaded");

		Xkcd {
			comics: comics,
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
			Box::new(|| update().unwrap())
		} else {
			Box::new(|| unreachable!("Invalid subcommand"))
		}
	};
	fun()
}

fn download_image(comic: &Comic) -> Result<(), Error> {
	// Get image
	let img = if comic.img.starts_with("http:") {
		format!("https{}", &comic.img[5..])
	} else {
		comic.img.to_string()
	};
	let mut buf = Vec::new();
	reqwest::get(&img)?.read_to_end(&mut buf)?;
	{
		let mut f = File::create(format!("data/{}", comic.get_img_path())
			.as_str())?;
		f.write_all(&buf)?;
	}
	Ok(())
}

fn update() -> Result<(), Error> {
	println!("Updating xkcd");
	let xkcd = Xkcd::new();
	// Get current id from /info.0.json
	let mut result = String::new();
	reqwest::get("https://xkcd.com/info.0.json")?.read_to_string(&mut result)?;
	let latest_comic: Comic = serde_json::from_str(&result)?;

	// Excluding the num because ids are one less (starting from 0 instead of 1)
	for id in 0..latest_comic.num {
		let _ = (|| -> errors::Result<()> {
			if let Some(comic) = xkcd.comics.get(id).and_then(|c| c.as_ref()) {
				let path = format!("data/{}", comic.get_img_path());
				if !Path::new(&path).is_file() {
					download_image(comic)?;
				}
			} else {
				print!("\rDownloading {} of {}", id, latest_comic.num - 1);
				std::io::stdout().flush().unwrap();
				// Get image data from /<id + 1>/info.0.json
				result.clear();
				reqwest::get(format!("https://xkcd.com/{}/info.0.json", id + 1)
					.as_str())?.read_to_string(&mut result)?;
				let comic: Comic = serde_json::from_str(&result)?;
				{
					let mut f = File::create(format!("data/{}.json", id).as_str())?;
					f.write_all(result.as_bytes())?;
				}

				// Get image
				download_image(&comic)?;
			}
			Ok(())
		})();
	}
	println!("                                              \n{} images up to date", latest_comic.num);
	Ok(())
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
		handle_comic,
		handle_comic_path,
		handle_static,
		handle_search,
		handle_random,
	])
		.manage(Arc::new(Mutex::new(xkcd)))
		.attach(Template::fairing());
	r.launch();
}

#[get("/static/<file..>")]
fn handle_static(file: PathBuf) -> Option<WebFile> {
	WebFile::new(Path::new("static").join(file))
}

#[get("/")]
fn handle_index(xkcd: State<Arc<Mutex<Xkcd>>>) -> Redirect {
	let xkcd = xkcd.lock().unwrap();
	let id = xkcd.comics.len() - 1;
	Redirect::to(format!("/comic/?id={}", id).as_str())
}

#[derive(FromForm)]
struct IdForm {
	id: Option<usize>
}

#[get("/comic?<id>")]
fn handle_comic(xkcd: State<Arc<Mutex<Xkcd>>>, id: IdForm)
	-> Result<Template, Redirect> {
	#[derive(Serialize)]
	struct ComicContext<'a> {
		id: usize,
		max_id: usize,
		comic: &'a Comic,
		img_path: String,
	}

	let xkcd = xkcd.lock().unwrap();
	if let Some(id) = id.id {
		if let Some(comic) = xkcd.comics.get(id).and_then(|c| c.as_ref()) {
			let context = ComicContext {
				id: id,
				max_id: xkcd.comics.len() - 1,
				comic: comic,
				img_path: comic.get_img_path(),
			};
			Ok(Template::render("comic", &context))
		} else {
			Err(Redirect::to(format!("/comic/?id={}",
				xkcd.comics.len() - 1).as_str()))
		}
	} else {
		// Invalid id
		Err(Redirect::to("/comic/?id=0"))
	}
}

#[get("/comic/<file..>")]
fn handle_comic_path(_xkcd: State<Arc<Mutex<Xkcd>>>, file: PathBuf)
	-> Option<WebFile> {
	WebFile::new(Path::new("data").join(file))
}

#[derive(Serialize)]
struct ComicSearchResult<'a> {
	id: usize,
	comic: &'a Comic,
	img_path: String,
}

struct SearchState<'a> {
	results: Vec<ComicSearchResult<'a>>,
	/// Comics that are already in the results
	stored_comics: BitVec,
}

impl<'a> SearchState<'a> {
	fn new(xkcd: &'a Xkcd) -> SearchState<'a> {
		SearchState {
			results: Vec::new(),
			stored_comics: BitVec::from_elem(xkcd.comics.len(), false),
		}
	}

	fn filter_comics(&mut self, xkcd: &'a Xkcd, f: &Fn(&Comic) -> bool) {
		for (i, comic) in xkcd.comics.iter().enumerate() {
			if let Some(ref comic) = *comic {
				if !self.stored_comics[i] && f(comic) {
					self.results.push(ComicSearchResult {
						id: i,
						comic: comic,
						img_path: comic.get_img_path(),
					});
					self.stored_comics.set(i, true);
				}
				if self.results.len() >= MAX_MATCHES {
					break;
				}
			}
		}
	}
}

#[derive(FromForm)]
struct SearchForm {
	q: String,
}

#[get("/search?<search>")]
fn handle_search(xkcd: State<Arc<Mutex<Xkcd>>>, search: SearchForm)
	-> Template {
	#[derive(Serialize)]
	struct SearchContext<'a, 'b> {
		search: &'a str,
		max_id: usize,
		results: Vec<ComicSearchResult<'b>>,
	}

	let xkcd = xkcd.lock().unwrap();
	let mut search_state = SearchState::new(&*xkcd);

	// Search for matching words
	let rgx = Regex::new(format!(r"\b{}\b", regex::escape(&search.q)).as_str()).unwrap();
	search_state.filter_comics(&*xkcd, &|c| rgx.is_match(&c.title));
	search_state.filter_comics(&*xkcd, &|c| rgx.is_match(&c.alt) || rgx.is_match(&c.transcript));
	// Search for substrings
	search_state.filter_comics(&*xkcd, &|c| c.title.contains(&search.q));
	search_state.filter_comics(&*xkcd, &|c| c.alt.contains(&search.q) || c.transcript.contains(&search.q));

	let context = SearchContext {
		search: &search.q,
		max_id: xkcd.comics.len() - 1,
		results: search_state.results,
	};
	Template::render("search", &context)
}

#[get("/random")]
fn handle_random(xkcd: State<Arc<Mutex<Xkcd>>>) -> Redirect {
	let xkcd = xkcd.lock().unwrap();
	let mut rng = rand::thread_rng();
	let id = rng.gen_range(0, xkcd.comics.len());
	Redirect::to(format!("/comic/?id={}", id).as_str())
}
