    use super::*;

    #[test]
    fn parses_pack_toml_versions_and_index() {
        let text = r#"
name = "My Pack"
version = "1.0.0"
pack-format = "packwiz:1.1.0"

[versions]
minecraft = "1.20.1"
fabric = "0.15.7"

[index]
file = "index.toml"
hash-format = "sha256"
hash = "abc123"
"#;
        let pack = parse_pack_toml(text);
        assert_eq!(pack.name, "My Pack");
        assert_eq!(pack.version, "1.0.0");
        assert_eq!(pack.pack_format, "packwiz:1.1.0");
        assert_eq!(pack.versions.get("minecraft").map(String::as_str), Some("1.20.1"));
        assert_eq!(pack.versions.get("fabric").map(String::as_str), Some("0.15.7"));
        assert_eq!(pack.index.file, "index.toml");
        assert_eq!(pack.index.hash_format, "sha256");
        assert_eq!(pack.index.hash, "abc123");
    }

    #[test]
    fn parses_index_toml_array_of_tables() {
        let text = r#"
hash-format = "sha256"

[[files]]
file = "mods/sodium.pw.toml"
hash = "deadbeef"
metafile = true

[[files]]
file = "config/sodium.json"
hash = "cafef00d"
preserve = true
"#;
        let index = parse_index_toml(text);
        assert_eq!(index.hash_format, "sha256");
        assert_eq!(index.files.len(), 2);

        // 元文件指向 .pw.toml。
        assert_eq!(index.files[0].file, "mods/sodium.pw.toml");
        assert_eq!(index.files[0].hash, "deadbeef");
        assert!(index.files[0].metafile);
        assert!(!index.files[0].preserve);

        // metafile=false 的就地真实文件。
        assert_eq!(index.files[1].file, "config/sodium.json");
        assert!(!index.files[1].metafile);
        assert!(index.files[1].preserve);
    }

    #[test]
    fn parses_pw_toml_with_modrinth_update_and_ignores_prism_ext() {
        let text = r#"
name = "Sodium"
filename = "sodium-fabric-0.5.3.jar"
side = "client"
x-prismlauncher-loaders = ["fabric"]

[download]
url = "https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar"
hash-format = "sha512"
hash = "longhash"

[update.modrinth]
mod-id = "AANobbMI"
version = "abcd1234"
"#;
        let pw = parse_pw_toml(text);
        assert_eq!(pw.name, "Sodium");
        assert_eq!(pw.filename, "sodium-fabric-0.5.3.jar");
        assert_eq!(pw.side, "client");
        assert_eq!(pw.download.url, "https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar");
        assert_eq!(pw.download.hash_format, "sha512");
        assert_eq!(pw.download.hash, "longhash");

        let upd = pw.update.as_ref().unwrap();
        let mr = upd.modrinth.as_ref().unwrap();
        assert_eq!(mr.mod_id, "AANobbMI");
        assert_eq!(mr.version, "abcd1234");
        assert!(upd.curseforge.is_none());
    }

    #[test]
    fn parses_pw_toml_with_curseforge_update_int_ids() {
        let text = r#"
name = "JEI"
filename = "jei.jar"
side = "both"

[download]
mode = "metadata:curseforge"
hash-format = "murmur2"
hash = "1234567890"

[update.curseforge]
file-id = 4567890
project-id = 238222
"#;
        let pw = parse_pw_toml(text);
        assert_eq!(pw.side, "both");
        assert_eq!(pw.download.mode, "metadata:curseforge");
        let cf = pw.update.as_ref().unwrap().curseforge.as_ref().unwrap();
        assert_eq!(cf.file_id, 4567890);
        assert_eq!(cf.project_id, 238222);
    }

    #[test]
    fn inline_comment_and_quotes_are_handled() {
        let text = r#"
name = "Has # Hash"  # trailing comment
version = "2.0"
"#;
        let pack = parse_pack_toml(text);
        // '#' 在引号内保留,引号外的行内注释被去掉。
        assert_eq!(pack.name, "Has # Hash");
        assert_eq!(pack.version, "2.0");
    }

    #[test]
    fn pw_toml_without_update_yields_none() {
        let text = r#"
name = "URL Mod"
filename = "urlmod.jar"
side = "both"

[download]
url = "https://example.com/urlmod.jar"
hash-format = "sha256"
hash = "abc"
"#;
        let pw = parse_pw_toml(text);
        assert!(pw.update.is_none(), "无 update 子表时应为 None");
    }
