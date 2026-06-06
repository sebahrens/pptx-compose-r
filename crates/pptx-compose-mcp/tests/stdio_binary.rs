use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

use serde_json::{Value, json};

#[test]
fn mcp_binary_completes_stdio_initialize_and_lists_tools() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pptx-compose-mcp"))
        .arg("--workspace")
        .arg(workspace_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mcp binary");

    let mut stdin = child.stdin.take().expect("child stdin is piped");
    let stdout = child.stdout.take().expect("child stdout is piped");
    let mut stdout = BufReader::new(stdout);

    write_message(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "pptx-compose-test-client",
                    "version": "0.1.0"
                }
            }
        }),
    );

    let initialize = read_message(&mut stdout);
    assert_eq!(initialize["id"], 1);
    assert_eq!(
        initialize["result"]["serverInfo"]["name"],
        "pptx-compose-mcp"
    );

    write_message(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    );
    write_message(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );

    let tools = read_message(&mut stdout);
    assert_eq!(tools["id"], 2);
    assert!(
        tools["result"]["tools"]
            .as_array()
            .is_some_and(|tools| { tools.iter().any(|tool| tool["name"] == "pptx_open") })
    );

    drop(stdin);
    child.kill().expect("terminate mcp binary");
    let _ = child.wait();
}

fn write_message(stdin: &mut impl Write, message: Value) {
    serde_json::to_writer(&mut *stdin, &message).expect("serialize json-rpc message");
    stdin.write_all(b"\n").expect("write json-rpc newline");
    stdin.flush().expect("flush json-rpc message");
}

fn read_message(stdout: &mut impl BufRead) -> Value {
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read json-rpc line");
    assert!(
        !line.is_empty(),
        "mcp server closed stdout before responding"
    );
    serde_json::from_str(&line).expect("parse json-rpc response")
}

fn workspace_root() -> String {
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .ancestors()
        .nth(2)
        .expect("workspace root is two levels above mcp crate")
        .display()
        .to_string()
}
