use anyhow::Result;
use async_trait::async_trait;
use collections::HashMap;
use dap::{
    StartDebuggingRequestArguments,
    adapters::{DebugTaskDefinition, TcpArguments},
};
use gpui::AsyncApp;
use language::LanguageName;
use serde_json::{Map, Value};
use std::path::PathBuf;
use task::{DebugScenario, TcpArgumentsTemplate, ZedDebugConfig};

use crate::*;

#[derive(Default)]
pub(crate) struct GDScriptDebugAdapter;

impl GDScriptDebugAdapter {
    const ADAPTER_NAME: &'static str = "GDScript";

    fn insert_launch_config(config: &mut Map<String, Value>, zed_scenario: &ZedDebugConfig) {
        if let dap::DebugRequest::Launch(launch) = &zed_scenario.request {
            config.insert("request".into(), "launch".into());
            config.insert("type".into(), "gdscript".into());
            config.insert("name".into(), zed_scenario.label.to_string().into());
            config.insert("program".into(), launch.program.clone().into());

            if !launch.args.is_empty() {
                config.insert("args".into(), launch.args.join(" ").into());
            } else {
                config.insert("args".into(), "".into());
            }
            if let Some(cwd) = launch.cwd.as_ref() {
                config.insert("cwd".into(), cwd.to_string_lossy().into_owned().into());
            }
        }
    }
}

#[async_trait(?Send)]
impl DebugAdapter for GDScriptDebugAdapter {
    fn name(&self) -> DebugAdapterName {
        DebugAdapterName(Self::ADAPTER_NAME.into())
    }

    async fn config_from_zed_format(&self, zed_scenario: ZedDebugConfig) -> Result<DebugScenario> {
        let mut config = Map::default();
        match &zed_scenario.request {
            dap::DebugRequest::Launch(_) => {
                Self::insert_launch_config(&mut config, &zed_scenario);
            }
            dap::DebugRequest::Attach(_) => {
                anyhow::bail!("GDScript debugging uses a launch request to Godot's debug server");
            }
        }

        Ok(DebugScenario {
            adapter: zed_scenario.adapter,
            label: zed_scenario.label,
            build: None,
            config: Value::Object(config),
            tcp_connection: Some(TcpArgumentsTemplate {
                port: Some(6006),
                host: Some(std::net::Ipv4Addr::LOCALHOST.into()),
                timeout: Some(5000),
            }),
        })
    }

    fn adapter_language_name(&self) -> Option<LanguageName> {
        Some(LanguageName::new("GDScript"))
    }

    fn dap_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["request"],
            "properties": {
                "request": {
                    "type": "string",
                    "enum": ["launch"]
                },
                "type": {
                    "type": "string",
                    "enum": ["gdscript"],
                    "default": "gdscript"
                },
                "name": {
                    "type": "string",
                    "default": "Gdscript Debug"
                },
                "program": {
                    "type": ["string", "null"]
                },
                "cwd": {
                    "type": "string"
                },
                "args": {
                    "type": "string",
                    "default": ""
                }
            }
        })
    }

    async fn get_binary(
        &self,
        delegate: &Arc<dyn DapDelegate>,
        config: &DebugTaskDefinition,
        _: Option<PathBuf>,
        _: Option<Vec<String>>,
        user_env: Option<HashMap<String, String>>,
        _: &mut AsyncApp,
    ) -> Result<DebugAdapterBinary> {
        let tcp_connection = config
            .tcp_connection
            .clone()
            .unwrap_or(TcpArgumentsTemplate {
                port: Some(6006),
                host: Some(std::net::Ipv4Addr::LOCALHOST.into()),
                timeout: Some(5000),
            });
        let (host, port, timeout) = crate::configure_tcp_connection(tcp_connection).await?;

        let mut configuration = config.config.clone();
        if let Some(configuration) = configuration.as_object_mut() {
            configuration
                .entry("type")
                .or_insert_with(|| "gdscript".into());
            configuration
                .entry("cwd")
                .or_insert_with(|| delegate.worktree_root_path().to_string_lossy().into());
            configuration.entry("args").or_insert_with(|| "".into());
        }

        let mut envs = delegate.shell_env().await;
        envs.extend(user_env.unwrap_or_default());

        Ok(DebugAdapterBinary {
            command: None,
            arguments: vec![],
            envs,
            cwd: Some(delegate.worktree_root_path().to_path_buf()),
            connection: Some(TcpArguments {
                host,
                port,
                timeout,
            }),
            request_args: StartDebuggingRequestArguments {
                request: self.request_kind(&configuration).await?,
                configuration,
            },
        })
    }
}
