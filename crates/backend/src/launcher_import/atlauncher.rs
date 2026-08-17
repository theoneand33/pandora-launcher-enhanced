use auth::{
    credentials::AccountCredentials,
    models::{TokenWithExpiry, XstsToken},
    secret::PlatformSecretStorage,
};
use bridge::{import::ImportFromOtherLauncherJob, modal_action::ModalAction};
use chrono::DateTime;
use log::debug;
use schema::{
    instance::{InstanceConfiguration, InstanceMemoryConfiguration, InstanceWrapperCommandConfiguration},
    loader::Loader,
};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use uuid::Uuid;

use crate::{BackendState, account::BackendAccount};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtLauncherConfig {
    maximum_memory: Option<usize>,
    // i'm assuming this is optional if there is no said last account.
    last_account: Option<Uuid>,
}

/// Going to just get the types converted before deleting a bunch probably...
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtLauncherInstance {
    launcher: Launcher,
    id: String,
    // NOTE: enable the below line will cause an error as `rules.features.has_custom_resolution` is a `"true"` not `true`
    // NOTE: That being said, we probably don't need to worry about it that much... hopefully...
    // arguments: LaunchArguments,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Launcher {
    // name: String,
    // pack: String,
    // description: String,
    // pack_id: usize,
    // external_pack_id: usize,
    /// This is modpack version. NOT GAME VERSION
    // version: String,
    // enable_curse_forge_integration: bool,
    // enable_editing_mods: bool,
    loader_version: Option<LoaderVersion>,
    required_memory: usize,
    // required_perm_gen: usize,
    maximum_memory: Option<usize>,
    enable_commands: Option<bool>,
    wrapper_command: Option<String>,
    // use_system_glfw: Option<bool>,
    // use_system_open_al: Option<bool>,
    account: Option<Uuid>,
    // quick_play: QuickPlay,
    // is_dev: bool,
    // is_playable: bool,
    // assets_map_to_resources: bool,
    // curse_forge_project: Option<CurseForgeProject>,
    // curse_forge_project_description: Option<String>,
    // curse_forge_file: Option<CurseForgeFile>,
    // override_paths: Vec<String>,
    // check_for_updates: bool,
    // mods: Vec<Mod>,
    // ignored_updates: Vec<String>,
    // ignore_all_updates: bool,
    // vanilla_instance: bool,
    // last_played: usize,
    // num_plays: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoaderVersion {
    raw_version: String,
    #[serde(rename = "type")]
    loader_type: Loader,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtLauncherAccount {
    access_token: String,
    xsts_auth: AtLauncherXstsAuth,
    access_token_expires_at: String,
    username: Uuid,
    minecraft_username: String,
    uuid: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AtLauncherXstsAuth {
    not_after: String,
    token: String,
    display_claims: AtLauncherDisplayClaims,
}

#[derive(Debug, Deserialize)]
struct AtLauncherDisplayClaims {
    xui: Vec<AtLauncherDisplayClaim>,
}

#[derive(Debug, Deserialize)]
struct AtLauncherDisplayClaim {
    uhs: String,
}

pub async fn import_from_atlauncher(
    backend: &BackendState,
    import_job: ImportFromOtherLauncherJob,
    modal_action: ModalAction,
) {
    let Ok(launcher_config_bytes) = std::fs::read(import_job.root.join("configs/ATLauncher.json")) else {
        return;
    };
    let launcher_config =
        serde_json::from_slice::<AtLauncherConfig>(&launcher_config_bytes).expect("Failed to parse to json");

    let accounts = import_accounts_from_atlauncher(backend, &import_job, &launcher_config, &modal_action).await;
    import_instances_from_atlauncher(backend, &import_job, &launcher_config, &modal_action, &accounts);
}

async fn import_accounts_from_atlauncher(
    backend: &BackendState,
    import_job: &ImportFromOtherLauncherJob,
    launcher_config: &AtLauncherConfig,
    modal_action: &ModalAction,
) -> Option<Vec<AtLauncherAccount>> {
    if !import_job.import_accounts {
        return None;
    }

    let tracker = modal_action.push_tracker("Reading accounts.json".into());
    tracker.set_total(1);

    let accounts_path = import_job.root.join("configs/accounts.json");
    let Ok(accounts_bytes) = std::fs::read(&accounts_path) else {
        tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Error);
        return None;
    };

    let Ok(accounts_json) = serde_json::from_slice::<Vec<AtLauncherAccount>>(&accounts_bytes) else {
        tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Error);
        return None;
    };

    let secret_storage = match backend.secret_storage.get_or_init(PlatformSecretStorage::new).await {
        Ok(secret_storage) => secret_storage,
        Err(error) => {
            log::error!("Error initializing secret storage: {error}");
            tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Error);
            return None;
        },
    };

    tracker.set_count(1);
    tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Normal);

    let num_accounts = accounts_json.len();
    let tracker = modal_action.push_tracker("Importing accounts".into());
    tracker.set_total(num_accounts);

    backend.account_info.write().modify(|accounts| {
        let mut last_account_username = None;
        for account in &accounts_json {
            tracker.add_count(1);
            accounts.accounts.insert(
                account.uuid,
                BackendAccount {
                    username: account.minecraft_username.clone().into(),
                    offline: false,
                    head: None,
                    offline_skin: None,
                    offline_skin_variant: None,
                },
            );
            if let Some(last_account) = launcher_config.last_account
                && account.username == last_account
            {
                last_account_username = Some(account.uuid);
            }
        }
        accounts.selected_account = last_account_username;
    });
    tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Normal);

    let tracker = modal_action.push_tracker("Importing credentials".into());
    tracker.set_total(num_accounts);

    for account in &accounts_json {
        let mut credentials = AccountCredentials::default();
        let mut non_default_creds = false;
        let now = chrono::Utc::now();

        if let Ok(expiry) = DateTime::from_str(&account.access_token_expires_at)
            && expiry < now
        {
            non_default_creds = true;
            credentials.access_token = Some(TokenWithExpiry {
                token: account.access_token.clone().into(),
                expiry,
            });
        }
        if let Ok(expiry) = DateTime::from_str(&account.xsts_auth.not_after)
            && expiry < now
        {
            non_default_creds = true;
            credentials.xsts = Some(XstsToken {
                token: account.xsts_auth.token.clone().into(),
                expiry,
                userhash: account.xsts_auth.display_claims.xui[0].uhs.clone().into(),
            });
        }

        // credential

        if non_default_creds {
            _ = secret_storage.write_credentials(account.uuid, &credentials).await;
        }
    }

    tracker.set_count(num_accounts);
    tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Normal);

    Some(accounts_json)
}

