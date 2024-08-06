#![allow(dead_code, unused_imports)]
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use poppler::{PopplerPage, PopplerDocument};
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::signal;
use tower::ServiceExt;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

use crate::intern::{get_str, intern, PoolId};

mod file_format;
mod intern;

const CACHE_PATH: &str = "paper-engine-cache.pec";

lazy_static::lazy_static! {
    pub static ref STEMMER: Stemmer = Stemmer::create(Algorithm::English);
}

fn log<T: std::fmt::Debug>(msg: T) -> T {
    eprintln!("{msg:#?}");
    msg
}

async fn root() -> Html<&'static str> {
    include_str!("index.html").into()
}

#[derive(Debug, Default)]
pub struct TfIdf {
    global_term_count: HashMap<Term, usize>,
    documents: HashMap<String, Document>,
}

#[derive(Debug)]
pub struct Document {
    title: String,
    path: String,
    // TODO: Add notes and tags
    term_frequency: HashMap<Term, f64>,
}

type Term = PoolId;

type DocShared = Arc<RwLock<TfIdf>>;

impl TfIdf {
    // TODO: Give higher weights to exact matches over stemmed matches
    //
    // Also normalize to not favor longer documents ("the")
    pub fn sort_documents(&self, terms: &[Term]) -> Vec<(u64, String, String)> {
        let mut documents = BTreeMap::new();
        for term in terms {
            let mut term_contains_all = 0;
            for (_, doc) in &self.documents {
                term_contains_all += doc.term_frequency.contains_key(term) as usize;
            }

            let idf =
                ((self.documents.len() as f64 + 1.0) / (term_contains_all as f64 + 1.0)).log10();

            for (_, doc) in &self.documents {
                if let Some(freq) = doc.term_frequency.get(term) {
                    eprintln!(
                        "freq: {freq}, idf: {idf}, title: {}, term: {term}",
                        doc.title
                    );
                    let score = (100000.0 * idf * freq) as u64;
                    documents
                        .entry(&doc.title)
                        .and_modify(|v| *v += score)
                        .or_insert(score);
                }
            }
        }

        let mut doc_list = vec![];
        for (title, tf_idf) in documents {
            let path = self.documents.get(title).unwrap().path.clone();
            doc_list.push((tf_idf / terms.len() as u64, path, title.to_owned()));
        }
        doc_list.sort_by(|a, b| b.cmp(a));
        doc_list
    }
}

pub fn drop_pdf(doc: PopplerDocument) {
    struct Layout (*mut u8);
    unsafe {
        gobject_sys::g_object_unref(std::mem::transmute::<_, Layout>(doc).0 as *mut gobject_sys::GObject);
    }
}

pub fn drop_page(page: PopplerPage) {
    struct Layout (*mut u8);
    unsafe {
        gobject_sys::g_object_unref(std::mem::transmute::<_, Layout>(page).0 as *mut gobject_sys::GObject);
    }
}

