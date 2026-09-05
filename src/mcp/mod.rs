//! MCP server over stdio (`abcop --mcp`).
//!
//! Uses the official [`rmcp`] SDK. Tool: `abcop_inspection` (ABC / UsedOnce /
//! NeverUsed). No autocorrect — abcop is a gate, not a rewriter.

mod offense;
mod tools;

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt};
use serde::Deserialize;
use tokio::runtime::Runtime;

use tools::State;

/// MCP session: shared scan defaults + tool router.
#[derive(Clone)]
pub struct AbcopMcp {
    state: Arc<State>,
    #[allow(dead_code)] // read by `#[tool_handler]` macro glue
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InspectionArgs {
    /// File or directory to inspect (default: current directory).
    #[serde(default)]
    path: Option<String>,
    /// Inline source (skips filesystem discovery; `path` selects language).
    #[serde(default)]
    source_code: Option<String>,
}

#[tool_router]
impl AbcopMcp {
    pub fn new() -> Self {
        Self {
            state: Arc::new(State::default()),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "abcop_inspection",
        description = "Inspect code for ABC complexity, UsedOnce, and NeverUsed findings. Provide `source_code` to check inline code (optional `path` for language) or `path` to check files.",
        annotations(
            title = "abcop inspection",
            read_only_hint = true,
            idempotent_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn inspection(
        &self,
        Parameters(args): Parameters<InspectionArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(tool_result(tools::inspect(
            &self.state,
            args.path,
            args.source_code,
        )))
    }
}

fn tool_result(r: Result<String, String>) -> CallToolResult {
    match r {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(text) => CallToolResult::error(vec![ContentBlock::text(text)]),
    }
}

#[tool_handler]
impl ServerHandler for AbcopMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "abcop_mcp_server",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
            .with_instructions(
                "abcop complexity gate: abcop_inspection (ABC / UsedOnce / NeverUsed)."
                    .to_string(),
            )
    }
}

/// CLI entry: serve MCP over stdin/stdout until the client disconnects.
pub fn run() -> ExitCode {
    match run_inner() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::from(2)
        }
    }
}

fn run_inner() -> Result<ExitCode> {
    let rt = Runtime::new()?;
    rt.block_on(async {
        let service = AbcopMcp::new().serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(ExitCode::SUCCESS)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;
    use serde_json::Map;

    async fn with_client<F, Fut>(f: F)
    where
        F: FnOnce(rmcp::service::RunningService<rmcp::RoleClient, ()>) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let (server_side, client_side) = tokio::io::duplex(64 * 1024);
        let server = AbcopMcp::new();
        let server_task = tokio::spawn(async move {
            let _ = server
                .serve(server_side)
                .await
                .expect("serve")
                .waiting()
                .await;
        });
        f(().serve(client_side).await.expect("client")).await;
        server_task.abort();
    }

    fn args_map(v: serde_json::Value) -> Map<String, serde_json::Value> {
        v.as_object().expect("object").clone()
    }

    #[tokio::test]
    async fn list_tools() {
        with_client(|client| async move {
            let names: Vec<_> = client
                .list_all_tools()
                .await
                .expect("list")
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect();
            assert!(names.iter().any(|n| n == "abcop_inspection"));
            assert_eq!(names.len(), 1);
            let _ = client.cancel().await;
        })
        .await;
    }

    #[tokio::test]
    async fn inspect_inline_never_used() {
        with_client(|client| async move {
            let result = client
                .call_tool(
                    CallToolRequestParams::new("abcop_inspection").with_arguments(args_map(
                        serde_json::json!({
                            "source_code": "def foo\n  x = 1\nend\n",
                            "path": "example.rb"
                        }),
                    )),
                )
                .await
                .expect("call");
            assert_eq!(result.is_error, Some(false));
            let offenses: serde_json::Value =
                serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
            assert!(
                offenses
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|o| o["code"] == "NeverUsed"),
                "got: {offenses}"
            );
            let _ = client.cancel().await;
        })
        .await;
    }
}
