//! todo-server: an MQTT broker and the todo service, in one process.
//!
//! ```text
//! todo-server [--listen ADDR] [--port N]     (default 127.0.0.1:1883)
//! ```
//!
//! The broker is rumqttd, speaking MQTT 3.1.1. The service is an ordinary MQTT
//! client (rumqttc) of that broker. It takes EXI commands from `todo/cmd`,
//! changes the list, and publishes the whole list as EXI to `todo/list` with the
//! retain flag set. A retained message is kept by the broker and handed to every
//! later subscriber at once, so a new client sees the current list without asking
//! ([MQTT 3.1.1 section 3.3.1.3](https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html#_Toc398718038)).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use rumqttd::{Broker, Config, ConnectionSettings, RouterConfig, ServerSettings};
use todo_model::{Action, CMD_TOPIC, Codec, Command, Error, Item, LIST_TOPIC, TodoList};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("todo-server: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    let mut ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut port: u16 = 1883;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => ip = args.next().ok_or("--listen needs an address")?.parse()?,
            "--port" => port = args.next().ok_or("--port needs a number")?.parse()?,
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let listen = SocketAddr::new(ip, port);

    let mut broker = Broker::new(broker_config(listen));
    thread::spawn(move || {
        if let Err(e) = broker.start() {
            eprintln!("todo-server: broker stopped: {e}");
            std::process::exit(1);
        }
    });

    serve(listen)
}

/// A single MQTT 3.1.1 listener with small limits; everything else is off.
fn broker_config(listen: SocketAddr) -> Config {
    let connections = ConnectionSettings {
        connection_timeout_ms: 60_000,
        max_payload_size: 64 * 1024,
        max_inflight_count: 100,
        auth: None,
        external_auth: None,
        dynamic_filters: true,
    };
    let server = ServerSettings {
        name: "v4".into(),
        listen,
        tls: None,
        next_connection_delay_ms: 1,
        connections,
    };
    Config {
        router: RouterConfig {
            max_connections: 100,
            max_outgoing_packet_count: 200,
            max_segment_size: 1024 * 1024,
            max_segment_count: 10,
            ..Default::default()
        },
        v4: Some(HashMap::from([("1".to_string(), server)])),
        ..Default::default()
    }
}

/// The todo service: the list lives here, and only here.
struct Service {
    codec: Codec,
    list: TodoList,
    next_id: u32,
}

impl Service {
    /// Applies one command. Returns false when it changed nothing (an unknown id).
    fn apply(&mut self, cmd: &Command) -> bool {
        match &cmd.action {
            Action::Add { title } => {
                self.list.items.push(Item {
                    id: self.next_id,
                    done: false,
                    title: title.clone(),
                });
                self.next_id += 1;
                true
            }
            Action::Done { id } => match self.list.items.iter_mut().find(|i| i.id == *id) {
                Some(item) => {
                    item.done = true;
                    true
                }
                None => false,
            },
            Action::Remove { id } => {
                let before = self.list.items.len();
                self.list.items.retain(|i| i.id != *id);
                self.list.items.len() != before
            }
        }
    }

    fn publish(&self, client: &Client) -> Result<(), Error> {
        let bytes = self.codec.encode(&self.list)?;
        client.publish(LIST_TOPIC, QoS::AtLeastOnce, true, bytes)?;
        Ok(())
    }
}

fn serve(listen: SocketAddr) -> Result<(), Error> {
    let mut service = Service {
        codec: Codec::new()?,
        list: TodoList::default(),
        next_id: 1,
    };

    let host = if listen.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else {
        listen.ip().to_string()
    };
    let mut options = MqttOptions::new("todo-service", host, listen.port());
    options.set_keep_alive(Duration::from_secs(30));
    let (client, mut connection) = Client::new(options, 16);
    let mut failures = 0;

    for event in connection.iter() {
        match event {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                failures = 0;
                // (Re)connected: subscribe, then publish the list so the retained
                // copy always exists, even before the first change.
                client.subscribe(CMD_TOPIC, QoS::AtLeastOnce)?;
                service.publish(&client)?;
            }
            Ok(Event::Incoming(Packet::SubAck(_))) => {
                println!("todo-server: listening on {listen}");
            }
            Ok(Event::Incoming(Packet::Publish(p))) if p.topic == CMD_TOPIC => {
                match service.codec.decode::<Command>(&p.payload) {
                    Ok(cmd) => {
                        let changed = service.apply(&cmd);
                        println!(
                            "todo-server: {:?} from {}{}",
                            cmd.action,
                            cmd.from,
                            if changed { "" } else { " (no such item)" }
                        );
                        if changed {
                            service.list.revision += 1;
                        }
                        // Publish even when nothing changed, so the sender gets an
                        // answer; the revision tells it whether anything happened.
                        service.publish(&client)?;
                    }
                    Err(e) => {
                        eprintln!("todo-server: bad command ({} bytes): {e}", p.payload.len())
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                // The broker thread may still be starting: wait and retry. Say so
                // only when it takes longer than a moment.
                failures += 1;
                if failures > 5 {
                    eprintln!("todo-server: waiting for the broker: {e}");
                }
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
    Ok(())
}
