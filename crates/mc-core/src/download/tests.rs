    use super::*;
use super::downloader::*;

    fn one_response_server(status: u16, body: &'static [u8]) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 1024];
                let _ = stream.read(&mut buf);
                let reason = match status {
                    200 => "OK",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "OK",
                };
                let headers = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        format!("http://{addr}")
    }

    fn one_response_server_without_length(body: &'static [u8]) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 1024];
                let _ = stream.read(&mut buf);
                let headers = "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn part_path_is_a_unique_sibling() {
        let p = PathBuf::from("/tmp/foo/client.jar");
        let a = crate::fs::unique_temp_sibling(&p, "part");
        let b = crate::fs::unique_temp_sibling(&p, "part");
        // Same directory (so the follow-up rename stays atomic on one filesystem).
        assert_eq!(a.parent(), p.parent());
        // Keeps the original filename and tags it (recognisable on disk).
        assert!(a.file_name().unwrap().to_str().unwrap().starts_with("client.jar.part-"));
        // Two writers racing on the SAME destination never collide.
        assert_ne!(a, b);
    }

    #[test]
    fn new_uses_mirror_none_by_default() {
        let d = Downloader::new(4).unwrap();
        // 默认无镜像:URL 原样返回。
        assert_eq!(
            d.mirror.rewrite("https://libraries.minecraft.net/a/b.jar"),
            "https://libraries.minecraft.net/a/b.jar"
        );
    }

    #[test]
    fn with_mirror_applies_rewrite() {
        let d = Downloader::new(4)
            .unwrap()
            .with_mirror(MirrorResolver::bmclapi());
        assert_eq!(
            d.mirror.rewrite("https://libraries.minecraft.net/a/b.jar"),
            "https://bmclapi2.bangbang93.com/maven/a/b.jar"
        );
    }

    #[test]
    fn zero_concurrency_is_bumped_to_one() {
        // Semaphore(0) 会永久阻塞;new 必须把 0 提升到 1。
        let d = Downloader::new(0).unwrap();
        assert!(d.sem.available_permits() >= 1);
    }

    #[test]
    fn curseforge_host_matching() {
        assert!(is_curseforge_host("api.curseforge.com"));
        assert!(is_curseforge_host("edge.forgecdn.net"));
        assert!(is_curseforge_host("mediafilez.forgecdn.net"));
        assert!(is_curseforge_host("FORGECDN.NET")); // 大小写无关
        assert!(is_curseforge_host("www.curseforge.com"));
        // 非 CF host 不命中。
        assert!(!is_curseforge_host("cdn.modrinth.com"));
        assert!(!is_curseforge_host("libraries.minecraft.net"));
        // 防 host 后缀伪造:`evilforgecdn.net` 不应命中(无点分隔)。
        assert!(!is_curseforge_host("evilforgecdn.net"));
        assert!(!is_curseforge_host("forgecdn.net.evil.com"));
    }

    #[test]
    fn cf_auth_header_only_for_cf_hosts() {
        // 决策矩阵:仅"持有 key 且 URL 是 CF host"时附加 x-api-key。
        let with_key = Downloader::new(2).unwrap().with_cf_api_key(Some("secret".into()));
        // 持有 key + CF host → 应注入(用 RequestBuilder 的可克隆性间接验证:
        // 我们无法直接读头,但能验证 url_is_curseforge 的决策面)。
        assert!(url_is_curseforge("https://edge.forgecdn.net/files/1/sodium.jar"));
        assert!(url_is_curseforge("https://api.curseforge.com/v1/mods/files"));
        assert!(!url_is_curseforge("https://cdn.modrinth.com/data/x/y.jar"));
        assert!(!url_is_curseforge("https://libraries.minecraft.net/a/b.jar"));
        // key 存在与否不影响 host 决策;空 key 被 with_cf_api_key 归一为 None。
        assert!(with_key.cf_api_key.is_some());
        let blank = Downloader::new(2).unwrap().with_cf_api_key(Some("   ".into()));
        assert!(blank.cf_api_key.is_none());
        let none = Downloader::new(2).unwrap().with_cf_api_key(None);
        assert!(none.cf_api_key.is_none());
    }

    #[test]
    fn apply_cf_auth_injects_header_only_when_keyed_and_cf_host() {
        // 用 try_clone + build 把请求物化成 reqwest::Request,直接检查头。
        let keyed = Downloader::new(2).unwrap().with_cf_api_key(Some("k123".into()));

        let cf_url = "https://edge.forgecdn.net/files/1/sodium.jar";
        let req = keyed
            .apply_cf_auth(keyed.client.get(cf_url), cf_url)
            .build()
            .unwrap();
        assert_eq!(req.headers().get("x-api-key").map(|v| v.to_str().unwrap()), Some("k123"));

        // 非 CF host:不带头。
        let mr_url = "https://cdn.modrinth.com/data/x/y.jar";
        let req2 = keyed
            .apply_cf_auth(keyed.client.get(mr_url), mr_url)
            .build()
            .unwrap();
        assert!(req2.headers().get("x-api-key").is_none());

        // 无 key 的 downloader:即便 CF host 也不带头。
        let unkeyed = Downloader::new(2).unwrap();
        let req3 = unkeyed
            .apply_cf_auth(unkeyed.client.get(cf_url), cf_url)
            .build()
            .unwrap();
        assert!(req3.headers().get("x-api-key").is_none());
    }

    #[test]
    fn concurrency_is_stable_under_outstanding_permits() {
        // download_batch 必须读构造时的并发度,而非当前可用许可:持有一个许可后
        // available_permits 会少 1,但 configured_concurrency 应仍报告原值。
        let d = Downloader::new(4).unwrap();
        assert_eq!(d.configured_concurrency(), 4);
        let permit = d.sem.clone().try_acquire_owned().expect("permit available");
        assert_eq!(d.sem.available_permits(), 3);
        assert_eq!(d.configured_concurrency(), 4);
        drop(permit);
    }

    #[tokio::test]
    async fn get_bytes_falls_back_from_mirror_to_official_candidate() {
        let mirror = one_response_server(404, b"missing");
        let official = one_response_server(200, b"official");
        let downloader = Downloader::new(1)
            .unwrap()
            .with_mirror(MirrorResolver::from_rules(vec![(official.clone(), mirror)]));

        let bytes = downloader
            .get_bytes(&format!("{official}/manifest.json"))
            .await
            .unwrap();

        assert_eq!(bytes, b"official");
    }

    #[tokio::test]
    async fn get_bytes_capped_errors_when_stream_exceeds_cap() {
        let server = one_response_server_without_length(b"larger than cap");
        let downloader = Downloader::new(1).unwrap();

        let err = downloader
            .get_bytes_capped(&format!("{server}/large.bin"), 6)
            .await
            .expect_err("body larger than cap should fail");

        assert!(
            err.to_string().contains("exceeds maximum size"),
            "unexpected error: {err}"
        );
    }
