//! todo-rust-client: the todo list from the command line, in Rust.
//!
//! ```text
//! todo-rust-client [--host H] [--port N] list
//! todo-rust-client [--host H] [--port N] add <title> | done <id> | remove <id>
//! ```
//!
//! Every client works the same way:
//! 1. subscribe to `todo/list`; the broker sends the retained list at once;
//! 2. for `list`, print it and stop;
//! 3. otherwise publish one EXI `<command>` to `todo/cmd`, wait for the next list
//!    the server publishes, print it and stop.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use todo_model::{Action, CMD_TOPIC, Codec, Command, Error, LIST_TOPIC, TodoList, format_list};

const TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("todo-rust-client: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 1883;
    let mut rest = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => host = args.next().ok_or("--host needs a name")?,
            "--port" => port = args.next().ok_or("--port needs a number")?.parse()?,
            _ => rest.push(arg),
        }
    }
    let id = |s: Option<&String>| -> Result<u32, Error> {
        Ok(s.ok_or("missing item id")?.parse::<u32>()?)
    };
    let action = match rest.first().map(String::as_str) {
        Some("list") => None,
        Some("add") => Some(Action::Add {
            title: rest.get(1).ok_or("missing title")?.clone(),
        }),
        Some("done") => Some(Action::Done { id: id(rest.get(1))? }),
        Some("remove") => Some(Action::Remove { id: id(rest.get(1))? }),
        _ => return Err("usage: todo-rust-client [--host H] [--port N] list | add <title> | done <id> | remove <id>".into()),
    };

    let codec = Codec::new()?;
    let client_id = format!("todo-rust-{}", std::process::id());
    let (client, mut connection) = Client::new(MqttOptions::new(client_id, host, port), 16);
    client.subscribe(LIST_TOPIC, QoS::AtLeastOnce)?;

    // Lists seen so far: the first is the retained one, the second the answer.
    let mut seen = 0;
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        let event = match connection.recv_timeout(deadline - Instant::now()) {
            Ok(event) => event?,
            Err(_) => break,
        };
        let Event::Incoming(Packet::Publish(p)) = event else {
            continue;
        };
        if p.topic != LIST_TOPIC {
            continue;
        }
        let list: TodoList = codec.decode(&p.payload)?;
        seen += 1;
        match (&action, seen) {
            (None, _) | (Some(_), 2) => {
                print!("{}", format_list(&list));
                client.disconnect()?;
                return Ok(());
            }
            (Some(action), _) => {
                let cmd = Command {
                    from: "rust".into(),
                    action: action.clone(),
                };
                client.publish(CMD_TOPIC, QoS::AtLeastOnce, false, codec.encode(&cmd)?)?;
            }
        }
    }
    Err("timed out waiting for todo/list".into())
}
