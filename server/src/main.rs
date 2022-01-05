use std::{error::Error, sync::Arc, time::Duration};

use futures::{channel::oneshot, SinkExt, StreamExt};
use linefeed::{Interface, ReadResult};
use log::error;
use warp::{fs, ws::Ws, Filter};

mod logging;
mod packets;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let console_interface = Arc::new(Interface::new("SAP")?);
    console_interface.set_prompt("> ")?;
    logging::init_logger(console_interface.clone())?;

    let server_shutdown = start_server();

    loop {
        match console_interface.read_line_step(Some(Duration::from_millis(50))) {
            Ok(result) => match result {
                Some(ReadResult::Input(command)) =>
                    if command.to_ascii_lowercase() == "stop" {
                        break;
                    },
                _ => {}
            },
            Err(e) => error!("Error reading console input: {}", e),
        }
    }

    server_shutdown.send(()).unwrap();
    logging::cleanup();
    println!();

    Ok(())
}

fn start_server() -> oneshot::Sender<()> {
    let websocket = warp::path("ws")
        .and(warp::ws())
        .and(warp::addr::remote())
        .map(|ws: Ws, _address| {
            ws.on_upgrade(move |socket| async {
                let (mut ws_tx, mut ws_rx) = socket.split();

                while let Some(result) = ws_rx.next().await {
                    let message = match result {
                        Ok(message) => message,
                        Err(e) => {
                            error!("WS error {}", e);
                            break;
                        }
                    };

                    log::debug!("{:?}", message);
                    ws_tx.send(message).await.unwrap();
                    ws_tx.flush().await.unwrap();
                }
            })
        });

    let html_hosting = fs::dir("client/out")
        .or(fs::dir("client/html/"))
        .or(warp::path::end().and(fs::file("client/html/index.html")));

    let (shutdown_hook, rx) = oneshot::channel::<()>();

    let (_addr, server) = warp::serve(html_hosting.or(websocket)).bind_with_graceful_shutdown(
        ([0, 0, 0, 0], 8080),
        async {
            rx.await.ok();
        },
    );

    tokio::task::spawn(server);
    shutdown_hook
}
