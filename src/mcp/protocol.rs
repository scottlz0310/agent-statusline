//! Codex CLI Custom status line 向け JSON-RPC 2.0 over stdio (改行区切り) プロトコル層。
//!
//! 契約根拠: `openai/codex` `rust-v0.171.0`
//! - `codex-rs/skills/src/assets/samples/create-statusline/assets/statusline.py`
//! - `codex-rs/tui/src/statusline_provider/descriptor.rs`
//! - `codex-rs/tui/src/chatwidget/statusline_reader/resource.rs`

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use crate::engine::ui_tree::UiTree;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_NAME: &str = "agent-statusline";
pub const TOOL_NAME: &str = "statusline";
pub const TOOL_TITLE: &str = "agent-statusline";
pub const RESOURCE_NAME: &str = "footer";
pub const RESOURCE_URI: &str = "ui://agent-statusline/footer";

/// 異常入力によるメモリ枯渇を防ぐ 1 行あたりの上限（4 MiB）
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

const ERR_PARSE: i64 = -32700;
const ERR_INVALID_REQUEST: i64 = -32600;
const ERR_METHOD_NOT_FOUND: i64 = -32601;
const ERR_INVALID_PARAMS: i64 = -32602;

/// `resources/read` で渡されるスナップショットから `UiTree` を生成するレンダラー境界（DI 用）
pub trait StatuslineRenderer {
    fn render_snapshot(&mut self, snapshot: &Value) -> UiTree;
}

pub struct CodexMcpHandler<R> {
    renderer: R,
}

impl<R: StatuslineRenderer> CodexMcpHandler<R> {
    pub const fn new(renderer: R) -> Self {
        Self { renderer }
    }

    /// JSON-RPC 2.0 の 1 行を処理し、返信 JSON 文字列を返す。
    /// 通知（`id` フィールドなし）または空行では `None` を返す。
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        let reply = match serde_json::from_str::<Value>(trimmed) {
            Err(_) => error_reply(&Value::Null, ERR_PARSE, "Parse error"),
            Ok(req) => self.handle_value(&req)?,
        };

        Some(serde_json::to_string(&reply).unwrap_or_else(|_| {
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error"}}"#
                .to_string()
        }))
    }

    fn handle_value(&mut self, req: &Value) -> Option<Value> {
        let Some(obj) = req.as_object() else {
            return Some(error_reply(
                &Value::Null,
                ERR_INVALID_REQUEST,
                "Invalid request",
            ));
        };
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return Some(error_reply(
                &Value::Null,
                ERR_INVALID_REQUEST,
                "Invalid request",
            ));
        };
        // JSON-RPC 2.0 通知（`id` キーなし）には応答しない
        let id = obj.get("id")?;
        let params = obj.get("params");

        match self.dispatch(method, params) {
            Ok(result) => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            })),
            Err((code, message)) => Some(error_reply(id, code, message)),
        }
    }

    fn dispatch(
        &mut self,
        method: &str,
        params: Option<&Value>,
    ) -> Result<Value, (i64, &'static str)> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {},
                    "resources": {}
                },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": [tool_descriptor()]
            })),
            "tools/call" => {
                let name = params
                    .and_then(Value::as_object)
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str);
                if name != Some(TOOL_NAME) {
                    return Err((ERR_INVALID_PARAMS, "Unknown tool name"));
                }
                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("The statusline resource is {RESOURCE_URI}.")
                    }]
                }))
            }
            "resources/list" => Ok(json!({
                "resources": [{
                    "name": RESOURCE_NAME,
                    "uri": RESOURCE_URI,
                    "mimeType": "application/json"
                }]
            })),
            "resources/templates/list" => Ok(json!({
                "resourceTemplates": []
            })),
            "resources/read" => {
                let Some(params_obj) = params.and_then(Value::as_object) else {
                    return Err((ERR_INVALID_PARAMS, "Invalid parameters"));
                };
                if params_obj.get("uri").and_then(Value::as_str) != Some(RESOURCE_URI) {
                    return Err((ERR_INVALID_PARAMS, "Unknown resource URI"));
                }
                let empty = Value::Object(serde_json::Map::new());
                let snapshot = params_obj
                    .get("_meta")
                    .and_then(Value::as_object)
                    .and_then(|meta| meta.get("com.openai/codex-statusline-state"))
                    .unwrap_or(&empty);
                let tree = self.renderer.render_snapshot(snapshot);
                let text = tree.to_json_string();
                Ok(json!({
                    "contents": [{
                        "uri": RESOURCE_URI,
                        "mimeType": "application/json",
                        "text": text
                    }]
                }))
            }
            _ => Err((ERR_METHOD_NOT_FOUND, "Method not found")),
        }
    }
}

