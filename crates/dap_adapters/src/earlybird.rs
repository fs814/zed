use anyhow::{Context as _, Result};
use async_trait::async_trait;
use collections::HashMap;
use dap::{StartDebuggingRequestArguments, adapters::DebugTaskDefinition};
use gpui::AsyncApp;
use language::LanguageName;
use serde_json::{Map, Value};
use std::{ffi::OsStr, path::PathBuf};
use task::{DebugScenario, ZedDebugConfig};

use crate::*;

#[derive(Default)]
pub(crate) struct EarlybirdDebugAdapter;

impl EarlybirdDebugAdapter {
    const ADAPTER_NAME: &'static str = "Earlybird";

    fn insert_launch_config(config: &mut Map<String, Value>, zed_scenario: &ZedDebugConfig) {
        if let dap::DebugRequest::Launch(launch) = &zed_scenario.request {
            config.insert("program".into(), launch.program.clone().into());

            if !launch.args.is_empty() {
                config.insert("arguments".into(), launch.args.clone().into());
            }
            if !launch.env.is_empty() {
                config.insert("env".into(), launch.env_json());
            }
            if let Some(stop_on_entry) = zed_scenario.stop_on_entry {
                config.insert("stopOnEntry".into(), stop_on_entry.into());
            }
            if let Some(cwd) = launch.cwd.as_ref() {
                config.insert("cwd".into(), cwd.to_string_lossy().into_owned().into());
            }
        }
    }
}

#[async_trait(?Send)]
impl DebugAdapter for EarlybirdDebugAdapter {
    fn name(&self) -> DebugAdapterName {
        DebugAdapterName(Self::ADAPTER_NAME.into())
    }

    async fn config_from_zed_format(&self, zed_scenario: ZedDebugConfig) -> Result<DebugScenario> {
        let mut config = Map::default();
        match &zed_scenario.request {
            dap::DebugRequest::Launch(_) => {
                config.insert("request".into(), "launch".into());
                Self::insert_launch_config(&mut config, &zed_scenario);
            }
            dap::DebugRequest::Attach(_) => {
                anyhow::bail!("Earlybird only supports launch requests");
            }
        }

        Ok(DebugScenario {
            adapter: zed_scenario.adapter,
            label: zed_scenario.label,
            build: None,
            config: Value::Object(config),
            tcp_connection: None,
        })
    }

    fn adapter_language_name(&self) -> Option<LanguageName> {
        Some(LanguageName::new("OCaml"))
    }

    fn dap_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["request", "program"],
            "properties": {
                "request": {
                    "type": "string",
                    "enum": ["launch"]
                },
                "program": {
                    "type": "string",
                    "description": "Path to the OCaml bytecode executable to debug"
                },
                "arguments": {
                    "type": "array",
                    "items": { "type": "string" },
                    "default": []
                },
                "cwd": {
                    "type": "string"
                },
                "env": {
                    "type": "object",
                    "patternProperties": {
                        ".*": { "type": "string" }
                    }
                },
                "stopOnEntry": {
                    "type": "boolean",
                    "default": false
                },
                "console": {
                    "type": "string",
                    "enum": ["internalConsole", "integratedTerminal", "externalTerminal"],
                    "default": "internalConsole"
                },
                "source_dirs": {
                    "type": "array",
                    "items": { "type": "string" },
                    "default": []
                },
                "onlyDebugGlob": {
                    "type": "string"
                },
                "yieldSteps": {
                    "type": "number",
                    "default": 4096
                },
                "_debugLog": {
                    "type": "string"
                }
            }
        })
    }

    async fn get_binary(
        &self,
        delegate: &Arc<dyn DapDelegate>,
        config: &DebugTaskDefinition,
        user_installed_path: Option<PathBuf>,
        user_args: Option<Vec<String>>,
        user_env: Option<HashMap<String, String>>,
        _: &mut AsyncApp,
    ) -> Result<DebugAdapterBinary> {
        let command = if let Some(path) = user_installed_path {
            path.to_string_lossy().into_owned()
        } else {
            delegate
                .which(OsStr::new("ocamlearlybird"))
                .await
                .context("Could not find ocamlearlybird in PATH or dap.Earlybird.binary")?
                .to_string_lossy()
                .into_owned()
        };

        let mut configuration = config.config.clone();
        if let Some(configuration) = configuration.as_object_mut() {
            configuration
                .entry("cwd")
                .or_insert_with(|| delegate.worktree_root_path().to_string_lossy().into());
        }

        let mut envs = delegate.shell_env().await;
        envs.extend(user_env.unwrap_or_default());
        if let Some(config_env) = config.config.get("env").and_then(|env| env.as_object()) {
            envs.extend(config_env.iter().filter_map(|(key, value)| {
                value.as_str().map(|value| (key.clone(), value.to_string()))
            }));
        }

        Ok(DebugAdapterBinary {
            command: Some(command),
            arguments: user_args.unwrap_or_else(|| vec!["debug".into()]),
            envs,
            cwd: Some(delegate.worktree_root_path().to_path_buf()),
            connection: None,
            request_args: StartDebuggingRequestArguments {
                request: self.request_kind(&config.config).await?,
                configuration,
            },
        })
    }
}
