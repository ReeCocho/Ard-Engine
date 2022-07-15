pub mod asset_meta;
pub mod editor_job;
pub mod gui;
pub mod inspect;
pub mod par_task;
pub mod scene_graph;

use ard_engine::{
    assets::prelude::*, core::prelude::*, game::GamePlugin, graphics::prelude::*,
    window::prelude::*,
};

use ard_engine::graphics_assets::prelude as graphics_assets;

use asset_meta::{AssetMeta, AssetMetaLoader};
use gui::Editor;
use scene_graph::{SceneGraph, SceneGraphAsset, SceneGraphLoader};

fn main() {
    AppBuilder::new(ard_engine::log::LevelFilter::Info)
        .add_plugin(ArdCorePlugin)
        .add_plugin(WindowPlugin {
            add_primary_window: Some(WindowDescriptor {
                width: 1280.0,
                height: 720.0,
                title: String::from("Ard Editor"),
                vsync: false,
                ..Default::default()
            }),
            exit_on_close: true,
        })
        .add_plugin(WinitPlugin)
        .add_plugin(VkGraphicsPlugin {
            context_create_info: GraphicsContextCreateInfo {
                window: WindowId::primary(),
                debug: true,
            },
        })
        .add_plugin(AssetsPlugin)
        .add_plugin(GamePlugin)
        .add_plugin(graphics_assets::GraphicsAssetsPlugin)
        .add_resource(SceneGraph::default())
        .add_startup_function(Editor::startup)
        .add_startup_function(setup)
        .run();
}

fn setup(app: &mut App) {
    // Register meta loader
    let assets = app.resources.get::<Assets>().unwrap();
    assets.register::<AssetMeta>(AssetMetaLoader);
    assets.register::<SceneGraphAsset>(SceneGraphLoader);

    let mut settings = app.resources.get_mut::<RendererSettings>().unwrap();

    // Don't use the canvas size
    settings.canvas_size = Some((100, 100));

    // Disable frame rate limit
    settings.render_time = None;

    // Don't render the scene to the surface
    settings.render_scene = false;
}