struct AtLauncherInstanceToImport {
    pandora_path: PathBuf,
    config_path: PathBuf,
    folder: Arc<Path>,
}

fn try_load_from_atlauncher(
    config_path: &Path,
    launcher_config: &AtLauncherConfig,
    accounts: &Option<Vec<AtLauncherAccount>>,
) -> anyhow::Result<InstanceConfiguration> {
    // let instance_cfg_bytes = std::fs::read(config_path)?;
    // let instance_cfg = serde_json::from_slice::<AtLauncherInstance>(&instance_cfg_bytes)?;
    let instance_cfg_bytes = std::fs::read(config_path).expect("Failed to read from fs");
    let instance_cfg =
        serde_json::from_slice::<AtLauncherInstance>(&instance_cfg_bytes).expect("Failed to convert to json");

    // tbh, idk why they have it as `id` they just do...
    // or at least, it's the most reliable one i've managed to read from so far.
    let mut configuration = InstanceConfiguration::new(
        instance_cfg.id.into(),
        instance_cfg
            .launcher
            .loader_version
            .as_ref()
            .map(|loader_version| loader_version.loader_type)
            .unwrap_or(Loader::Vanilla),
    );

    configuration.memory =
        if let Some(max_memory) = instance_cfg.launcher.maximum_memory.or(launcher_config.maximum_memory) {
            Some(InstanceMemoryConfiguration {
                enabled: true,
                min: instance_cfg.launcher.required_memory as u32,
                max: max_memory as u32,
            })
        } else {
            None
        };

    if let Some(enable_commands) = instance_cfg.launcher.enable_commands
        && enable_commands
    {
        configuration.wrapper_command = if let Some(wrapper_command) = instance_cfg.launcher.wrapper_command {
            Some(InstanceWrapperCommandConfiguration {
                enabled: true,
                flags: wrapper_command.into(),
            })
        } else {
            None
        };
    }

    configuration.preferred_loader_version = instance_cfg
        .launcher
        .loader_version
        .map(|loader_version| loader_version.raw_version.into());
    if let Some(accounts) = accounts {
        configuration.preferred_account = instance_cfg
            .launcher
            .account
            .map(|username| accounts.iter().find(|account| account.username == username).map(|account| account.uuid))
            .flatten();
    }

    Ok(configuration)
}

