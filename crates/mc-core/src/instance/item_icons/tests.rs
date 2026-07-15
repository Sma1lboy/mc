use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

use super::resolve_item_icon;
use crate::instance::{base64_encode, Instance};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nicon";

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("mc-core-item-icon-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default();
        for (name, content) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(content).unwrap();
        }
        zip.finish().unwrap();
    }
    fs::write(path, bytes.into_inner()).unwrap();
}

#[test]
fn resolves_item_icon_from_mod_jar_model_texture_reference() {
    let temp = TempRoot::new("mod-jar");
    let inst = Instance::new("pack", temp.path.clone());
    fs::create_dir_all(inst.mods_dir()).unwrap();
    write_zip(
            &inst.mods_dir().join("create.jar"),
            &[
                (
                    "assets/create/models/item/andesite_casing.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"create:item/andesite_casing"}}"#,
                ),
                ("assets/create/textures/item/andesite_casing.png", PNG),
            ],
        );

    let icon = resolve_item_icon(&inst, "create:andesite_casing")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "create:andesite_casing");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("create.jar"));
}

#[test]
fn resolves_item_icon_from_kubejs_assets_before_mod_jar() {
    let temp = TempRoot::new("kubejs");
    let inst = Instance::new("pack", temp.path.clone());
    let texture = inst
        .game_dir()
        .join("kubejs/assets/create/textures/item/andesite_casing.png");
    fs::create_dir_all(texture.parent().unwrap()).unwrap();
    fs::write(&texture, PNG).unwrap();

    let icon = resolve_item_icon(&inst, "create:andesite_casing")
        .unwrap()
        .unwrap();

    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon
        .source
        .ends_with("kubejs/assets/create/textures/item/andesite_casing.png"));
}

#[test]
fn resolves_item_icon_from_unpacked_resource_pack_directory() {
    let temp = TempRoot::new("resourcepack-dir");
    let inst = Instance::new("pack", temp.path.clone());
    let texture = inst
        .resourcepacks_dir()
        .join("pack/assets/create/textures/item/andesite_casing.png");
    fs::create_dir_all(texture.parent().unwrap()).unwrap();
    fs::write(&texture, PNG).unwrap();

    let icon = resolve_item_icon(&inst, "create:andesite_casing")
        .unwrap()
        .unwrap();

    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon
        .source
        .ends_with("pack/assets/create/textures/item/andesite_casing.png"));
}

#[test]
fn resolves_vanilla_item_icon_from_inherited_version_jar() {
    let temp = TempRoot::new("version-jar");
    let inst = Instance::new("pack", temp.path.clone());
    let paths = inst.paths();
    fs::create_dir_all(paths.version_dir("pack")).unwrap();
    fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
    fs::write(
        paths.version_json("pack"),
        r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
    )
    .unwrap();
    fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
    write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "assets/minecraft/models/item/iron_nugget.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/iron_nugget"}}"#,
                ),
                ("assets/minecraft/textures/item/iron_nugget.png", PNG),
            ],
        );

    let icon = resolve_item_icon(&inst, "minecraft:iron_nugget")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "minecraft:iron_nugget");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("1.19.2.jar"));
}

#[test]
fn resolves_item_tag_icon_from_mod_jar_tag() {
    let temp = TempRoot::new("mod-jar-tag");
    let inst = Instance::new("pack", temp.path.clone());
    let paths = inst.paths();
    fs::create_dir_all(paths.version_dir("pack")).unwrap();
    fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
    fs::create_dir_all(inst.mods_dir()).unwrap();
    fs::write(
        paths.version_json("pack"),
        r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
    )
    .unwrap();
    fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
    write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "assets/minecraft/models/item/iron_nugget.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/iron_nugget"}}"#,
                ),
                ("assets/minecraft/textures/item/iron_nugget.png", PNG),
            ],
        );
    write_zip(
        &inst.mods_dir().join("forge-tags.jar"),
        &[(
            "data/forge/tags/items/nuggets/iron.json",
            br#"{"values":["minecraft:iron_nugget"]}"#,
        )],
    );

    let icon = resolve_item_icon(&inst, "#forge:nuggets/iron")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "#forge:nuggets/iron");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("1.19.2.jar"));
}

