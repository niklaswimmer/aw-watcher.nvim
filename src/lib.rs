use std::{sync::Mutex, thread, time::Instant};

use aw_client_rust::{AwClient, Event};
use chrono::{Duration, Utc};
use gethostname::gethostname;
use lazy_static::{__Deref, lazy_static};
use nvim_oxi as oxi;
use oxi::{opts::CreateAutocmdOpts, types::AutocmdCallbackArgs};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

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
fn aw_watcher_nvim() -> oxi::Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<Event>();

    let _ = thread::spawn(move || handle_events(rx));

    setup_vim_enter()?;
    setup_heartbeat_sources(&tx)?;
    setup_start_command()?;
    setup_stop_command()?;
    setup_status_command()?;

    Ok(())
}

fn setup_vim_enter() -> oxi::Result<u32> {
    let opts = CreateAutocmdOpts::builder()
        .callback(|_| start_watcher().map(|_| false))
        .build();

    oxi::api::create_autocmd(vec!["VimEnter"], &opts)
}

fn start_watcher() -> oxi::Result<()> {
    let result = AW_CLIENT
        .create_bucket_simple(AW_BUCKET_NAME.deref(), "app.editor.activity")
        .map_err(|err| nvim_oxi::Error::Other(err.to_string()));

    // if there was no error, we are connected
    let connected = result.is_ok();
    thread::spawn(move || {
        // need to be in another thread to avoid panic when calling lock
        // unwrap is safe as long as we never panic somewhere else I guess
        GLOBALS.lock().unwrap().connected = connected;
    });

    result
}

fn setup_heartbeat_sources(tx: &UnboundedSender<Event>) -> oxi::Result<u32> {
    let tx = tx.clone();
    let opts = CreateAutocmdOpts::builder()
        .callback(move |args| trigger_heartbeat(args, tx.clone()))
        .build();

    oxi::api::create_autocmd(
        vec![
            "BufEnter",
            "CursorMoved",
            "CursorMovedI",
            "CmdlineEnter",
            "CmdlineChanged",
        ],
        &opts,
    )
}

fn trigger_heartbeat(_: AutocmdCallbackArgs, tx: UnboundedSender<Event>) -> oxi::Result<bool> {
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
            oxi::Error::Other(
                "error when converting current buffer's path into a string representation"
                    .to_owned(),
            )
        })?
        .to_owned();

    let project = std::env::current_dir()
        .map_err(|err| oxi::Error::Other(err.to_string()))?
        .to_str()
        .ok_or_else(|| {
            oxi::Error::Other(
                "error when converting current working directory into a string representation"
                    .to_owned(),
            )
        })?
        .to_owned();

    let language: String = oxi::api::get_current_buf().get_option("filetype")?;

    let data_unchanged = file == globals.last_file
        && language == globals.last_language
        && project == globals.last_project;


    if data_unchanged
    {
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

    tx.send(event)
        .map_err(|err| nvim_oxi::Error::Other(err.to_string()))?;

    Ok(false)
}

fn setup_start_command() -> oxi::Result<()> {
    oxi::api::create_user_command("AWStart", |_| start_watcher(), None)
}

fn setup_stop_command() -> oxi::Result<()> {
    oxi::api::create_user_command(
        "AWStop",
        |_| {
            thread::spawn(|| GLOBALS.lock().unwrap().connected = false);
            Ok(())
        },
        None,
    )
}

fn setup_status_command() -> oxi::Result<()> {
    oxi::api::create_user_command(
        "AWStatus",
        |_| {
            println!(
                "aw-watcher-nvim running: {}",
                GLOBALS.lock().unwrap().connected
            );
            Ok(())
        },
        None,
    )
}

#[tokio::main(flavor = "current_thread")]
async fn handle_events(mut rx: UnboundedReceiver<Event>) {
    loop {
        if let Some(event) = rx.recv().await {
            let result = AW_CLIENT.heartbeat(AW_BUCKET_NAME.deref().as_ref(), &event, 30.0);

            if result.is_err() {
                println!("{}", result.unwrap_err());
                GLOBALS.lock().unwrap().connected = false;
            }
        }
    }
}