fn import_instances_from_atlauncher(
    backend: &BackendState,
    import_job: &ImportFromOtherLauncherJob,
    launcher_config: &AtLauncherConfig,
    modal_action: &ModalAction,
    accounts: &Option<Vec<AtLauncherAccount>>,
) {
    if import_job.paths.is_empty() {
        return;
    }

    let all_tracker = modal_action.push_tracker("Importing instances".into());

    let mut to_import = Vec::new();

    for folder in import_job.paths.iter() {
        if !folder.is_dir() {
            continue;
        }

        let Some(filename) = folder.file_name() else {
            continue;
        };

        let pandora_path = backend.directories.instances_dir.join(filename);
        if pandora_path.exists() {
            continue;
        }

        let atlauncher_instance_cfg = folder.join("instance.json");
        if !atlauncher_instance_cfg.exists() {
            continue;
        }

        debug!("Loading: {:?}", filename);

        to_import.push(AtLauncherInstanceToImport {
            pandora_path,
            config_path: atlauncher_instance_cfg,
            folder: folder.clone(),
        });
    }

    all_tracker.set_total(to_import.len());

    for to_import in to_import {
        let title = format!("Importing {}", to_import.folder.file_name().unwrap().to_string_lossy());
        let tracker = modal_action.push_tracker(title.into());

        let Ok(configuration) = try_load_from_atlauncher(&to_import.config_path, launcher_config, accounts) else {
            tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Error);
            log::error!(
                "Failed to load config path from atlauncher for {:?}",
                to_import.folder.file_name().unwrap()
            );
            continue;
        };

        let Ok(configuration_bytes) = serde_json::to_vec(&configuration) else {
            tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Error);
            continue;
        };

        _ = std::fs::create_dir_all(&to_import.pandora_path);
        let target_dot_minecraft = to_import.pandora_path.join(".minecraft");

        _ = std::fs::create_dir_all(&target_dot_minecraft);
        if let Err(err) =
            crate::fs::copy_content_recursive(&to_import.folder, &target_dot_minecraft, false, &|copied, total| {
                tracker.set_total(total as usize);
                tracker.set_count(copied as usize);
            })
        {
            log::error!("Failed to copy ATLauncher instance {:?}: {err:?}", to_import.folder);
            backend.send.send_error(format!("Failed to copy instance: {err}"));
            tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Error);
            if let Err(cleanup_err) = std::fs::remove_dir_all(&to_import.pandora_path) {
                log::error!(
                    "Failed to clean up partial ATLauncher import {:?}: {cleanup_err:?}",
                    to_import.pandora_path
                );
            }
            continue;
        }

        // remove old configuration, rename icon path.
        _ = std::fs::rename(&target_dot_minecraft.join("instance.png"), &to_import.pandora_path.join("icon.png"));
        _ = std::fs::remove_file(&target_dot_minecraft.join("instance.json"));

        // move disable mods
        let mods_path = target_dot_minecraft.join("mods");
        let resourcepacks_path = target_dot_minecraft.join("resourcepacks");

        let disabled_mods_path = target_dot_minecraft.join("disabledmods");
        if let Ok(disabled_mods_folder) = std::fs::read_dir(&disabled_mods_path) {
            // moving mods to the mods folder could throw an error if there was no mod folder, if all mods were disabled for example
            _ = std::fs::create_dir(&mods_path);
            _ = std::fs::create_dir(&resourcepacks_path);

            for mod_file in disabled_mods_folder {
                let Ok(entry) = mod_file else {
                    continue;
                };

                let Ok(file_name) = entry.file_name().to_owned().into_string() else {
                    continue;
                };

                let new_path = match &file_name {
                    resourcepack if resourcepack.ends_with(".zip") => &resourcepacks_path,
                    jar_mod if jar_mod.ends_with(".jar") => &mods_path,
                    _ => continue,
                };

                _ = std::fs::rename(entry.path(), new_path.join(file_name + ".disabled"));
            }

            // cleanup old disabled mod folder
            _ = std::fs::remove_dir_all(&disabled_mods_path);
        }

        let info_path = to_import.pandora_path.join("info_v1.json");
        if let Err(err) = crate::fs::write_safe(&info_path, &configuration_bytes) {
            log::error!("Failed to write ATLauncher instance config {:?}: {err:?}", info_path);
            backend.send.send_error(format!("Failed to write instance config: {err}"));
            tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Error);
            if let Err(cleanup_err) = std::fs::remove_dir_all(&to_import.pandora_path) {
                log::error!(
                    "Failed to clean up partial ATLauncher import {:?}: {cleanup_err:?}",
                    to_import.pandora_path
                );
            }
            continue;
        }

        all_tracker.add_count(1);

        tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Fast);
    }

    all_tracker.set_finished(bridge::modal_action::ProgressTrackerFinishType::Normal);
}