#[test]
fn resolves_nested_item_tag_icon_from_mod_jar_tags() {
    let temp = TempRoot::new("mod-jar-nested-tag");
    let inst = Instance::new("pack", temp.path.clone());
    let paths = inst.paths();
    fs::create_dir_all(paths.version_dir("pack")).unwrap();
    fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
    fs::create_dir_all(inst.mods_dir()).unwrap();
    fs::write(
        paths.version_json("pack"),
        r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
    )
    .unwrap();
    fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
    write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "assets/minecraft/models/item/quartz.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/quartz"}}"#,
                ),
                ("assets/minecraft/textures/item/quartz.png", PNG),
            ],
        );
    write_zip(
        &inst.mods_dir().join("forge-tags.jar"),
        &[
            (
                "data/forge/tags/items/gems/quartz.json",
                br##"{"values":["#forge:gems/nether_quartz"]}"##,
            ),
            (
                "data/forge/tags/items/gems/nether_quartz.json",
                br#"{"values":[{"id":"minecraft:quartz","required":false}]}"#,
            ),
        ],
    );

    let icon = resolve_item_icon(&inst, "#forge:gems/quartz")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "#forge:gems/quartz");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("1.19.2.jar"));
}

#[test]
fn prefers_stable_representative_for_broad_vanilla_item_tag() {
    let temp = TempRoot::new("vanilla-tag-representative");
    let inst = Instance::new("pack", temp.path.clone());
    let paths = inst.paths();
    fs::create_dir_all(paths.version_dir("pack")).unwrap();
    fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
    fs::create_dir_all(inst.mods_dir()).unwrap();
    fs::write(
        paths.version_json("pack"),
        r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
    )
    .unwrap();
    fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
    write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "data/minecraft/tags/items/planks.json",
                    br#"{"values":["minecraft:oak_planks"]}"#,
                ),
                (
                    "assets/minecraft/models/item/oak_planks.json",
                    br#"{"parent":"minecraft:block/oak_planks"}"#,
                ),
                (
                    "assets/minecraft/models/block/oak_planks.json",
                    br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/oak_planks"}}"#,
                ),
                ("assets/minecraft/textures/block/oak_planks.png", PNG),
            ],
        );
    write_zip(
            &inst.mods_dir().join("custom-planks.jar"),
            &[
                (
                    "data/minecraft/tags/items/planks.json",
                    br#"{"values":["example:powdery_planks"]}"#,
                ),
                (
                    "assets/example/models/item/powdery_planks.json",
                    br#"{"parent":"example:block/powdery_planks"}"#,
                ),
                (
                    "assets/example/models/block/powdery_planks.json",
                    br#"{"parent":"minecraft:block/cube_all","textures":{"all":"example:block/powdery_planks"}}"#,
                ),
                ("assets/example/textures/block/powdery_planks.png", b"fake"),
            ],
        );

    let icon = resolve_item_icon(&inst, "#minecraft:planks")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "#minecraft:planks");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("1.19.2.jar"));
}

#[test]
fn resolves_common_forge_item_tag_without_tag_json() {
    let temp = TempRoot::new("forge-tag-fallback");
    let inst = Instance::new("pack", temp.path.clone());
    let paths = inst.paths();
    fs::create_dir_all(paths.version_dir("pack")).unwrap();
    fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
    fs::write(
        paths.version_json("pack"),
        r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
    )
    .unwrap();
    fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
    write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "assets/minecraft/models/item/redstone.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/redstone"}}"#,
                ),
                ("assets/minecraft/textures/item/redstone.png", PNG),
            ],
        );

    let icon = resolve_item_icon(&inst, "#forge:dusts/redstone")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "#forge:dusts/redstone");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("1.19.2.jar"));
}

