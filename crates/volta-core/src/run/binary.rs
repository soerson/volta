use std::ffi::OsStr;
use std::path::PathBuf;

use super::{debug_tool_message, ToolCommand};
use crate::error::{ErrorKind, Fallible};
use crate::layout::volta_home;
use crate::platform::{CliPlatform, Platform, Sourced};
use crate::session::{ActivityKind, Session};
use crate::tool::bin_full_path;
use crate::tool::{BinConfig, BinLoader};
use log::debug;

pub(crate) fn command(
    exe: &OsStr,
    cli: CliPlatform,
    session: &mut Session,
) -> Fallible<ToolCommand> {
    session.add_event_start(ActivityKind::Binary);

    // first try to use the project toolchain
    if let Some(project) = session.project()? {
        // check if the executable is a direct dependency
        if project.has_direct_bin(&exe)? {
            // use the full path to the file
            let mut path_to_bin = project.local_bin_dir();
            path_to_bin.push(&exe);

            if !path_to_bin.is_file() {
                return Err(ErrorKind::ProjectLocalBinaryNotFound {
                    command: path_to_bin.to_string_lossy().to_string(),
                }
                .into());
            }

            debug!(
                "Found {} in project at '{}'",
                exe.to_string_lossy(),
                path_to_bin.display()
            );
            let path_to_bin = path_to_bin.as_os_str();

            if let Some(platform) = Platform::with_cli(cli, session)? {
                debug_tool_message("node", &platform.node);

                let image = platform.checkout(session)?;
                let path = image.path()?;
                return Ok(ToolCommand::project_local(&path_to_bin, &path));
            }

            // if there's no platform available, pass through to existing PATH.
            debug!("Could not find Volta configuration, delegating to system");
            return ToolCommand::passthrough(&path_to_bin, ErrorKind::NoPlatform);
        }
    }

    // try to use the default toolchain
    if let Some(default_tool) = DefaultBinary::from_name(&exe, session)? {
        let image = cli.merge(default_tool.platform).checkout(session)?;
        debug!(
            "Found default {} in '{}'",
            exe.to_string_lossy(),
            default_tool.bin_path.display()
        );
        debug_tool_message("node", &image.node);

        let path = image.path()?;
        let tool_path = default_tool.bin_path.into_os_string();
        let cmd = match default_tool.loader {
            Some(loader) => {
                let mut command = ToolCommand::direct(loader.command.as_ref(), &path);
                command.args(loader.args);
                command.arg(tool_path);
                command
            }
            None => ToolCommand::direct(&tool_path, &path),
        };
        return Ok(cmd);
    }

    // at this point, there is no project or default toolchain
    // Pass through to the existing PATH
    debug!(
        "Could not find {}, delegating to system",
        exe.to_string_lossy()
    );
    ToolCommand::passthrough(
        &exe,
        ErrorKind::BinaryNotFound {
            name: exe.to_string_lossy().to_string(),
        },
    )
}

/// Information about the location and execution context of default binaries
///
/// Fetched from the config files in the Volta directory, represents the binary that is executed
/// when the user is outside of a project that has the given bin as a dependency.
pub struct DefaultBinary {
    pub bin_path: PathBuf,
    pub platform: Platform,
    pub loader: Option<BinLoader>,
}

impl DefaultBinary {
    pub fn from_config(bin_config: BinConfig, session: &mut Session) -> Fallible<Self> {
        let bin_path = bin_full_path(
            &bin_config.package,
            &bin_config.version,
            &bin_config.name,
            &bin_config.path,
        )?;

        // If the user does not have yarn set in the platform for this binary, use the default
        // This is necessary because some tools (e.g. ember-cli with the `--yarn` option) invoke `yarn`
        let yarn = match bin_config.platform.yarn {
            Some(yarn) => Some(yarn),
            None => session
                .default_platform()?
                .and_then(|ref plat| plat.yarn.clone()),
        };
        let platform = Platform {
            node: Sourced::with_binary(bin_config.platform.node),
            npm: bin_config.platform.npm.map(Sourced::with_binary),
            yarn: yarn.map(Sourced::with_binary),
        };

        Ok(DefaultBinary {
            bin_path,
            platform,
            loader: bin_config.loader,
        })
    }

    pub fn from_name(tool_name: &OsStr, session: &mut Session) -> Fallible<Option<Self>> {
        let bin_config_file = match tool_name.to_str() {
            Some(name) => volta_home()?.default_tool_bin_config(name),
            None => return Ok(None),
        };

        if bin_config_file.exists() {
            let bin_config = BinConfig::from_file(bin_config_file)?;
            DefaultBinary::from_config(bin_config, session).map(Some)
        } else {
            Ok(None) // no config means the tool is not installed
        }
    }
}
