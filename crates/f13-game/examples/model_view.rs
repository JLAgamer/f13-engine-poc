//! Renders one exported glTF at the origin and saves a screenshot: model_view <export dir> <glb path> <out.png>
use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::world_serialization::WorldAssetRoot;

#[derive(Resource)]
struct Out(String, u32);

fn main() {
    let a: Vec<String> = std::env::args().collect();
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin { file_path: a[1].clone(), ..default() }))
        .insert_resource(Out(a[3].clone(), 0))
        .insert_resource(Model(a[2].clone()))
        .add_systems(Startup, setup)
        .add_systems(Update, shoot)
        .run();
}

#[derive(Resource)]
struct Model(String);

fn setup(mut commands: Commands, assets: Res<AssetServer>, model: Res<Model>) {
    // Parented under an offset entity, like the player.
    let off = Vec3::new(50.0, -0.3, 40.0);
    commands
        .spawn((Transform::from_translation(off), Visibility::default()))
        .with_child(WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(model.0.clone()))));
    commands.spawn((Camera3d::default(), Transform::from_translation(off + Vec3::new(0.0, 1.0, 3.0)).looking_at(off + Vec3::new(0.0, 0.9, 0.0), Vec3::Y)));
    commands.spawn((DirectionalLight::default(), Transform::from_xyz(1.0, 2.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y)));
}

fn shoot(mut commands: Commands, mut out: ResMut<Out>, mut exit: MessageWriter<AppExit>) {
    out.1 += 1;
    if out.1 == 200 {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(out.0.clone()));
    } else if out.1 == 240 {
        exit.write(AppExit::Success);
    }
}
