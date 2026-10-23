//! Recover malformed JSON lines before passing valid messages to rmcp.
use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::security::MAX_REQUEST_BYTES;

/// Read a complete request before allowing any of its bytes to reach rmcp. `lines()`
/// buffers without a limit, while forwarding partial input can make a valid JSON prefix
/// executable at EOF after an oversized suffix is rejected.
async fn bounded_line<R: tokio::io::AsyncRead + Unpin>(source: &mut BufReader<R>, maximum: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let available = source.fill_buf().await?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            return Ok(Some(bytes));
        }
        let newline = available.iter().position(|&byte| byte == b'\n');
        let length = newline.map_or(available.len(), |at| at + 1);
        if length > maximum.saturating_sub(bytes.len()) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("request exceeds {maximum} bytes")));
        }
        bytes.extend_from_slice(&available[..length]);
        source.consume(length);
        if newline.is_some() {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return Ok(Some(bytes));
        }
    }
}

impl PhotocraftMcp {
    /// Serve a JSON-lines stream. A malformed line receives -32700/id:null; the next line
    /// is still handled. A single writer serializes parse errors and SDK responses.
    pub async fn serve_io(
        self,
        input: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        output: impl tokio::io::AsyncWrite + Unpin + Send + 'static,
    ) -> Result<(), AutomationError> {
        let (mut to_service, service_in) = tokio::io::duplex(1 << 20);
        let (service_out, from_service) = tokio::io::duplex(1 << 20);
        let (errors, mut error_rx) = tokio::sync::mpsc::channel::<String>(16);
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(input);
            while let Some(line) = bounded_line(&mut lines, MAX_REQUEST_BYTES).await? {
                if std::str::from_utf8(&line).is_ok_and(|text| text.trim().is_empty()) {
                    continue;
                }
                // Bytes that are not UTF-8 are malformed JSON too: they get the -32700 reply
                // like any other bad line instead of ending the session.
                match serde_json::from_slice::<Value>(&line) {
                    Ok(_) => {
                        to_service.write_all(&line).await?;
                        to_service.write_all(b"\n").await?;
                    }
                    Err(e) => {
                        let reply = json!({"jsonrpc":"2.0", "id":null, "error":{"code":-32700, "message":format!("parse error: {e}")}});
                        if errors.send(reply.to_string()).await.is_err() {
                            break;
                        }
                    }
                }
            }
            to_service.shutdown().await
        });
        let writer = tokio::spawn(async move {
            let mut output = output;
            let mut replies = BufReader::new(from_service).lines();
            loop {
                let line = tokio::select! {
                    biased;
                    Some(error) = error_rx.recv() => error,
                    reply = replies.next_line() => match reply? { Some(line) => line, None => break },
                };
                output.write_all(format!("{line}\n").as_bytes()).await?;
                output.flush().await?;
            }
            Ok::<_, std::io::Error>(())
        });
        let result = match self.serve((service_in, service_out)).await {
            Ok(running) => running.waiting().await.map(|_| ()).map_err(|e| AutomationError::Other(e.to_string())),
            Err(e) => Err(AutomationError::Other(format!("MCP init: {e}"))),
        };
        reader.abort();
        match reader.await {
            Ok(r) => r.map_err(|e| AutomationError::Io(e.to_string()))?,
            Err(e) if !e.is_cancelled() => return Err(AutomationError::Other(e.to_string())),
            Err(_) => {}
        }
        writer.await.map_err(|e| AutomationError::Other(e.to_string()))?.map_err(|e| AutomationError::Io(e.to_string()))?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_line_accepts_exact_limit_and_final_line() {
        let mut input = BufReader::new(std::io::Cursor::new(b"1234\nlast!".to_vec()));
        assert_eq!(bounded_line(&mut input, 5).await.unwrap().as_deref(), Some(&b"1234"[..]));
        assert_eq!(bounded_line(&mut input, 5).await.unwrap().as_deref(), Some(&b"last!"[..]));
        assert!(bounded_line(&mut input, 5).await.unwrap().is_none());

        let mut oversized_eof = BufReader::new(std::io::Cursor::new(b"longer".to_vec()));
        assert_eq!(bounded_line(&mut oversized_eof, 5).await.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn oversized_valid_tool_call_never_reaches_dispatch() {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "admission-test", "version": "1" }
            }
        });
        let mut input = format!("{initialize}\n{}\n", json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).into_bytes();
        let call = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "doc_new", "arguments": { "width": 8, "height": 8 } }
        })
        .to_string()
        .into_bytes();
        assert!(call.len() < MAX_REQUEST_BYTES);
        input.extend_from_slice(&call);
        // The valid JSON prefix spans multiple BufReader fills; only its whitespace suffix
        // crosses the byte limit. No part of this line may be forwarded to rmcp.
        input.extend(vec![b' '; MAX_REQUEST_BYTES - call.len()]);
        input.push(b'\n');
        let server = PhotocraftMcp::headless();
        let observer = server.clone();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), server.serve_io(std::io::Cursor::new(input), tokio::io::sink()))
            .await
            .expect("oversized MCP input should end the session");
        assert!(result.unwrap_err().to_string().contains("request exceeds 1048576 bytes"));
        let documents = observer.headless_op(|h| Ok(h.session.documents().len())).await.unwrap().unwrap();
        assert_eq!(documents, 0, "the oversized tool call must not run");
    }

    async fn next_reply(replies: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>) -> Value {
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), replies.next_line()).await.expect("a reply in time").unwrap().expect("a reply line");
        serde_json::from_str(&line).unwrap()
    }

    #[tokio::test]
    async fn a_line_that_is_not_utf8_gets_a_parse_error_and_the_session_keeps_serving() {
        let (mut client, input) = tokio::io::duplex(1 << 16);
        let (output, replies) = tokio::io::duplex(1 << 16);
        let server = PhotocraftMcp::headless();
        let observer = server.clone();
        let session = tokio::spawn(server.serve_io(input, output));
        let mut replies = BufReader::new(replies).lines();

        let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "utf8-test", "version": "1"}
        }});
        client.write_all(format!("{initialize}\n").as_bytes()).await.unwrap();
        assert_eq!(next_reply(&mut replies).await["id"], 1);
        client.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n").await.unwrap();

        client.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\xff\xfe}\n").await.unwrap();
        let error = next_reply(&mut replies).await;
        assert_eq!(error["error"]["code"], -32700, "{error}");
        assert_eq!(error["id"], Value::Null, "{error}");

        let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "doc_new", "arguments": {"width": 8, "height": 8}}});
        client.write_all(format!("{call}\n").as_bytes()).await.unwrap();
        let reply = next_reply(&mut replies).await;
        assert_eq!(reply["id"], 2, "{reply}");
        assert_ne!(reply["result"]["isError"], true, "{reply}");
        let documents = observer.headless_op(|h| Ok(h.session.documents().len())).await.unwrap().unwrap();
        assert_eq!(documents, 1, "the call after the bad line must run");

        drop(client);
        session.await.unwrap().unwrap();
    }
}
