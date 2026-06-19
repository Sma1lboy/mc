//! Launcher news/announcements. In-memory for v1 (no DB); swap for a store later.

use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct NewsItem {
    pub id: String,
    pub title: String,
    pub body: String,
    /// ISO-8601 date.
    pub date: String,
    pub url: Option<String>,
}

/// The current news feed. Static for now; later sourced from a CMS/file.
pub fn feed() -> Vec<NewsItem> {
    vec![
        NewsItem {
            id: "welcome".into(),
            title: "欢迎使用 mc-launcher".into(),
            body: "支持原版 / Fabric / Quilt,以及 Modrinth 模组与整合包。".into(),
            date: "2026-06-18".into(),
            url: None,
        },
        NewsItem {
            id: "lite-server".into(),
            title: "Lite 服务器上线".into(),
            body: "加载器版本聚合、新闻、实例分享现在由我们自己的轻量后端提供。".into(),
            date: "2026-06-18".into(),
            url: None,
        },
    ]
}
