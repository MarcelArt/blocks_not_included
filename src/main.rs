use bevy::prelude::*;
use bevy_egui::{EguiGlobalSettings, EguiPlugin};
use blocks_not_included::debug_panel::DebugPanelPlugin;
use blocks_not_included::input_policy::InputPolicyPlugin;
use blocks_not_included::lifecycle::{Library, LifecyclePlugin};
use blocks_not_included::menu::MenuPlugin;
use blocks_not_included::overlay::OverlayPlugin;
use blocks_not_included::player::PlayerPlugin;
use blocks_not_included::readout::ReadoutPlugin;
use blocks_not_included::render::{ChunkMeshPlugin, ScenePlugin};
use blocks_not_included::saves::{RulesPolicy, WorldStore};
use blocks_not_included::session::SessionPlugin;
use blocks_not_included::sim_bridge::SimBridgePlugin;
use blocks_not_included::sim_files::{SimFilesPlugin, assets_folder, read_rules};
use blocks_not_included::tools::GodToolsPlugin;
use blocks_not_included::user_dirs::{Build, UserDirs};

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                // The lifecycle saves the open World before the window closes.
                close_when_requested: false,
                ..default()
            }),
            SessionPlugin,
            InputPolicyPlugin,
            SimBridgePlugin,
            SimFilesPlugin,
            ChunkMeshPlugin,
            ScenePlugin,
            PlayerPlugin,
        ))
        .add_plugins((
            EguiPlugin::default(),
            GodToolsPlugin,
            OverlayPlugin,
            ReadoutPlugin,
            DebugPanelPlugin,
            LifecyclePlugin,
            MenuPlugin,
        ))
        // The main menu's camera, and each World's, hold egui's primary context: egui would give
        // it only to the first camera ever spawned.
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        .insert_resource(library())
        .run();
}

/// Where this build saves Worlds, and the rules it loads them under: a development build loads
/// each World with the rules it was saved with, and a release build only Worlds simulated under
/// the rules it ships.
fn library() -> Library {
    let assets = assets_folder();
    let policy = match Build::CURRENT {
        Build::Development => Ok(RulesPolicy::AsSaved),
        Build::Release => read_rules(&assets)
            .map(|(elements, settings)| RulesPolicy::Shipped { elements, settings })
            .map_err(|error| format!("the game's own rules can't be read: {error}")),
    };
    let store = policy.and_then(|policy| {
        let dirs = UserDirs::for_this_build().map_err(|error| error.to_string())?;
        WorldStore::open(&dirs.saves, policy)
            .map_err(|error| format!("can't open {}: {error}", dirs.saves.display()))
    });
    match store {
        Ok(store) => Library::new(store, assets),
        Err(why) => Library::unavailable(why, assets),
    }
}
