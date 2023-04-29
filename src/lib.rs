mod handler;

use std::{convert::Infallible, sync::Mutex, thread, time::Instant};

use aw_client_rust::{AwClient, Event};
use chrono::{Duration, Utc};
use gethostname::gethostname;
use handler::{start_event_handler, HandlerConfig};
use lazy_static::{__Deref, lazy_static};
use nvim_oxi as oxi;
use oxi::api::opts::CreateCommandOpts;
use oxi::{api::opts::CreateAutocmdOpts, api::types::AutocmdCallbackArgs, libuv::AsyncHandle};
use tokio::sync::mpsc::{self, UnboundedSender};

// make anyhow errors usable as std::error:Error (required by the oxi::module macro)
#[derive(thiserror::Error, Debug)]
#[error(transparent)]
struct Error(#[from] anyhow::Error);

struct Globals {
    connected: bool,
    last_heartbeat: Instant,
    last_file: String,
    last_project: String,
    last_language: String,
}

impl Default for Globals {
    fn default() -> Self {
        Globals {
            connected: false,
            last_heartbeat: Instant::now(),
            last_file: "".to_owned(),
            last_project: "".to_owned(),
            last_language: "".to_owned(),
        }
    }
}

lazy_static! {
    static ref GLOBALS: Mutex<Globals> = Mutex::new(Globals::default());
    static ref AW_BUCKET_NAME: String = format!(
        "aw-watcher-nvim_{}",
        gethostname().to_str().unwrap_or("unknown")
    );
    static ref AW_CLIENT: AwClient = AwClient::new("localhost", "5666", "aw-watcher-nvim");
}

#[oxi::module]
fn aw_watcher_nvim() -> anyhow::Result<(), crate::Error> {
    entry()?;
    Ok(())
}

// In the above function the ? operator tries to turn errors into crate::Error which only works
// with anyhow::Error. In this function the ? operator turns everything into an anyhow::Error.
fn entry() -> anyhow::Result<()> {
    let (error_tx, mut error_rx) = mpsc::unbounded_channel();

    let handle = AsyncHandle::new(move || {
        while let Some(err) = error_rx.blocking_recv() {
            oxi::schedule(move |_| {
                oxi::print!("{err}");
                Ok(())
            });
            GLOBALS.lock().unwrap().connected = false;
        }
        Ok::<(), Infallible>(())
    })?;

    let event_tx = start_event_handler(error_tx, handle);

    setup_vim_enter()?;
    setup_heartbeat_sources(event_tx)?;
    setup_start_command()?;
    setup_stop_command()?;
    setup_status_command()?;

    Ok(())
}

fn setup_vim_enter() -> anyhow::Result<u32> {
    let opts = CreateAutocmdOpts::builder()
        .callback(|_| start_watcher().map(|_| false))
        .build();

    Ok(oxi::api::create_autocmd(vec!["VimEnter"], &opts)?)
}

fn start_watcher() -> anyhow::Result<(), oxi::api::Error> {
    let result = AW_CLIENT
        .create_bucket_simple(AW_BUCKET_NAME.deref(), "app.editor.activity")
        .map_err(|err| nvim_oxi::api::Error::Other(err.to_string()));

    // if there was no error, we are connected
    let connected = result.is_ok();
    thread::spawn(move || {
        // need to be in another thread to avoid panic when calling lock
        // unwrap is safe as long as we never panic somewhere else I guess
        GLOBALS.lock().unwrap().connected = connected;
    });

    result
}

fn setup_heartbeat_sources(tx: UnboundedSender<(Event, HandlerConfig)>) -> oxi::Result<u32> {
    let opts = CreateAutocmdOpts::builder()
        .callback(move |args| trigger_heartbeat(args, tx.clone()))
        .build();

    Ok(oxi::api::create_autocmd(
        vec![
            "BufEnter",
            "CursorMoved",
            "CursorMovedI",
            "CmdlineEnter",
            "CmdlineChanged",
        ],
        &opts,
    )?)
}

fn trigger_heartbeat(
    _: AutocmdCallbackArgs,
    tx: UnboundedSender<(Event, HandlerConfig)>,
) -> oxi::Result<bool> {
    let mut globals = if let Ok(globals) = GLOBALS.lock() {
        globals
    } else {
        return Ok(false);
    };

    if !globals.connected {
        return Ok(false);
    }

    let now = Instant::now();

    // only run heartbeat if more than a second passed since last one
    // this has to be at the top, because even the calculations below are too slow otherwise
    if now.saturating_duration_since(globals.last_heartbeat) <= std::time::Duration::from_secs(1) {
        return Ok(false);
    }

    globals.last_heartbeat = now;

    let file = oxi::api::get_current_buf()
        .get_name()?
        .to_str()
        .ok_or_else(|| {
            oxi::api::Error::Other(
                "error when converting current buffer's path into a string representation"
                    .to_owned(),
            )
        })?
        .to_owned();

    let project = std::env::current_dir()
        .map_err(|err| oxi::api::Error::Other(err.to_string()))?
        .to_str()
        .ok_or_else(|| {
            oxi::api::Error::Other(
                "error when converting current working directory into a string representation"
                    .to_owned(),
            )
        })?
        .to_owned();

    let language: String = oxi::api::get_current_buf().get_option("filetype")?;

    let data_unchanged = file == globals.last_file
        && language == globals.last_language
        && project == globals.last_project;

    if data_unchanged {
        return Ok(false);
    }

    globals.last_file = file.clone();
    globals.last_project = project.clone();
    globals.last_language = language.clone();

    let event = Event {
        id: None,
        timestamp: Utc::now(),
        duration: Duration::seconds(0),
        data: {
            let mut data = serde_json::Map::new();
            data.extend(vec![
                ("file".to_owned(), serde_json::Value::String(file)),
                ("project".to_owned(), serde_json::Value::String(project)),
                ("language".to_owned(), serde_json::Value::String(language)),
            ]);
            data
        },
    };

    tx.send((
        event,
        HandlerConfig::new(&*AW_CLIENT, &*AW_BUCKET_NAME, 30_f64),
    ))
    .map_err(|err| nvim_oxi::api::Error::Other(err.to_string()))?;

    Ok(false)
}

fn setup_start_command() -> oxi::Result<()> {
    Ok(oxi::api::create_user_command(
        "AWStart",
        |_| start_watcher(),
        &CreateCommandOpts::default(),
    )?)
}

fn setup_stop_command() -> oxi::Result<()> {
    Ok(oxi::api::create_user_command(
        "AWStop",
        |_| {
            thread::spawn(|| GLOBALS.lock().unwrap().connected = false);
            Ok(())
        },
        &CreateCommandOpts::default(),
    )?)
}

fn setup_status_command() -> oxi::Result<()> {
    Ok(oxi::api::create_user_command(
        "AWStatus",
        |_| {
            println!(
                "aw-watcher-nvim running: {}",
                GLOBALS.lock().unwrap().connected
            );
            Ok(())
        },
        &CreateCommandOpts::default(),
    )?)
}
