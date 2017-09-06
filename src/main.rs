#![feature(attr_literals, custom_derive, plugin, fnbox, iterator_step_by)]
#![plugin(rocket_codegen)]
// Limit for error_chain
#![recursion_limit = "1024"]

extern crate bit_vec;
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
extern crate structopt;
#[macro_use]
extern crate structopt_derive;

use std::fs::{DirBuilder, File};
use std::io::{BufReader, Cursor};
use std::io::prelude::*;
use std::net::IpAddr;
#[cfg(all(not(dox), any(target_os = "redox", unix)))]
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::result::Result;
use std::sync::{Arc, Mutex};
use std::thread;

use bit_vec::BitVec;
use rand::Rng;
use regex::Regex;
use rocket::{config, State};
use rocket::http::{ContentType, Status};
use rocket::response::{Redirect, Responder, Response};
use rocket::request::Request;
use rocket_contrib::Template;
use structopt::StructOpt;
use structopt::clap::AppSettings;

#[allow(unused_doc_comment)]
mod errors {
	// Create the Error, ErrorKind, ResultExt, and Result types
	error_chain! {
		foreign_links {
			Io(::std::io::Error);
			Reqwest(::reqwest::Error);
			Rocket(::rocket::config::ConfigError);
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
		if !comics.is_empty() {
			let mut i = comics.len();
			while comics[i - 1].is_none() {
				i -= 1;
			}
			comics.drain(i..);
		}
		println!("\rAll local comics loaded");

		Xkcd {
			comics,
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
				path,
				content,
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

fn get_web_file(path: &str) -> Option<Vec<u8>> {
	File::open(path).ok().and_then(|mut file| {
		let mut content = Vec::new();
		file.read_to_end(&mut content).ok().map(|_| content)
	})
}

#[derive(StructOpt, Debug)]
#[structopt(global_settings_raw = "&[AppSettings::ColoredHelp, AppSettings::VersionlessSubcommands]")]
enum Args {
	#[structopt(name = "server", about = "start the local xkcd server")]
	Server {
		/// The address for the server to listen
		#[structopt(short = "a", long = "address", default_value = "0.0.0.0")]
		address: IpAddr,
		/// The port for the server to listen
		#[structopt(short = "p", long = "port", default_value = "8080")]
		port: u16,
	},

	#[structopt(name = "update", about = "update the data from xkcd")]
	Update {
		/// The number of threads to use for parallel downloading
		#[structopt(short = "j", long = "jobs", default_value = "4")]
		jobs: u16,
	},
}

quick_main!(|| -> Result<(), Error> {
	let fun: Box<std::boxed::FnBox() -> Result<(), Error>> = {
		// Parse command line options
		let args = Args::from_args();

		match args {
			Args::Server { address, port } => {
				let config = Config {
					address,
					port,
				};
				Box::new(|| start_server(config, Xkcd::new()))
			}
			Args::Update { jobs } => Box::new(move || update(jobs)),
		}
	};
	fun()?;
	Ok(())
});

fn download_image(comic: &Comic) -> Result<(), Error> {
	print!("\rDownloading {}", comic.num - 1);
	std::io::stdout().flush().unwrap();
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

fn download_comic(id: usize, comic: Option<&Comic>) -> Result<(), Error> {
	if let Some(comic) = comic {
		let path = format!("data/{}", comic.get_img_path());
		if !Path::new(&path).is_file() {
			download_image(comic)?;
		}
	} else {
		// Get image data from /<id + 1>/info.0.json
		let mut result = String::new();
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
}

fn update(thread_count: u16) -> Result<(), Error> {
	println!("Updating xkcd");

	// Be sure that the data folder exists
	let mut dir = DirBuilder::new();
	dir.recursive(true);
	#[cfg(all(not(dox), any(target_os = "redox", unix)))]
	dir.mode(0o755);

	dir.create("data").unwrap();

	let xkcd = Arc::new(Xkcd::new());
	// Get current id from /info.0.json
	let mut result = String::new();
	reqwest::get("https://xkcd.com/info.0.json")?.read_to_string(&mut result)?;
	let latest_comic: Comic = serde_json::from_str(&result)?;
	println!("Latest comic: {}", latest_comic.num - 1);

	// Parallel download
	let mut threads = Vec::new();
	for i in 0..(thread_count as usize) {
		// Excluding the num because ids are one less (starting from 0 instead of 1)
		let end = latest_comic.num;
		let xkcd = xkcd.clone();
		threads.push(thread::spawn(move || {
			for id in (i..end).step_by(thread_count as usize) {
				let comic = xkcd.comics.get(id).and_then(|c| c.as_ref());
				let _ = download_comic(id, comic);
			}
		}));
	}

	// Wait for threads to finish
	for t in threads {
		t.join().unwrap();
	}

	println!("                                              \r\
		{} images up to date", latest_comic.num);
	Ok(())
}

fn start_server(config: Config, xkcd: Xkcd) -> Result<(), Error> {
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
	Ok(())
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
		next: usize,
		prev: usize,
		max_id: usize,
		comic: &'a Comic,
		img_path: String,
	}

	let xkcd = xkcd.lock().unwrap();
	if let Some(id) = id.id {
		if let Some(comic) = xkcd.comics.get(id).and_then(|c| c.as_ref()) {
			let next = {
				let mut i = id;
				while let Some(c) = xkcd.comics.get(i + 1) {
					i += 1;
					if c.is_some() {
						break;
					}
				}
				i
			};
			let prev = {
				let mut i = id;
				while let Some(c) = xkcd.comics.get(i.saturating_sub(1)) {
					if i == 0 {
						break;
					}
					i -= 1;
					if c.is_some() {
						break;
					}
				}
				i
			};

			let context = ComicContext {
				id,
				next,
				prev,
				max_id: xkcd.comics.len() - 1,
				comic,
				img_path: comic.get_img_path(),
			};
			Ok(Template::render("comic", &context))
		} else {
			Err(Redirect::to(format!("/comic/?id={}",
				xkcd.comics.len() - 1).as_str()))
		}
	} else {
		// Invalid id (like -1)
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
						comic,
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
