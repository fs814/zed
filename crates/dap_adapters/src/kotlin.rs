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
pub(crate) struct KotlinDebugAdapter;

impl KotlinDebugAdapter {
    const ADAPTER_NAME: &'static str = "Kotlin";

    fn stem_from_main_file(configuration: &Map<String, Value>) -> Option<String> {
        configuration
            .get("mainFile")
            .and_then(|main_file| main_file.as_str())
            .and_then(|main_file| {
                PathBuf::from(main_file)
                    .file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
            })
    }

    fn stem_from_project_root(configuration: &Map<String, Value>) -> Option<String> {
        configuration
            .get("projectRoot")
            .and_then(|project_root| project_root.as_str())
            .and_then(|project_root| {
                PathBuf::from(project_root)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
    }

    fn substitute_main_class(configuration: &mut Map<String, Value>) {
        let Some(main_class) = configuration
            .get("mainClass")
            .and_then(|value| value.as_str())
        else {
            return;
        };
        if !main_class.contains("$ZED_STEM") {
            return;
        }
        let Some(stem) = Self::stem_from_main_file(configuration)
            .or_else(|| Self::stem_from_project_root(configuration))
        else {
            return;
        };
        configuration.insert(
            "mainClass".into(),
            main_class.replace("$ZED_STEM", &stem).into(),
        );
    }

    fn project_root_from_main_file(configuration: &Map<String, Value>) -> Option<String> {
        let main_file = configuration
            .get("mainFile")
            .and_then(|value| value.as_str())?;
        let mut dir = PathBuf::from(main_file).parent()?.to_path_buf();

        loop {
            if dir.join("build.gradle.kts").is_file() || dir.join("build.gradle").is_file() {
                return Some(dir.to_string_lossy().into_owned());
            }
            if !dir.pop() {
                break;
            }
        }

        PathBuf::from(main_file)
            .parent()
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn normalize_project_root(configuration: &mut Map<String, Value>) {
        let Some(project_root) = Self::project_root_from_main_file(configuration) else {
            return;
        };
        configuration.insert("projectRoot".into(), project_root.into());
    }

    async fn proxy_command(
        delegate: &Arc<dyn DapDelegate>,
        adapter_command: String,
        user_args: Option<Vec<String>>,
    ) -> (String, Vec<String>) {
        let proxy_path = paths::home_dir()
            .join("sourcecode")
            .join("Settings")
            .join("binlinux")
            .join("kotlin-dap-proxy.py");

        let Some(python) = delegate
            .which(OsStr::new(if cfg!(windows) { "python" } else { "python3" }))
            .await
        else {
            return (adapter_command, user_args.unwrap_or_default());
        };

        if !proxy_path.is_file() {
            return (adapter_command, user_args.unwrap_or_default());
        }

        let mut arguments = vec![proxy_path.to_string_lossy().into_owned(), adapter_command];
        arguments.extend(user_args.unwrap_or_default());
        (python.to_string_lossy().into_owned(), arguments)
    }

    fn insert_launch_config(config: &mut Map<String, Value>, zed_scenario: &ZedDebugConfig) {
        if let dap::DebugRequest::Launch(launch) = &zed_scenario.request {
            config.insert("request".into(), "launch".into());
            config.insert("type".into(), "kotlin".into());
            config.insert("name".into(), zed_scenario.label.to_string().into());
            if let Some(cwd) = launch.cwd.as_ref() {
                config.insert(
                    "projectRoot".into(),
                    cwd.to_string_lossy().into_owned().into(),
                );
            }
        }
    }
}

#[async_trait(?Send)]
impl DebugAdapter for KotlinDebugAdapter {
    fn name(&self) -> DebugAdapterName {
        DebugAdapterName(Self::ADAPTER_NAME.into())
    }

    async fn config_from_zed_format(&self, zed_scenario: ZedDebugConfig) -> Result<DebugScenario> {
        let mut config = Map::default();
        match &zed_scenario.request {
            dap::DebugRequest::Launch(_) => {
                Self::insert_launch_config(&mut config, &zed_scenario);
            }
            dap::DebugRequest::Attach(attach) => {
                config.insert("request".into(), "attach".into());
                config.insert("type".into(), "kotlin".into());
                config.insert("name".into(), zed_scenario.label.to_string().into());
                config.insert("hostName".into(), "localhost".into());
                config.insert("port".into(), attach.process_id.unwrap_or(5005).into());
                config.insert("timeout".into(), 2000.into());
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
        Some(LanguageName::new("Kotlin"))
    }

    fn dap_schema(&self) -> serde_json::Value {
        json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["request", "projectRoot", "mainClass"],
                    "properties": {
                        "request": {
                            "type": "string",
                            "enum": ["launch"]
                        },
                        "type": {
                            "type": "string",
                            "enum": ["kotlin"],
                            "default": "kotlin"
                        },
                        "name": {
                            "type": "string",
                            "default": "Kotlin Launch"
                        },
                        "projectRoot": {
                            "type": "string"
                        },
                        "mainFile": {
                            "type": "string"
                        },
                        "mainClass": {
                            "type": "string"
                        },
                        "vmArguments": {
                            "type": "string",
                            "default": ""
                        },
                        "enableJsonLogging": {
                            "type": "boolean",
                            "default": false
                        },
                        "jsonLogFile": {
                            "type": ["string", "null"]
                        }
                    }
                },
                {
                    "type": "object",
                    "required": ["request", "projectRoot", "hostName", "port", "timeout"],
                    "properties": {
                        "request": {
                            "type": "string",
                            "enum": ["attach"]
                        },
                        "type": {
                            "type": "string",
                            "enum": ["kotlin"],
                            "default": "kotlin"
                        },
                        "name": {
                            "type": "string",
                            "default": "Kotlin Attach"
                        },
                        "projectRoot": {
                            "type": "string"
                        },
                        "hostName": {
                            "type": "string",
                            "default": "localhost"
                        },
                        "port": {
                            "type": "number",
                            "default": 5005
                        },
                        "timeout": {
                            "type": "number",
                            "default": 2000
                        },
                        "enableJsonLogging": {
                            "type": "boolean",
                            "default": false
                        },
                        "jsonLogFile": {
                            "type": ["string", "null"]
                        }
                    }
                }
            ]
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
        let adapter_command = if let Some(path) = user_installed_path {
            path.to_string_lossy().into_owned()
        } else {
            delegate
                .which(OsStr::new("kotlin-debug-adapter"))
                .await
                .context("Could not find kotlin-debug-adapter in PATH or dap.Kotlin.binary")?
                .to_string_lossy()
                .into_owned()
        };

        let mut configuration = config.config.clone();
        if let Some(configuration) = configuration.as_object_mut() {
            configuration
                .entry("type")
                .or_insert_with(|| "kotlin".into());
            configuration
                .entry("projectRoot")
                .or_insert_with(|| delegate.worktree_root_path().to_string_lossy().into());
            configuration
                .entry("enableJsonLogging")
                .or_insert_with(|| false.into());
            Self::normalize_project_root(configuration);
            Self::substitute_main_class(configuration);
            configuration.remove("mainFile");
        }

        let mut envs = delegate.shell_env().await;
        envs.extend(user_env.unwrap_or_default());
        if let Some(project_root) = configuration
            .get("projectRoot")
            .and_then(|value| value.as_str())
        {
            envs.insert(
                "ZED_KOTLIN_DAP_PROJECT_ROOT".into(),
                project_root.to_owned(),
            );
        }

        let (command, arguments) = Self::proxy_command(delegate, adapter_command, user_args).await;

        Ok(DebugAdapterBinary {
            command: Some(command),
            arguments,
            envs,
            cwd: Some(delegate.worktree_root_path().to_path_buf()),
            connection: None,
            request_args: StartDebuggingRequestArguments {
                request: self.request_kind(&configuration).await?,
                configuration,
            },
        })
    }
}
