use aw_client_rust::{AwClient, Event};
use nvim_oxi::libuv::AsyncHandle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub(crate) struct HandlerConfig {
    client: &'static AwClient,
    bucketname: &'static str,
    pulsetime: f64,
}

impl HandlerConfig {
    pub(crate) fn new(client: &'static AwClient, bucketname: &'static str, pulsetime: f64) -> Self {
        HandlerConfig {
            client,
            bucketname,
            pulsetime,
        }
    }
}

pub(crate) fn start_event_handler(
    error_channel: UnboundedSender<reqwest::Error>,
    error_handle: AsyncHandle,
) -> UnboundedSender<(Event, HandlerConfig)> {
    let (tx, rx) = mpsc::unbounded_channel();

    let _detach = std::thread::spawn(|| {
        run_handler(rx, error_channel, error_handle);
    });

    tx
}

#[tokio::main(flavor = "current_thread")]
async fn run_handler(
    mut rx: UnboundedReceiver<(Event, HandlerConfig)>,
    error_channel: UnboundedSender<reqwest::Error>,
    error_handle: AsyncHandle,
) {
    while let Some((event, config)) = rx.recv().await {
        let result = config
            .client
            .heartbeat(config.bucketname, &event, config.pulsetime);

        if let Err(e) = result {
            if error_channel.send(e).is_err() {
                // receiving end got closed -> main thread exited
                break;
            } else {
                error_handle.send().expect("");
            }
        }
    }
}