fn tool_descriptor() -> Value {
    json!({
        "name": TOOL_NAME,
        "title": TOOL_TITLE,
        "description": "Render customizable statusline for Codex CLI.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        },
        "_meta": {
            "openai/ui": {
                "entrypoints": [{
                    "type": "statusline",
                    "resourceUri": RESOURCE_URI
                }]
            }
        }
    })
}

fn error_reply(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

enum ReadLineOutcome {
    Eof,
    Line(String),
    TooLong,
}

/// 上限バイト数付きで 1 行読み取る。上限超過時は次の改行まで読み捨てて `TooLong` を返す。
fn read_bounded_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<ReadLineOutcome> {
    let mut buf = Vec::new();
    let mut overflowed = false;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if overflowed {
                return Ok(ReadLineOutcome::TooLong);
            }
            if buf.is_empty() {
                return Ok(ReadLineOutcome::Eof);
            }
            return Ok(ReadLineOutcome::Line(
                String::from_utf8_lossy(&buf).into_owned(),
            ));
        }

        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            if !overflowed {
                if buf.len() + pos <= max_bytes {
                    buf.extend_from_slice(&available[..pos]);
                } else {
                    overflowed = true;
                }
            }
            reader.consume(pos + 1);
            if overflowed {
                return Ok(ReadLineOutcome::TooLong);
            }
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(ReadLineOutcome::Line(
                String::from_utf8_lossy(&buf).into_owned(),
            ));
        }

        let len = available.len();
        if !overflowed {
            if buf.len() + len <= max_bytes {
                buf.extend_from_slice(available);
            } else {
                overflowed = true;
                buf.clear();
            }
        }
        reader.consume(len);
    }
}

/// 改行区切り JSON-RPC 2.0 over stdio ループを実行する（I/O を DI で差し替え可能）
pub fn serve_stdio<I: BufRead, O: Write, R: StatuslineRenderer>(
    mut reader: I,
    mut writer: O,
    handler: &mut CodexMcpHandler<R>,
) -> io::Result<()> {
    serve_stdio_with_limit(&mut reader, &mut writer, handler, MAX_LINE_BYTES)
}