async fn submit_document(
    Query(params): Query<HashMap<String, String>>,
    State(docs): State<DocShared>,
) -> Result<(), String> {
    let path = params
        .get("path")
        .ok_or_else(|| log("Missing `path` parameter; give path to document"))?;
    eprintln!("Submitting document... \"{path}\"");

    if !path::Path::new(path).is_file() {
        return Err(log(format!("{path:?} is not a file")));
    }

    let pdf = PopplerDocument::new_from_file(path, None)
        .map_err(|e| log(format!("Could not open file: {path:?}: {e}")))?;
    let mut title = pdf.get_title().unwrap_or(path.to_string());
    if title.is_empty() {
        title = path.to_string();
    }
    {
        let mut docs = docs
            .write()
            .map_err(|e| log(format!("Could not take `DocShared` lock: {e}")))?;
        if let Some(doc) = docs.documents.get(&title) {
            let s = params.get("dupe");
            let s = s.map(|v| v.as_str());
            match s {
                Some("replace") => {
                    // TODO: Need to update counts
                    docs.documents.remove(&title);
                    log(format!("Removing title... {title:?}"));
                }
                Some("rename") => {
                    // TODO: This can collide
                    title = format!("{title}-1");
                }
                Some("ignore") => {
                    drop_pdf(pdf);
                    return Ok(());
                }
                _ => {
                    let err_msg = format!(
                        r#"Found document with identical titles: {:?}: you submitted {:?}, but found {:?}; use query parameters "dupe={{replace,rename,ignore}}" to handle this"#,
                        title, path, doc.path
                    );
                    drop_pdf(pdf);
                    return Err(log(err_msg));
                }
            }
        }
    }

    let mut term_count = HashMap::new();
    {
        let mut docs = docs
            .write()
            .map_err(|e| log(format!("Could not take `DocShared` lock: {e}")))?;
        for page in pdf.pages() {
            if let Some(text) = page.get_text() {
                for word in text.split_whitespace() {
                    let word = word.to_lowercase();
                    let word = STEMMER.stem(&word);
                    let id = intern(word);
                    term_count
                        .entry(id)
                        .and_modify(|v| *v += 1)
                        .or_insert(1);
                    docs.global_term_count
                        .entry(id)
                        .and_modify(|v| *v += 1)
                        .or_insert(1);
                }
            }
            drop_page(page);
        }
    }

    drop_pdf(pdf);
    let mut term_frequency = HashMap::new();
    for (term, n) in &term_count {
        assert!(term_frequency
            .insert(term.to_owned(), *n as f64 / term_count.len() as f64)
            .is_none());
    }

    let document = Document {
        path: path.to_string(),
        title: title.clone(),
        term_frequency,
    };

    {
        docs
            .write()
            .map_err(|e| log(format!("Could not take `DocShared` lock: {e}")))?
            .documents.insert(title, document);
    }
    Ok(())
}

pub async fn document_info(Path(_document_id): Path<u32>) -> Result<(), String> {
    todo!()
}

pub async fn search_document(
    Query(params): Query<HashMap<String, String>>,
    State(docs): State<DocShared>,
) -> Result<impl IntoResponse, String> {
    let terms = params
        .get("s")
        .ok_or_else(|| log("Missing `s` parameter; give search terms"))?;

    let docs = docs
        .read()
        .map_err(|e| log(format!("Could not get `DocShared` read lock: {e}")))?;
    let terms = terms.split_whitespace().map(|v| v.to_lowercase());
    let terms = terms
        .map(|v| intern(STEMMER.stem(&v)))
        .collect::<Vec<PoolId>>();
    return Ok(Json(docs.sort_documents(&terms)));
}

async fn shutdown(docs: DocShared) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    let mut f = match std::fs::File::create(CACHE_PATH) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to create file: {e}");
            return;
        }
    };
    match docs.read() {
        Ok(v) => {
            v.serialize(&mut f).map_err(|e| eprintln!("{e}")).ok();
        }
        Err(e) => {
            eprintln!("Could not get read lock to serialize `DocShared`: {e}");
            return;
        }
    }
    log("Successfully wrote cache");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tf_idf = match std::fs::File::open(CACHE_PATH) {
        Ok(mut f) => {
            // TODO: Buffer this in small chunks to be able to handle larger files
            //
            // But honestly, at that point just use a database
            let mut data = vec![];
            f.read_to_end(&mut data)?;
            TfIdf::deserialize(&data)?
        }
        _ => TfIdf::default(),
    };
    let docs: DocShared = Arc::new(RwLock::new(tf_idf));
    let docs_resource = Arc::clone(&docs);
    let document_routes = Router::new()
        .route("/submit", get(submit_document))
        .route("/search", get(search_document))
        .with_state(docs_resource);

    let api_routes = Router::new().nest("/document", document_routes);

    let app = Router::new()
        .route_service("/", ServeFile::new("src/index.html"))
        .nest("/api", api_routes);

    let addr = "127.0.0.1:42069";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("Now serving at: {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown(docs))
        .await?;
    Ok(())
}
