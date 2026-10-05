#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use phoenix_grid::{
        protocol::{GridProtocol, Input, Output},
        Grid,
    };
    use phoenix_runtime::HostSlot;
    use phoenix_transport::{
        relay::{RelayHostConfig, RelayNotice, RelayTransport},
        relay_socket::WsRelaySocket,
    };
    use phoenix_transport::{DeliveryClass, Dispatch, Event, Target, Transport};
    let args: Vec<_> = std::env::args().collect();
    let base = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("http://127.0.0.1:8788");
    let origin = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("http://127.0.0.1:8000");
    let save = args.get(3).map(std::path::PathBuf::from);
    let mut grid = Grid::new(HostSlot(1), []);
    if let Some(path) = save.as_ref().filter(|p| p.exists()) {
        grid.restore(&std::fs::read_to_string(path)?)
            .map_err(std::io::Error::other)?;
    }
    let socket = WsRelaySocket::connect(base, origin)?;
    let mut transport = RelayTransport::<GridProtocol>::new(
        socket,
        RelayHostConfig {
            namespace: "client".into(),
            version: None,
            stamp: (),
        },
    );
    loop {
        for event in transport.poll() {
            if let Event::Received { token, msg } = event {
                match msg {
                    Input::Move(movement) => {
                        grid.submit(movement);
                    }
                    Input::Identify { .. } | Input::Recover => {
                        let message = Output::Recovery {
                            checkpoint: grid.checkpoint().map_err(std::io::Error::other)?,
                        };
                        transport.dispatch(Dispatch {
                            target: &Target::Token(token),
                            msg: &message,
                            delivery: DeliveryClass::Reliable,
                        });
                    }
                }
            }
        }
        grid.advance();
        transport.dispatch(Dispatch {
            target: &Target::All,
            msg: &Output::state(&grid),
            delivery: DeliveryClass::Snapshot,
        });
        for notice in transport.drain_notices() {
            if let RelayNotice::Coded(code) = notice {
                println!("Grid join code: {}", code.suffix);
            }
        }
        if let Some(path) = &save {
            phoenix_platform::native_file::replace(
                path,
                grid.checkpoint().map_err(std::io::Error::other)?.as_bytes(),
                phoenix_platform::native_file::TemporaryPolicy::Unique { prefix: ".grid-" },
            )?;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
#[cfg(target_arch = "wasm32")]
fn main() {}