fn serve_stdio_with_limit<I: BufRead, O: Write, R: StatuslineRenderer>(
    reader: &mut I,
    writer: &mut O,
    handler: &mut CodexMcpHandler<R>,
    max_line_bytes: usize,
) -> io::Result<()> {
    loop {
        match read_bounded_line(reader, max_line_bytes)? {
            ReadLineOutcome::Eof => return Ok(()),
            ReadLineOutcome::TooLong => {
                let reply = error_reply(&Value::Null, ERR_PARSE, "Request line exceeds size limit");
                writeln!(writer, "{reply}")?;
                writer.flush()?;
            }
            ReadLineOutcome::Line(line) => {
                if let Some(reply) = handler.handle_line(&line) {
                    writeln!(writer, "{reply}")?;
                    writer.flush()?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::engine::ui_tree::UiNode;

    struct FakeRenderer {
        calls: usize,
    }

    impl StatuslineRenderer for FakeRenderer {
        fn render_snapshot(&mut self, snapshot: &Value) -> UiTree {
            self.calls += 1;
            let model = snapshot
                .pointer("/thread/model/display_name")
                .and_then(Value::as_str)
                .unwrap_or("none");
            UiTree(Some(UiNode::String(format!("{model}#{}", self.calls))))
        }
    }

    #[test]
    fn test_full_mcp_handshake_and_resource_read() {
        let input = [
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"statusline"}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"resources/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"resources/templates/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"resources/read","params":{"uri":"ui://agent-statusline/footer","_meta":{"com.openai/codex-statusline-state":{"thread":{"model":{"display_name":"GPT-6.2-Sol"}}}}}}"#,
        ]
        .join("\n")
            + "\n";

        let mut handler = CodexMcpHandler::new(FakeRenderer { calls: 0 });
        let mut out = Vec::new();
        serve_stdio(Cursor::new(input), &mut out, &mut handler).unwrap();

        let text = String::from_utf8(out).unwrap();
        let responses: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        // notifications/initialized は無応答なので全 7 応答
        assert_eq!(responses.len(), 7);
        assert_eq!(responses[0]["id"], 0);
        assert_eq!(responses[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(responses[1]["id"], 1);
        assert_eq!(responses[1]["result"], json!({}));

        // tools/list がちょうど 1 つの有効な statusline entrypoint を広告すること
        let tools = responses[2]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], TOOL_NAME);
        assert_eq!(tools[0]["title"], TOOL_TITLE);
        let entrypoints = tools[0]["_meta"]["openai/ui"]["entrypoints"]
            .as_array()
            .unwrap();
        assert_eq!(entrypoints.len(), 1);
        assert_eq!(entrypoints[0]["type"], "statusline");
        assert_eq!(entrypoints[0]["resourceUri"], RESOURCE_URI);

        // tools/call, resources/list, resources/templates/list
        assert!(
            responses[3]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(RESOURCE_URI)
        );
        assert_eq!(responses[4]["result"]["resources"][0]["uri"], RESOURCE_URI);
        assert_eq!(responses[5]["result"]["resourceTemplates"], json!([]));

        // resources/read: ちょうど 1 つの text resource、URI 一致、application/json
        let contents = responses[6]["result"]["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["uri"], RESOURCE_URI);
        assert_eq!(contents[0]["mimeType"], "application/json");
        assert_eq!(contents[0]["text"], "\"GPT-6.2-Sol#1\"");
    }

    #[test]
    fn test_protocol_error_codes() {
        let cases = [
            ("parse-error", "{broken", ERR_PARSE),
            ("not-object", "[1,2,3]", ERR_INVALID_REQUEST),
            (
                "missing-method",
                r#"{"jsonrpc":"2.0","id":1}"#,
                ERR_INVALID_REQUEST,
            ),
            (
                "unknown-method",
                r#"{"jsonrpc":"2.0","id":2,"method":"unknown/x"}"#,
                ERR_METHOD_NOT_FOUND,
            ),
            (
                "unknown-tool",
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"other"}}"#,
                ERR_INVALID_PARAMS,
            ),
            (
                "unknown-uri",
                r#"{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"ui://other/footer"}}"#,
                ERR_INVALID_PARAMS,
            ),
            (
                "missing-params",
                r#"{"jsonrpc":"2.0","id":5,"method":"resources/read"}"#,
                ERR_INVALID_PARAMS,
            ),
        ];

        let mut handler = CodexMcpHandler::new(FakeRenderer { calls: 0 });
        for (name, line, expected_code) in cases {
            let reply_str = handler.handle_line(line).expect(name);
            let val: Value = serde_json::from_str(&reply_str).unwrap();
            assert_eq!(val["error"]["code"], expected_code, "{name}");
        }
    }

    #[test]
    fn test_oversized_line_returns_parse_error_and_continues() {
        let input = format!(
            "{}\n{}\n",
            "x".repeat(128),
            r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#
        );
        let mut handler = CodexMcpHandler::new(FakeRenderer { calls: 0 });
        let mut out = Vec::new();
        serve_stdio_with_limit(&mut Cursor::new(input), &mut out, &mut handler, 64).unwrap();

        let text = String::from_utf8(out).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["error"]["code"], ERR_PARSE);
        assert_eq!(lines[1]["id"], 9);
        assert_eq!(lines[1]["result"], json!({}));
    }
}