#[test]
fn resolves_common_forge_material_tag_across_mod_namespaces() {
    let temp = TempRoot::new("forge-tag-mod-namespace-fallback");
    let inst = Instance::new("pack", temp.path.clone());
    fs::create_dir_all(inst.mods_dir()).unwrap();
    write_zip(
            &inst.mods_dir().join("create.jar"),
            &[
                (
                    "assets/create/models/item/zinc_ingot.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"create:item/zinc_ingot"}}"#,
                ),
                ("assets/create/textures/item/zinc_ingot.png", PNG),
            ],
        );

    let icon = resolve_item_icon(&inst, "#forge:ingots/zinc")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "#forge:ingots/zinc");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("create.jar"));
}

#[test]
fn resolves_block_item_icon_through_parent_model() {
    let temp = TempRoot::new("block-item-parent");
    let inst = Instance::new("pack", temp.path.clone());
    let paths = inst.paths();
    fs::create_dir_all(paths.version_dir("pack")).unwrap();
    fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
    fs::write(
        paths.version_json("pack"),
        r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
    )
    .unwrap();
    fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
    write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "assets/minecraft/models/item/andesite.json",
                    br#"{"parent":"minecraft:block/andesite"}"#,
                ),
                (
                    "assets/minecraft/models/block/andesite.json",
                    br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/andesite"}}"#,
                ),
                ("assets/minecraft/textures/block/andesite.png", PNG),
            ],
        );

    let icon = resolve_item_icon(&inst, "minecraft:andesite")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "minecraft:andesite");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("1.19.2.jar"));
}

#[test]
fn resolves_block_item_icon_from_particle_texture() {
    let temp = TempRoot::new("block-item-particle");
    let inst = Instance::new("pack", temp.path.clone());
    let paths = inst.paths();
    fs::create_dir_all(paths.version_dir("pack")).unwrap();
    fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
    fs::write(
        paths.version_json("pack"),
        r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
    )
    .unwrap();
    fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
    write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "assets/minecraft/models/item/crafting_table.json",
                    br#"{"parent":"minecraft:block/crafting_table"}"#,
                ),
                (
                    "assets/minecraft/models/block/crafting_table.json",
                    br#"{"parent":"minecraft:block/cube","textures":{"particle":"minecraft:block/crafting_table_front","north":"minecraft:block/crafting_table_front"}}"#,
                ),
                (
                    "assets/minecraft/textures/block/crafting_table_front.png",
                    PNG,
                ),
            ],
        );

    let icon = resolve_item_icon(&inst, "minecraft:crafting_table")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "minecraft:crafting_table");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("1.19.2.jar"));
}

#[test]
fn resolves_mod_block_item_icon_from_particle_texture() {
    let temp = TempRoot::new("mod-block-item-particle");
    let inst = Instance::new("pack", temp.path.clone());
    fs::create_dir_all(inst.mods_dir()).unwrap();
    write_zip(
            &inst.mods_dir().join("create.jar"),
            &[
                (
                    "assets/create/models/item/mechanical_crafter.json",
                    br#"{"parent":"create:block/mechanical_crafter/item"}"#,
                ),
                (
                    "assets/create/models/block/mechanical_crafter/item.json",
                    br#"{"parent":"block/block","textures":{"particle":"create:block/brass_casing","4":"create:block/crafter_side"}}"#,
                ),
                ("assets/create/textures/block/brass_casing.png", PNG),
            ],
        );

    let icon = resolve_item_icon(&inst, "create:mechanical_crafter")
        .unwrap()
        .unwrap();

    assert_eq!(icon.item_id, "create:mechanical_crafter");
    assert_eq!(
        icon.data_url,
        format!("data:image/png;base64,{}", base64_encode(PNG))
    );
    assert!(icon.source.ends_with("create.jar"));
}
